//! Conversiones entre hexadecimal, UUID y bytes.
//
// Vive aparte porque lo usan la autenticacion y la atadura de sesion, y un
// error de un solo byte aqui hace que el host rechace la conexion sin decir
// por que. Son funciones puras y se prueban solas.

/** Convierte una cadena hexadecimal en bytes. */
export function hexABytes(hex: string): Uint8Array {
  const limpio = hex.trim().toLowerCase();
  if (limpio.length % 2 !== 0 || /[^0-9a-f]/.test(limpio)) {
    throw new Error(`hexadecimal invalido: ${hex}`);
  }
  const out = new Uint8Array(limpio.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(limpio.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/** Convierte bytes en hexadecimal minusculo, que es lo que espera el protocolo. */
export function bytesAHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Los 16 bytes de un UUID, en el orden en que se escribe.
 *
 * Es el mismo orden que da `Uuid::as_bytes()` en Rust, que es lo que se firma.
 */
export function uuidABytes(uuid: string): Uint8Array {
  const hex = uuid.replace(/-/g, "");
  if (hex.length !== 32) {
    throw new Error(`UUID invalido: ${uuid}`);
  }
  return hexABytes(hex);
}

/** Un UUID version 4 del generador del sistema. */
export function uuidNuevo(): string {
  return crypto.randomUUID();
}

/** Acumula bytes para una transcripcion que se va a firmar. */
export class Transcripcion {
  private readonly trozos: Uint8Array[] = [];

  /** Anade texto ASCII, como un dominio de separacion. */
  texto(valor: string): this {
    this.trozos.push(new TextEncoder().encode(valor));
    return this;
  }

  /** Anade un solo byte. */
  byte(valor: number): this {
    this.trozos.push(new Uint8Array([valor & 0xff]));
    return this;
  }

  /** Anade un entero de 16 bits en orden de byte menos significativo primero. */
  u16le(valor: number): this {
    const buf = new Uint8Array(2);
    new DataView(buf.buffer).setUint16(0, valor, true);
    this.trozos.push(buf);
    return this;
  }

  /** Anade un entero de 64 bits en orden de byte menos significativo primero. */
  u64le(valor: number): this {
    const buf = new Uint8Array(8);
    new DataView(buf.buffer).setBigUint64(0, BigInt(valor), true);
    this.trozos.push(buf);
    return this;
  }

  /** Anade bytes tal cual. */
  bytes(valor: Uint8Array): this {
    this.trozos.push(valor);
    return this;
  }

  /** Los bytes acumulados. */
  terminar(): Uint8Array {
    const total = this.trozos.reduce((suma, t) => suma + t.length, 0);
    const out = new Uint8Array(total);
    let offset = 0;
    for (const trozo of this.trozos) {
      out.set(trozo, offset);
      offset += trozo.length;
    }
    return out;
  }
}
