//! El canal con el servidor de senalizacion.
//
// Hace el saludo, contesta al reto firmando con la clave del navegador y deja
// la conexion lista para pedir sesion. Todo lo que llega del servidor se trata
// como dato: el servidor enruta mensajes, no decide nada.

import { bytesAHex } from "./bytes.js";
import type { Identidad } from "./identidad.js";
import { transcripcionDeAutenticacion } from "./transcripciones.js";

/** Un mensaje cualquiera del servidor. Se discrimina por `type`. */
export interface MensajeDelServidor {
  readonly type: string;
  readonly [campo: string]: unknown;
}

/**
 * Version del protocolo que va en cada sobre.
 *
 * Todo lo que viaja por este canal va envuelto —`{v, ts, payload}`—, igual que
 * lo que viaja por el canal de control. El servidor rechaza sin explicacion lo
 * que llegue sin sobre.
 */
const VERSION_PROTOCOLO = 1;

/** Datos de conexion que entrega el servidor al aceptar la autenticacion. */
export interface Aceptado {
  readonly iceServers: RTCIceServer[];
}

/** Desvio de reloj que el servidor tolera, en milisegundos. */
const TOLERANCIA_DE_RELOJ_MS = 30_000;

/** Capacidades que este cliente declara. Solo lo que de verdad hace. */
const CAPACIDADES = ["screen_stream", "multi_monitor", "remote_input", "codec_h264"];

/** Permisos que se piden al host. Lo que el host conceda puede ser menos. */
export const PERMISOS_PEDIDOS = [
  "view_screen",
  "control_pointer",
  "control_keyboard",
  "system_info",
];

/** Conexion con el servidor de senalizacion. */
export class Senalizacion {
  private readonly socket: WebSocket;
  private readonly cola: MensajeDelServidor[] = [];
  /**
   * Quien esta esperando el siguiente mensaje.
   *
   * Se guarda tambien como se le dice que no va a llegar: si el socket se
   * cierra mientras alguien espera y solo se resolviera al llegar un mensaje,
   * la espera no terminaria nunca y la pagina se quedaria en blanco sin decir
   * por que.
   */
  private esperando: {
    resolver: (mensaje: MensajeDelServidor) => void;
    rechazar: (error: Error) => void;
  } | null = null;
  private cerrado: Error | null = null;

  private constructor(socket: WebSocket) {
    this.socket = socket;
    socket.onmessage = (evento) => {
      let mensaje: MensajeDelServidor;
      try {
        const sobre = JSON.parse(String(evento.data)) as { payload?: MensajeDelServidor };
        if (!sobre.payload?.type) {
          return;
        }
        mensaje = sobre.payload;
      } catch {
        return;
      }
      if (this.esperando) {
        const { resolver } = this.esperando;
        this.esperando = null;
        resolver(mensaje);
      } else {
        this.cola.push(mensaje);
      }
    };
    const terminar = (motivo: string) => {
      this.cerrado = new Error(motivo);
      if (this.esperando) {
        const { rechazar } = this.esperando;
        this.esperando = null;
        rechazar(this.cerrado);
      }
    };
    socket.onclose = (evento) => {
      const detalle = evento.reason ? `: ${evento.reason}` : "";
      terminar(`el servidor cerro la conexion (codigo ${evento.code}${detalle})`);
    };
  }

  /** Abre el socket. */
  static abrir(url: string): Promise<Senalizacion> {
    return new Promise((resolver, rechazar) => {
      const socket = new WebSocket(url);
      socket.onopen = () => resolver(new Senalizacion(socket));
      socket.onerror = () => rechazar(new Error(`no se pudo conectar a ${url}`));
    });
  }

  /**
   * Envia un mensaje al servidor, con su sobre.
   *
   * `marca` existe porque la prueba de autenticacion se firma sobre la marca
   * del sobre: ahi hay que enviar exactamente la que se firmo, no una nueva.
   */
  enviar(mensaje: Record<string, unknown>, marca = Date.now()): void {
    this.socket.send(JSON.stringify({ v: VERSION_PROTOCOLO, ts: marca, payload: mensaje }));
  }

