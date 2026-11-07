//! El cliente web: una pagina que ve y controla el PC.
//
// Lo que hace, en orden: carga la identidad de este navegador, se autentica
// contra el servidor, pide sesion al PC, firma la atadura, pide la pantalla y
// la muestra. Los pasos se van escribiendo en la pagina para que, cuando algo
// falle, se vea **donde** fallo.

import { cargarIdentidad, type Identidad } from "./identidad.js";
import { Senalizacion } from "./senalizacion.js";
import { Sesion } from "./sesion.js";

/** Lo que la pagina expone para poder comprobarla desde fuera. */
interface Expuesto {
  identidad: { dispositivoId: string; clavePublicaHex: string } | null;
  pasos: string[];
  error: string | null;
  pantallaViva: boolean;
  fotogramas: () => Promise<number>;
  diagnostico: () => Promise<Record<string, unknown>>;
  conectar: (servidor: string, hostId: string) => Promise<void>;
}

const elemento = <T extends HTMLElement>(id: string): T => {
  const encontrado = document.getElementById(id);
  if (!encontrado) {
    throw new Error(`falta el elemento ${id}`);
  }
  return encontrado as T;
};

const estado = elemento<HTMLSpanElement>("estado");
const pasos = elemento<HTMLUListElement>("pasos");
const video = elemento<HTMLVideoElement>("pantalla");
const tarjeta = elemento<HTMLDivElement>("tarjeta");

const expuesto: Expuesto = {
  identidad: null,
  pasos: [],
  error: null,
  pantallaViva: false,
  fotogramas: () => Promise.resolve(0),
  diagnostico: () => Promise.resolve({}),
  conectar: () => Promise.reject(new Error("todavia no hay identidad")),
};
(globalThis as unknown as { nexus: Expuesto }).nexus = expuesto;

function paso(texto: string, mal = false): void {
  expuesto.pasos.push(texto);
  const linea = document.createElement("li");
  linea.textContent = texto;
  if (mal) {
    linea.className = "mal";
    expuesto.error = texto;
  }
  pasos.append(linea);
  pasos.scrollTop = pasos.scrollHeight;
  estado.textContent = texto;
}

/** Guarda lo ultimo que funciono, para no teclearlo en cada visita. */
const recordar = (servidor: string, host: string): void => {
  try {
    localStorage.setItem("nexus-servidor", servidor);
    localStorage.setItem("nexus-host", host);
  } catch {
    // Navegacion privada: no poder recordarlo no es un fallo.
  }
};

const recordado = (clave: string): string => {
  try {
    return localStorage.getItem(clave) ?? "";
  } catch {
    return "";
  }
};

async function conectar(identidad: Identidad, servidor: string, hostId: string): Promise<void> {
  paso(`conectando a ${servidor}`);
  const sig = await Senalizacion.abrir(servidor);
  paso("socket abierto; autenticando");

  const aceptado = await sig.autenticar(identidad, nombreDelDispositivo());
  paso(`autenticado; ${aceptado.iceServers.length} servidor(es) ICE`);

  const sesion = await Sesion.abrir(sig, identidad, hostId, aceptado.iceServers, {
    paso: (texto) => paso(texto),
    pantalla: (flujo) => {
      video.srcObject = flujo;
      video.classList.add("viva");
      tarjeta.style.display = "none";
      expuesto.pantallaViva = true;
      void video.play().catch(() => paso("el navegador no dejo arrancar el video", true));
    },
    evento: (evento) => {
      if (evento["event"] === "screen_stopped") {
        paso(`el PC dejo de enviar la pantalla: ${String(evento["reason"])}`, true);
      }
    },
  });

  expuesto.fotogramas = () => sesion.fotogramasDecodificados();
  expuesto.diagnostico = () => sesion.diagnostico();
  conectarRaton(sesion);
  recordar(servidor, hostId);
}

