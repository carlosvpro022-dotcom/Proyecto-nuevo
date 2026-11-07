//! La identidad del navegador: una clave Ed25519 que no sale de aqui.
//
// ## Lo que se decide en este archivo
//
// **La clave privada no es extraible.** Se genera con `extractable: false`, asi
// que ningun script puede leerla, ni este. El navegador firma con ella pero no
// la entrega. Si alguien se cuela en la pagina puede pedir firmas mientras esta
// abierta, pero no puede llevarse la identidad del dispositivo.
//
// **La guarda IndexedDB, no `localStorage`.** `localStorage` solo guarda texto,
// asi que obligaria a exportar la clave para guardarla, que es justo lo que no
// se quiere. IndexedDB guarda el objeto `CryptoKey` tal cual.
//
// **No se implementa criptografia.** Todo sale de `crypto.subtle`, que es la
// del navegador.

import { bytesAHex, uuidNuevo } from "./bytes.js";

/** Nombre de la base y del almacen donde vive la identidad. */
const BASE = "nexus-remote";
const ALMACEN = "identidad";
const CLAVE = "dispositivo";

/** La identidad de este navegador como dispositivo del sistema. */
export interface Identidad {
  /** Identificador del dispositivo, en forma de UUID. */
  readonly dispositivoId: string;
  /** Clave publica en hexadecimal: es lo que se empareja con el PC. */
  readonly clavePublicaHex: string;
  /** Firma una transcripcion con la clave privada. */
  firmar(mensaje: Uint8Array): Promise<Uint8Array>;
}

interface Guardado {
  id: string;
  dispositivoId: string;
  privada: CryptoKey;
  publica: CryptoKey;
}

function abrirBase(): Promise<IDBDatabase> {
  return new Promise((resolver, rechazar) => {
    const peticion = indexedDB.open(BASE, 1);
    peticion.onupgradeneeded = () => {
      peticion.result.createObjectStore(ALMACEN, { keyPath: "id" });
    };
    peticion.onsuccess = () => resolver(peticion.result);
    peticion.onerror = () => rechazar(peticion.error ?? new Error("no se pudo abrir IndexedDB"));
  });
}

function leer(base: IDBDatabase): Promise<Guardado | undefined> {
  return new Promise((resolver, rechazar) => {
    const peticion = base.transaction(ALMACEN, "readonly").objectStore(ALMACEN).get(CLAVE);
    peticion.onsuccess = () => resolver(peticion.result as Guardado | undefined);
    peticion.onerror = () => rechazar(peticion.error ?? new Error("no se pudo leer la identidad"));
  });
}

function escribir(base: IDBDatabase, valor: Guardado): Promise<void> {
  return new Promise((resolver, rechazar) => {
    const tx = base.transaction(ALMACEN, "readwrite");
    tx.objectStore(ALMACEN).put(valor);
    tx.oncomplete = () => resolver();
    tx.onerror = () => rechazar(tx.error ?? new Error("no se pudo guardar la identidad"));
  });
}

/**
 * Carga la identidad de este navegador, y la crea la primera vez.
 *
 * La misma identidad en los siguientes arranques: si cambiara, el PC veria un
 * dispositivo desconocido y habria que emparejarlo otra vez.
 */
export async function cargarIdentidad(): Promise<Identidad> {
  const base = await abrirBase();
  let guardado = await leer(base);

  if (!guardado) {
    const par = (await crypto.subtle.generateKey({ name: "Ed25519" }, false, [
      "sign",
      "verify",
    ])) as CryptoKeyPair;
    guardado = {
      id: CLAVE,
      dispositivoId: uuidNuevo(),
      privada: par.privateKey,
      publica: par.publicKey,
    };
    await escribir(base, guardado);
  }

  const bruta = await crypto.subtle.exportKey("raw", guardado.publica);
  const identidad = guardado;

  return {
    dispositivoId: identidad.dispositivoId,
    clavePublicaHex: bytesAHex(new Uint8Array(bruta)),
    async firmar(mensaje: Uint8Array): Promise<Uint8Array> {
      const firma = await crypto.subtle.sign(
        { name: "Ed25519" },
        identidad.privada,
        mensaje as unknown as BufferSource,
      );
      return new Uint8Array(firma);
    },
  };
}

/** Borra la identidad guardada. Deja de estar emparejado con ningun PC. */
export async function olvidarIdentidad(): Promise<void> {
  const base = await abrirBase();
  await new Promise<void>((resolver, rechazar) => {
    const tx = base.transaction(ALMACEN, "readwrite");
    tx.objectStore(ALMACEN).delete(CLAVE);
    tx.oncomplete = () => resolver();
    tx.onerror = () => rechazar(tx.error ?? new Error("no se pudo borrar la identidad"));
  });
}
