//! Una sesion con el PC: negociar, atar y recibir la pantalla.
//
// ## El orden importa y es deliberado
//
// 1. Se pide sesion y el host decide (contra su registro local, no contra el
//    servidor).
// 2. Llega la oferta, se contesta y con eso quedan las dos huellas DTLS.
// 3. **Solo entonces** se firma la atadura de sesion: es lo que demuestra que
//    quien habla por este canal cifrado es el dueno de la clave.
// 4. Cuando el canal de control esta abierto, se pide la pantalla.
//
// ## Lo que este cliente todavia no hace
//
// El host envia tambien su propia prueba de atadura. Este cliente la recibe y
// **no la comprueba**, porque para comprobarla haria falta tener guardada la
// clave publica del PC, que es lo que deja el emparejamiento. Hasta que eso
// este, quien verifica es el host; el navegador confia en el servidor para
// saber con que PC habla. Esta escrito aqui para que no se olvide.

import type { Senalizacion } from "./senalizacion.js";
import { PERMISOS_PEDIDOS } from "./senalizacion.js";
import type { Identidad } from "./identidad.js";
import { bytesAHex } from "./bytes.js";
import { huellaCanonica, huellaDeSdp, transcripcionDeAtadura } from "./transcripciones.js";

/** Canal por el que viajan las ordenes. */
export const CANAL_CONTROL = "control";
/** Canal por el que viaja el raton y el teclado. */
export const CANAL_ENTRADA = "input";

/** Lo que una sesion va contando de si misma. */
export interface Observador {
  /** Un paso que se completo, en lenguaje llano. */
  paso(texto: string): void;
  /** Llego la pista de video. */
  pantalla(flujo: MediaStream): void;
  /** El host conto algo por el canal de control. */
  evento?(evento: Record<string, unknown>): void;
}

/** Una sesion establecida con el PC. */
export class Sesion {
  private readonly pc: RTCPeerConnection;
  private readonly sig: Senalizacion;
  private readonly observador: Observador;
  private control: RTCDataChannel | null = null;
  private entrada: RTCDataChannel | null = null;
  private siguienteId = 1;
  /** Candidatos propios que se han enviado, para poder diagnosticar. */
  readonly enviados: string[] = [];
  /** Candidatos del host que se han recibido. */
  readonly recibidos: string[] = [];
  /** A quien avisar cuando aparezca el primer camino propio. */
  private avisarDelPrimerCamino: (() => void) | null = null;
  /** Caminos propios que esperan a que la respuesta este enviada. */
  private readonly encolados: RTCIceCandidate[] = [];
  /** Si la respuesta ya salio. */
  private respondido = false;

  private constructor(pc: RTCPeerConnection, sig: Senalizacion, observador: Observador) {
    this.pc = pc;
    this.sig = sig;
    this.observador = observador;
  }

  /**
   * Abre una sesion contra un host y la lleva hasta ver la pantalla.
   *
   * Vuelve en cuanto la pista de video esta entregada; lo demas sigue
   * ocurriendo en segundo plano hasta que se cierre.
   */
  static async abrir(
    sig: Senalizacion,
    identidad: Identidad,
    hostId: string,
    iceServers: RTCIceServer[],
    observador: Observador,
  ): Promise<Sesion> {
    const pc = new RTCPeerConnection({ iceServers });
    const sesion = new Sesion(pc, sig, observador);

    sig.enviar({
      type: "request_session",
      host_id: hostId,
      requested_permissions: PERMISOS_PEDIDOS,
    });
    const aceptada = await sig.esperarTipo("session_accepted");
    const sesionId = String(aceptada["session_id"]);
    observador.paso(`el PC acepto la sesion ${sesionId.slice(0, 8)}`);

    pc.ontrack = (evento) => {
      const flujo = evento.streams[0] ?? new MediaStream([evento.track]);
      observador.paso("llego la pista de video");
      observador.pantalla(flujo);
    };
    pc.ondatachannel = (evento) => {
      observador.paso(`el PC abrio el canal "${evento.channel.label}"`);
      sesion.registrarCanal(evento.channel);
    };
    pc.onicecandidate = (evento) => {
      if (evento.candidate) {
        sesion.enviados.push(evento.candidate.candidate);
        sesion.soltarEspera();
        // Ni uno antes de la respuesta: hasta que el otro extremo la procesa no
        // tiene con que relacionar un camino, y recibirlo antes es un error
        // para el. Se guardan y salen en cuanto la respuesta esta enviada.
        sesion.encolar(sesionId, evento.candidate);
      } else {
        sesion.enviados.push("(fin de candidatos)");
        // Tambien al terminar: si no hubo ni un camino, contestar sin ellos es
        // mejor que no contestar.
        sesion.soltarEspera();
      }
    };

    void sesion.atender(sesionId, identidad, hostId);
    return sesion;
  }