/** El raton: posicion normalizada sobre la imagen, como espera el protocolo. */
function conectarRaton(sesion: Sesion): void {
  const normalizar = (evento: PointerEvent): { x: number; y: number } | null => {
    const caja = video.getBoundingClientRect();
    if (caja.width === 0 || caja.height === 0) {
      return null;
    }
    // `object-fit: contain` deja bandas: hay que descontarlas o el puntero
    // iria desplazado justo en los bordes.
    const relacionVideo = video.videoWidth / video.videoHeight;
    const relacionCaja = caja.width / caja.height;
    let ancho = caja.width;
    let alto = caja.height;
    if (relacionVideo > relacionCaja) {
      alto = caja.width / relacionVideo;
    } else {
      ancho = caja.height * relacionVideo;
    }
    const izquierda = caja.left + (caja.width - ancho) / 2;
    const arriba = caja.top + (caja.height - alto) / 2;
    const x = (evento.clientX - izquierda) / ancho;
    const y = (evento.clientY - arriba) / alto;
    if (x < 0 || x > 1 || y < 0 || y > 1) {
      return null;
    }
    return { x, y };
  };

  video.addEventListener("pointermove", (evento) => {
    const punto = normalizar(evento);
    if (punto) {
      sesion.enviarEntrada({ kind: "pointer_move", x: punto.x, y: punto.y });
    }
  });
  video.addEventListener("pointerdown", (evento) => {
    const punto = normalizar(evento);
    if (!punto) {
      return;
    }
    sesion.enviarEntrada({ kind: "pointer_move", x: punto.x, y: punto.y });
    sesion.enviarEntrada({ kind: "pointer_button", button: boton(evento), state: "down" });
  });
  video.addEventListener("pointerup", (evento) => {
    sesion.enviarEntrada({ kind: "pointer_button", button: boton(evento), state: "up" });
  });
  video.addEventListener("contextmenu", (evento) => evento.preventDefault());
}

const boton = (evento: PointerEvent): string =>
  evento.button === 2 ? "right" : evento.button === 1 ? "middle" : "left";

/** Un nombre que la persona reconozca en el panel del PC. */
function nombreDelDispositivo(): string {
  const ua = navigator.userAgent;
  if (/iPad/.test(ua)) {
    return "iPad (navegador)";
  }
  if (/iPhone/.test(ua)) {
    return "iPhone (navegador)";
  }
  return "Navegador";
}

async function arrancar(): Promise<void> {
  if (!window.isSecureContext) {
    paso("esta pagina necesita HTTPS: sin contexto seguro no hay claves ni WebRTC", true);
    return;
  }

  const identidad = await cargarIdentidad();
  expuesto.identidad = {
    dispositivoId: identidad.dispositivoId,
    clavePublicaHex: identidad.clavePublicaHex,
  };
  elemento("id").textContent = identidad.dispositivoId;
  elemento("clave").textContent = identidad.clavePublicaHex;
  paso("identidad lista");

  const campoServidor = elemento<HTMLInputElement>("servidor");
  const campoHost = elemento<HTMLInputElement>("host");
  const parametros = new URLSearchParams(location.search);
  campoServidor.value = parametros.get("servidor") ?? recordado("nexus-servidor");
  campoHost.value = parametros.get("host") ?? recordado("nexus-host");

  const intentar = async (servidor: string, hostId: string): Promise<void> => {
    try {
      await conectar(identidad, servidor, hostId);
    } catch (error) {
      paso(`fallo: ${error instanceof Error ? error.message : String(error)}`, true);
    }
  };

  expuesto.conectar = (servidor, hostId) => intentar(servidor, hostId);
  elemento("conectar").addEventListener("click", () => {
    void intentar(campoServidor.value.trim(), campoHost.value.trim());
  });

  if (campoServidor.value && campoHost.value && parametros.has("servidor")) {
    await intentar(campoServidor.value, campoHost.value);
  }
}

void arrancar().catch((error: unknown) => {
  paso(`no se pudo arrancar: ${error instanceof Error ? error.message : String(error)}`, true);
});
