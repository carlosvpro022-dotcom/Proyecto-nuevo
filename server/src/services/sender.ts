import { db } from '../db/index.js';
import { broadcast } from '../realtime/hub.js';
import * as wa from '../whatsapp/client.js';
import { WhatsAppError } from '../whatsapp/client.js';
import {
  getOrCreateConversation,
  insertMessage,
  serializeMessage,
  touchConversation,
  emitConversation,
  type MessageRow,
} from './store.js';
import type { SendResult } from '../whatsapp/types.js';

function finalize(row: MessageRow, wamId: string | null, status: string, error?: WhatsAppError): MessageRow {
  db.prepare(
    'UPDATE messages SET wam_id = COALESCE(?, wam_id), status = ?, error_code = ?, error_message = ? WHERE id = ?'
  ).run(
    wamId,
    status,
    error?.code !== undefined ? String(error.code) : null,
    error ? error.message : null,
    row.id
  );
  const updated = db.prepare('SELECT * FROM messages WHERE id = ?').get(row.id) as MessageRow;
  broadcast({ type: 'message:update', payload: serializeMessage(updated) });
  return updated;
}

/**
 * Inserta el mensaje como 'pending' (para que aparezca ya en la UI), lo envía
 * y refleja el resultado. Un fallo de Meta no revienta la petición: el mensaje
 * queda marcado como 'failed' con el motivo, igual que en WhatsApp.
 */
async function dispatch(
  waId: string,
  draft: Parameters<typeof insertMessage>[0],
  call: () => Promise<SendResult>
): Promise<{ message: ReturnType<typeof serializeMessage>; error: WhatsAppError | null }> {
  const conv = getOrCreateConversation(waId);
  const row = insertMessage({ ...draft, conversationId: conv.id, waId, direction: 'out' });
  broadcast({ type: 'message:new', payload: serializeMessage(row) });
  touchConversation(conv.id, { messageAt: row.timestamp, inbound: false });
  emitConversation(conv.id);

  try {
    const res = await call();
    const wamId = res.messages?.[0]?.id ?? null;
    const updated = finalize(row, wamId, 'sent');
    return { message: serializeMessage(updated), error: null };
  } catch (err) {
    const waErr =
      err instanceof WhatsAppError ? err : new WhatsAppError((err as Error).message, 500);
    const updated = finalize(row, null, 'failed', waErr);
    return { message: serializeMessage(updated), error: waErr };
  }
}

export function sendText(waId: string, text: string, replyTo?: string) {
  return dispatch(
    waId,
    { conversationId: 0, waId, direction: 'out', type: 'text', body: text, contextWamId: replyTo ?? null },
    () => wa.sendText(waId, text, replyTo)
  );
}

export function sendMedia(
  waId: string,
  kind: wa.MediaKind,
  mediaId: string,
  opts: { caption?: string; filename?: string; localPath?: string; mime?: string; size?: number },
  replyTo?: string
) {
  return dispatch(
    waId,
    {
      conversationId: 0,
      waId,
      direction: 'out',
      type: kind,
      body: opts.caption ?? null,
      contextWamId: replyTo ?? null,
      mediaId,
      mediaPath: opts.localPath ?? null,
      mediaMime: opts.mime ?? null,
      mediaFilename: opts.filename ?? null,
      mediaSize: opts.size ?? null,
      mediaState: opts.localPath ? 'ready' : 'pending',
    },
    () => wa.sendMedia(waId, kind, { id: mediaId }, { caption: opts.caption, filename: opts.filename }, replyTo)
  );
}

export function sendTemplate(waId: string, name: string, language: string, components?: unknown[]) {
  return dispatch(
    waId,
    {
      conversationId: 0,
      waId,
      direction: 'out',
      type: 'template',
      body: `Plantilla: ${name}`,
      payload: { template: name, language, components: components ?? [] },
    },
    () => wa.sendTemplate(waId, name, language, components)
  );
}

export function sendProduct(waId: string, catalogId: string, retailerId: string, bodyText?: string) {
  return dispatch(
    waId,
    {
      conversationId: 0,
      waId,
      direction: 'out',
      type: 'interactive',
      body: bodyText ?? 'Producto del catálogo',
      payload: { interactive: 'product', catalogId, retailerId },
    },
    () => wa.sendProduct(waId, catalogId, retailerId, bodyText)
  );
}

export function sendProductList(
  waId: string,
  catalogId: string,
  header: string,
  bodyText: string,
  retailerIds: string[]
) {
  const sections = [
    { title: header.slice(0, 24) || 'Productos', product_items: retailerIds.map((id) => ({ product_retailer_id: id })) },
  ];
  return dispatch(
    waId,
    {
      conversationId: 0,
      waId,
      direction: 'out',
      type: 'interactive',
      body: bodyText,
      payload: { interactive: 'product_list', catalogId, retailerIds },
    },
    () => wa.sendProductList(waId, catalogId, header, bodyText, sections)
  );
}

export function sendLocation(
  waId: string,
  loc: { latitude: number; longitude: number; name?: string; address?: string },
  replyTo?: string
) {
  return dispatch(
    waId,
    {
      conversationId: 0,
      waId,
      direction: 'out',
      type: 'location',
      body: loc.name ?? `${loc.latitude}, ${loc.longitude}`,
      payload: loc,
      contextWamId: replyTo ?? null,
    },
    () => wa.sendLocation(waId, loc, replyTo)
  );
}

/** Las reacciones no se guardan como mensaje: se adjuntan al mensaje destino. */
export async function sendReaction(waId: string, targetWamId: string, emoji: string): Promise<void> {
  await wa.sendReaction(waId, targetWamId, emoji);
  const target = db.prepare('SELECT * FROM messages WHERE wam_id = ?').get(targetWamId) as
    | MessageRow
    | undefined;
  if (target) {
    db.prepare('UPDATE messages SET reaction_emoji = ? WHERE id = ?').run(emoji || null, target.id);
    const updated = db.prepare('SELECT * FROM messages WHERE id = ?').get(target.id) as MessageRow;
    broadcast({ type: 'message:update', payload: serializeMessage(updated) });
  }
}