  /** Guarda un camino propio, o lo envia si ya se puede. */
  private encolar(sesionId: string, candidato: RTCIceCandidate): void {
    if (!this.respondido) {
      this.encolados.push(candidato);
      return;
    }
    this.sig.enviar({
      type: "ice_candidate",
      session_id: sesionId,
      candidate: candidato.candidate,
      sdp_mid: candidato.sdpMid,
    });
  }

  /** Suelta los caminos guardados: la respuesta ya esta enviada. */
  private soltarCaminos(sesionId: string): void {
    this.respondido = true;
    for (const candidato of this.encolados.splice(0)) {
      this.encolar(sesionId, candidato);
    }
  }

  /** Suelta a quien estuviera esperando el primer camino. */
  private soltarEspera(): void {
    const avisar = this.avisarDelPrimerCamino;
    this.avisarDelPrimerCamino = null;
    avisar?.();
  }

  /** Registra un canal de datos que abrio el host. */
  private registrarCanal(canal: RTCDataChannel): void {
    if (canal.label === CANAL_CONTROL) {
      this.control = canal;
      canal.onmessage = (evento) => this.recibirControl(String(evento.data));
      canal.onopen = () => {
        this.observador.paso("canal de control abierto; se pide la pantalla");
        // Aqui y no antes: pedirla es tambien decir "ya estoy listo para verla".
        this.pedir({ op: "start_screen" });
      };
    } else if (canal.label === CANAL_ENTRADA) {
      this.entrada = canal;
    }
  }

  /** Atiende la senalizacion de la sesion hasta que se cierre. */
  private async atender(sesionId: string, identidad: Identidad, hostId: string): Promise<void> {
    for (;;) {
      let mensaje;
      try {
        mensaje = await this.sig.siguiente();
      } catch {
        return;
      }

      switch (mensaje.type) {
        case "offer": {
          await this.pc.setRemoteDescription({ type: "offer", sdp: String(mensaje["sdp"]) });
          const respuesta = await this.pc.createAnswer();
          await this.pc.setLocalDescription(respuesta);

          // Se espera a reunir los caminos antes de contestar, de modo que la
          // respuesta los lleve dentro. El otro extremo los espera ahi: los que
          // se envian despues, uno a uno, no le llegan al agente ICE, y sin
          // camino no hay canal cifrado ni imagen. Comprobado midiendolo.
          await this.esperarPrimerCamino();

          this.sig.enviar({
            type: "answer",
            session_id: sesionId,
            sdp: this.pc.localDescription?.sdp ?? respuesta.sdp ?? "",
          });
          this.soltarCaminos(sesionId);
          this.observador.paso(
            `conexion cifrada negociada con ${this.enviados.length} camino(s)`,
          );
          await this.atar(sesionId, identidad, hostId);
          break;
        }
        case "ice_candidate": {
          const candidato = String(mensaje["candidate"]);
          this.recibidos.push(candidato);
          const sdpMid = mensaje["sdp_mid"];
          await this.pc.addIceCandidate({
            candidate: candidato,
            ...(typeof sdpMid === "string" ? { sdpMid } : {}),
          });
          break;
        }
        case "peer_gone":
          this.observador.paso("el PC se desconecto");
          return;
        // El servidor explica por que rechaza algo. Ignorarlo deja fallos
        // invisibles: la sesion parece seguir y en realidad le falta una pieza.
        case "error":
          this.observador.paso(
            `el servidor rechazo un mensaje de la sesion: ${String(mensaje["code"])} ` +
              `(${String(mensaje["message"])})`,
          );
          break;
        default:
          break;
      }
    }
  }

  /**
   * Espera a tener al menos un camino antes de contestar.
   *
   * No se espera a reunirlos todos: con un servidor TURN que no contesta, la
   * reunion puede no terminar nunca. Con uno basta, porque lo que importa es
   * que la respuesta no salga vacia de caminos: el otro extremo los lee de
   * ahi.
   */
  private esperarPrimerCamino(): Promise<void> {
    if (this.enviados.length > 0) {
      return Promise.resolve();
    }
    return new Promise((resolver) => {
      this.avisarDelPrimerCamino = resolver;
    });
  }

