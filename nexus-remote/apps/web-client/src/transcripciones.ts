//! Los dos mensajes que este navegador firma, byte a byte.
//
// Estas dos disposiciones **tienen que coincidir exactamente** con las de
// `crates/security` en el host y el servidor. Un solo byte distinto y la firma
// no verifica; el otro extremo no puede decir donde esta el error, porque lo
// unico que ve es que no cuadra. Por eso estan aqui solas, explicitas y
// probadas contra vectores calculados por el codigo de Rust.

import { Transcripcion, hexABytes, uuidABytes } from "./bytes.js";

/** Separador de dominio de la autenticacion mutua. */
const DOMINIO_AUTENTICACION = "NEXUS-REMOTE/v1/mutual-auth";
/** Separador de dominio de la atadura de sesion. */
const DOMINIO_ATADURA = "NEXUS-REMOTE/v1/session-binding";
/** Version del protocolo que se firma. */
const VERSION_PROTOCOLO = 1;
/** Byte que identifica al papel de controlador en las transcripciones. */
const ROL_CONTROLADOR = 0x02;

/** Byte de cada algoritmo de huella admitido. */
const ALGORITMOS: Record<string, number> = {
  "sha-256": 0x01,
  "sha-384": 0x02,
  "sha-512": 0x03,
};

/** Huella del certificado DTLS de un extremo. */
export interface Huella {
  /** Nombre tal como aparece en el SDP, por ejemplo `sha-256`. */
  readonly algoritmo: string;
  /** Resumen en bytes. */
  readonly digest: Uint8Array;
}

/**
 * Saca la huella DTLS de un SDP.
 *
 * @throws si el SDP no trae huella, lo que significa que no hay DTLS y por
 * tanto no hay nada que atar.
 */
export function huellaDeSdp(sdp: string): Huella {
  const linea = /^a=fingerprint:(\S+)\s+(\S+)/m.exec(sdp);
  if (!linea?.[1] || !linea[2]) {
    throw new Error("el SDP no trae huella DTLS");
  }
  const algoritmo = linea[1].toLowerCase();
  if (!(algoritmo in ALGORITMOS)) {
    throw new Error(`algoritmo de huella no admitido: ${algoritmo}`);
  }
  return { algoritmo, digest: hexABytes(linea[2].replace(/:/g, "")) };
}

/** Escribe la huella como la escribe el SDP, que es lo que viaja en la prueba. */
export function huellaCanonica(huella: Huella): string {
  const hex = Array.from(huella.digest, (b) => b.toString(16).padStart(2, "0").toUpperCase());
  return `${huella.algoritmo} ${hex.join(":")}`;
}

function anadirHuella(t: Transcripcion, huella: Huella): void {
  const tag = ALGORITMOS[huella.algoritmo];
  if (tag === undefined) {
    throw new Error(`algoritmo de huella no admitido: ${huella.algoritmo}`);
  }
  t.byte(tag).byte(huella.digest.length).bytes(huella.digest);
}

/** Lo que hace falta para firmar el reto del servidor. */
export interface DatosDeAutenticacion {
  readonly nonceServidorHex: string;
  readonly nonceClienteHex: string;
  readonly dispositivoId: string;
  /**
   * Marca de tiempo que va en el sobre, en milisegundos.
   *
   * Es la del cliente, no la del servidor: el servidor verifica la firma
   * contra la marca con la que llega el mensaje.
   */
  readonly marca: number;
}

/**
 * La transcripcion que se firma para autenticarse ante el servidor.
 *
 * Es la misma que construye `AuthTranscript::to_bytes` en Rust.
 */
export function transcripcionDeAutenticacion(datos: DatosDeAutenticacion): Uint8Array {
  return new Transcripcion()
    .texto(DOMINIO_AUTENTICACION)
    .byte(0x00)
    .byte(ROL_CONTROLADOR)
    .u16le(VERSION_PROTOCOLO)
    .bytes(hexABytes(datos.nonceServidorHex))
    .bytes(hexABytes(datos.nonceClienteHex))
    .bytes(uuidABytes(datos.dispositivoId))
    .u64le(datos.marca)
    .terminar();
}

/** Lo que hace falta para firmar la atadura de sesion. */
export interface DatosDeAtadura {
  readonly sesionId: string;
  /** Este dispositivo. */
  readonly miId: string;
  /** El PC al otro lado. */
  readonly hostId: string;
  /** Huella del canal cifrado de este extremo. */
  readonly miHuella: Huella;
  /** Huella del canal cifrado del otro extremo. */
  readonly huellaDelHost: Huella;
}

/**
 * La transcripcion que ata la sesion al canal cifrado concreto.
 *
 * Es la misma que construye `SessionBinding::to_bytes` en Rust. Firmarla es lo
 * que demuestra que quien habla por **este** canal DTLS es el dueno de la
 * clave: sin ella, un servidor manipulado podria ponerse en medio.
 */
export function transcripcionDeAtadura(datos: DatosDeAtadura): Uint8Array {
  const t = new Transcripcion()
    .texto(DOMINIO_ATADURA)
    .byte(0x00)
    .byte(ROL_CONTROLADOR)
    .u16le(VERSION_PROTOCOLO)
    .bytes(uuidABytes(datos.sesionId))
    .bytes(uuidABytes(datos.miId))
    .bytes(uuidABytes(datos.hostId));
  anadirHuella(t, datos.miHuella);
  anadirHuella(t, datos.huellaDelHost);
  return t.terminar();
}