  /** Espera el siguiente mensaje del servidor. */
  siguiente(): Promise<MensajeDelServidor> {
    const pendiente = this.cola.shift();
    if (pendiente) {
      return Promise.resolve(pendiente);
    }
    if (this.cerrado) {
      return Promise.reject(this.cerrado);
    }
    return new Promise((resolver, rechazar) => {
      this.esperando = { resolver, rechazar };
    });
  }

  /** Espera hasta que llegue un mensaje de un tipo concreto. */
  async esperarTipo(tipo: string): Promise<MensajeDelServidor> {
    for (;;) {
      const mensaje = await this.siguiente();
      if (mensaje.type === tipo) {
        return mensaje;
      }
      // El servidor explica por que antes de cerrar. Ignorarlo dejaria la
      // pagina esperando para siempre un mensaje que ya no va a llegar.
      if (mensaje.type === "error") {
        throw new Error(`el servidor rechazo el mensaje: ${String(mensaje["code"])} (${String(mensaje["message"])})`);
      }
      if (mensaje.type === "auth_rejected") {
        const reintento = mensaje["retry_after_secs"];
        const espera = typeof reintento === "number" ? `; reintente en ${reintento} s` : "";
        throw new Error(`autenticacion rechazada: ${String(mensaje["reason"])}${espera}`);
      }
      if (mensaje.type === "session_rejected") {
        throw new Error(`el PC rechazo la sesion: ${String(mensaje["reason"])}`);
      }
    }
  }

  /** Cierra la conexion. */
  cerrar(): void {
    this.socket.close();
  }

  /**
   * Saluda, contesta al reto y queda autenticado.
   *
   * El reto lo fija el servidor pero el nonce del cliente lo aporta este
   * navegador: que los dos aporten nonce es lo que impide que el servidor
   * elija por completo lo que se firma.
   */
  async autenticar(identidad: Identidad, nombre: string): Promise<Aceptado> {
    this.enviar({
      type: "hello",
      device: {
        id: identidad.dispositivoId,
        display_name: nombre,
        platform: plataforma(),
        role: "controller",
        os_version: navigator.platform || "navegador",
        app_version: "nexus-web/0.1.0",
      },
      capabilities: CAPACIDADES,
      public_key_hex: identidad.clavePublicaHex,
    });

    const reto = await this.esperarTipo("challenge");
    const nonceCliente = new Uint8Array(32);
    crypto.getRandomValues(nonceCliente);

    // El reloj: si este dispositivo esta desviado, el servidor rechazara la
    // firma. Mas vale decirlo aqui que dejar un "credenciales invalidas" que no
    // explica nada.
    const marca = Date.now();
    const desvio = Math.abs(Number(reto["server_time"]) - marca);
    if (desvio > TOLERANCIA_DE_RELOJ_MS) {
      throw new Error(
        `el reloj de este dispositivo esta ${Math.round(desvio / 1000)} s desviado del ` +
          "servidor; pongalo en hora o el servidor rechazara la firma",
      );
    }

    // Se firma **la marca del sobre**, no la del servidor: el servidor verifica
    // la firma contra la marca con la que llega el mensaje. Por eso la misma
    // `marca` va a la transcripcion y al sobre.
    const firma = await identidad.firmar(
      transcripcionDeAutenticacion({
        nonceServidorHex: String(reto["nonce_hex"]),
        nonceClienteHex: bytesAHex(nonceCliente),
        dispositivoId: identidad.dispositivoId,
        marca,
      }),
    );

    this.enviar(
      {
        type: "auth_proof",
        device_id: identidad.dispositivoId,
        client_nonce_hex: bytesAHex(nonceCliente),
        signature_hex: bytesAHex(firma),
      },
      marca,
    );

    const aceptado = await this.esperarTipo("auth_accepted");
    const servidores = (aceptado["ice_servers"] ?? []) as RTCIceServer[];
    return { iceServers: servidores };
  }
}

/** Lo que este navegador puede decir honestamente de su plataforma. */
function plataforma(): string {
  const ua = navigator.userAgent;
  if (/iPad/.test(ua)) {
    return "ipad_os";
  }
  if (/iPhone|iPod/.test(ua)) {
    return "ios";
  }
  // Un navegador de escritorio no es ninguno de los que el protocolo nombra, y
  // decir que es Windows o Linux seria inventarselo: la version de la
  // aplicacion ya dice que esto es el cliente web.
  return "unknown";
}
