import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
import mime from 'mime-types';
import { env } from '../env.js';
import { getMediaMeta, downloadMedia } from '../whatsapp/client.js';
import { setMessageMedia } from './store.js';

/** Extensión segura a partir del mime; nunca confiamos en el nombre remoto. */
function extFor(mimeType: string | null | undefined, fallback = 'bin'): string {
  if (!mimeType) return fallback;
  const base = mimeType.split(';')[0].trim();
  const ext = mime.extension(base);
  return typeof ext === 'string' && ext.length <= 8 ? ext : fallback;
}

/** Evita que un filename del remoto escape del directorio de media. */
export function safeFilename(name: string | null | undefined, fallback: string): string {
  if (!name) return fallback;
  const base = path.basename(name).replace(/[^\w.\-\s()]/g, '_').trim();
  return base.length > 0 && base.length <= 180 ? base : fallback;
}

export function mediaAbsolutePath(relative: string): string {
  const abs = path.resolve(env.mediaDir, relative);
  if (!abs.startsWith(path.resolve(env.mediaDir) + path.sep)) {
    throw new Error('Ruta de media fuera del directorio permitido');
  }
  return abs;
}

/**
 * Descarga el adjunto de un mensaje entrante y lo guarda en disco.
 * Se ejecuta en segundo plano: el mensaje ya se mostró con media_state='pending'.
 */
export async function fetchInboundMedia(messageId: number, mediaId: string): Promise<void> {
  try {
    const meta = await getMediaMeta(mediaId);
    const bytes = await downloadMedia(meta.url);

    const ext = extFor(meta.mime_type);
    const name = `${Date.now()}-${crypto.randomBytes(6).toString('hex')}.${ext}`;
    const dir = path.join(env.mediaDir, 'inbound');
    await fs.mkdir(dir, { recursive: true });
    await fs.writeFile(path.join(dir, name), bytes);

    setMessageMedia(messageId, {
      path: path.join('inbound', name),
      mime: meta.mime_type,
      size: bytes.byteLength,
      state: 'ready',
    });
  } catch (err) {
    console.error(`[media] fallo al descargar ${mediaId}:`, (err as Error).message);
    setMessageMedia(messageId, { state: 'error' });
  }
}

/** Guarda un archivo que el usuario envía desde el panel. */
export async function storeOutboundMedia(
  bytes: Buffer,
  mimeType: string,
  originalName: string
): Promise<{ relativePath: string; size: number }> {
  const ext = extFor(mimeType, extFor(mime.lookup(originalName) || undefined));
  const name = `${Date.now()}-${crypto.randomBytes(6).toString('hex')}.${ext}`;
  const dir = path.join(env.mediaDir, 'outbound');
  await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(path.join(dir, name), bytes);
  return { relativePath: path.join('outbound', name), size: bytes.byteLength };
}

/** Clasifica un mime en el tipo de mensaje que espera la Cloud API. */
export function kindForMime(mimeType: string): 'image' | 'video' | 'audio' | 'document' | 'sticker' {
  const m = mimeType.split(';')[0].trim().toLowerCase();
  if (m === 'image/webp') return 'sticker';
  if (m.startsWith('image/')) return 'image';
  if (m.startsWith('video/')) return 'video';
  if (m.startsWith('audio/')) return 'audio';
  return 'document';
}