  /** Firma la atadura de sesion y la envia. */
  private async atar(sesionId: string, identidad: Identidad, hostId: string): Promise<void> {
    const miSdp = this.pc.localDescription?.sdp;
    const suSdp = this.pc.remoteDescription?.sdp;
    if (!miSdp || !suSdp) {
      throw new Error("no hay descripciones con las que atar la sesion");
    }

    const miHuella = huellaDeSdp(miSdp);
    const huellaDelHost = huellaDeSdp(suSdp);
    const firma = await identidad.firmar(
      transcripcionDeAtadura({
        sesionId,
        miId: identidad.dispositivoId,
        hostId,
        miHuella,
        huellaDelHost,
      }),
    );

    this.sig.enviar({
      type: "peer_auth",
      session_id: sesionId,
      fingerprint: huellaCanonica(miHuella),
      signature_hex: bytesAHex(firma),
    });
    this.observador.paso("atadura de sesion firmada y enviada");
  }

  /** Atiende lo que llega por el canal de control. */
  private recibirControl(texto: string): void {
    let sobre: { payload?: Record<string, unknown> };
    try {
      sobre = JSON.parse(texto) as { payload?: Record<string, unknown> };
    } catch {
      return;
    }
    const carga = sobre.payload;
    if (carga && carga["channel"] === "event") {
      this.observador.evento?.(carga);
    }
  }

  /** Envia una peticion por el canal de control. */
  pedir(peticion: Record<string, unknown>): void {
    if (!this.control || this.control.readyState !== "open") {
      return;
    }
    const id = this.siguienteId;
    this.siguienteId += 1;
    this.control.send(
      JSON.stringify({
        v: 1,
        ts: Date.now(),
        payload: { channel: "request", id, request: peticion },
      }),
    );
  }

  /** Envia un evento de raton o teclado por su canal. */
  enviarEntrada(evento: Record<string, unknown>): void {
    if (!this.entrada || this.entrada.readyState !== "open") {
      return;
    }
    const id = this.siguienteId;
    this.siguienteId += 1;
    this.entrada.send(
      JSON.stringify({
        v: 1,
        ts: Date.now(),
        payload: { channel: "request", id, request: { op: "input", event: evento } },
      }),
    );
  }

  /** Estadisticas de la pista de video, para saber si de verdad se decodifica. */
  async fotogramasDecodificados(): Promise<number> {
    const informe = await this.pc.getStats();
    let total = 0;
    informe.forEach((entrada) => {
      const dato = entrada as { type?: string; kind?: string; framesDecoded?: number };
      if (dato.type === "inbound-rtp" && dato.kind === "video") {
        total += dato.framesDecoded ?? 0;
      }
    });
    return total;
  }

  /**
   * Lo que el navegador sabe de la pista, en crudo.
   *
   * Sirve para distinguir dos fallos que se parecen: que no lleguen bytes (es
   * la red) y que lleguen y no se decodifiquen (es el codec). Sin esto, las dos
   * cosas se ven igual: una pantalla en negro.
   */
  async diagnostico(): Promise<Record<string, unknown>> {
    const informe = await this.pc.getStats();
    const interesan = [
      "inbound-rtp",
      "candidate-pair",
      "transport",
      "local-candidate",
      "remote-candidate",
    ];
    const pistas: Record<string, unknown>[] = [];
    informe.forEach((entrada) => {
      const dato = entrada as unknown as Record<string, unknown>;
      if (interesan.includes(String(dato["type"]))) {
        // Se copia campo a campo: los objetos de estadisticas no sobreviven a
        // una serializacion directa.
        pistas.push({ ...dato });
      }
    });
    return {
      conexion: this.pc.connectionState,
      ice: this.pc.iceConnectionState,
      senalizacion: this.pc.signalingState,
      sctp: this.pc.sctp?.state ?? "sin sctp",
      canalControl: this.control?.readyState ?? "sin canal",
      canalEntrada: this.entrada?.readyState ?? "sin canal",
      candidatosEnviados: this.enviados,
      candidatosRecibidos: this.recibidos,
      pistas,
    };
  }

  /** Cierra la sesion. */
  cerrar(): void {
    this.pc.close();
  }
}
