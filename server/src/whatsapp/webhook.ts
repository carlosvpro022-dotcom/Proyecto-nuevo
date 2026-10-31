import crypto from 'node:crypto';
import type { Request, Response } from 'express';
import { env, whatsappConfigured } from '../env.js';
import { claimEvent, getSetting } from '../db/index.js';
import { broadcast } from '../realtime/hub.js';
import {
  getOrCreateConversation,
  insertMessage,
  serializeMessage,
  touchConversation,
  emitConversation,
  updateMessageStatus,
  upsertContact,
} from '../services/store.js';
import { fetchInboundMedia } from '../services/media.js';
import { pickAutoReply } from '../services/autoreply.js';
import * as sender from '../services/sender.js';
import { markAsRead } from './client.js';
import type { InboundMessage, MediaObject, StatusUpdate, WebhookBody } from './types.js';

/** GET /webhook — handshake de verificación de Meta. */
export function verifyWebhook(req: Request, res: Response): void {
  const mode = req.query['hub.mode'];
  const token = req.query['hub.verify_token'];
  const challenge = req.query['hub.challenge'];

  if (mode === 'subscribe' && typeof token === 'string' && env.verifyToken && token === env.verifyToken) {
    res.status(200).send(String(challenge ?? ''));
    return;
  }
  console.warn('[webhook] verificación rechazada: token no coincide');
  res.sendStatus(403);
}

/** Comprueba la cabecera X-Hub-Signature-256 contra el App Secret. */
export function signatureIsValid(req: Request): boolean {
  if (!env.appSecret) return env.allowUnsignedWebhooks;

  const header = req.get('x-hub-signature-256');
  if (!header || !header.startsWith('sha256=')) return false;

  const raw = (req as Request & { rawBody?: Buffer }).rawBody;
  if (!raw) return false;

  const expected = 'sha256=' + crypto.createHmac('sha256', env.appSecret).update(raw).digest('hex');
  const a = Buffer.from(header);
  const b = Buffer.from(expected);
  return a.length === b.length && crypto.timingSafeEqual(a, b);
}

function mediaOf(msg: InboundMessage): { kind: string; obj: MediaObject } | null {
  for (const kind of ['image', 'video', 'audio', 'document', 'sticker'] as const) {
    const obj = msg[kind];
    if (obj) return { kind, obj };
  }
  return null;
}

function bodyOf(msg: InboundMessage): string | null {
  switch (msg.type) {
    case 'text':
      return msg.text?.body ?? null;
    case 'button':
      return msg.button?.text ?? null;
    case 'interactive':
      return msg.interactive?.button_reply?.title ?? msg.interactive?.list_reply?.title ?? null;
    case 'location':
      return msg.location?.name ?? msg.location?.address ?? null;
    case 'system':
      return msg.system?.body ?? null;
    case 'order':
      return msg.order?.text ?? 'Pedido recibido';
    case 'unsupported':
      return 'Mensaje no soportado por la Cloud API';
    default:
      return mediaOf(msg)?.obj.caption ?? null;
  }
}

function payloadOf(msg: InboundMessage): unknown {
  switch (msg.type) {
    case 'location':
      return msg.location ?? null;
    case 'contacts':
      return msg.contacts ?? null;
    case 'order':
      return msg.order ?? null;
    case 'interactive':
      return msg.interactive ?? null;
    case 'button':
      return msg.button ?? null;
    case 'system':
      return msg.system ?? null;
    default:
      return null;
  }
}

async function handleInbound(msg: InboundMessage, profileName?: string | null): Promise<void> {
  const waId = msg.from;
  if (!waId) return;
  if (msg.id && !claimEvent(`msg:${msg.id}`)) return; // reintento de Meta ya procesado

  const timestamp = msg.timestamp ? Number(msg.timestamp) * 1000 : Date.now();
  const conv = getOrCreateConversation(waId, profileName);

  // Las reacciones modifican el mensaje reaccionado, no crean uno nuevo.
  if (msg.type === 'reaction') {
    const target = msg.reaction?.message_id;
    const emoji = msg.reaction?.emoji ?? null;
    if (target) {
      const { db } = await import('../db/index.js');
      const row = db.prepare('SELECT id FROM messages WHERE wam_id = ?').get(target) as
        | { id: number }
        | undefined;
      if (row) {
        db.prepare('UPDATE messages SET reaction_emoji = ? WHERE id = ?').run(emoji, row.id);
        const updated = db.prepare('SELECT * FROM messages WHERE id = ?').get(row.id) as any;
        broadcast({ type: 'message:update', payload: serializeMessage(updated) });
      }
    }
    return;
  }

  const media = mediaOf(msg);
  const row = insertMessage({
    wamId: msg.id ?? null,
    conversationId: conv.id,
    waId,
    direction: 'in',
    type: msg.type ?? 'unsupported',
    body: bodyOf(msg),
    status: 'received',
    timestamp,
    contextWamId: msg.context?.id ?? null,
    mediaId: media?.obj.id ?? null,
    mediaMime: media?.obj.mime_type ?? null,
    mediaFilename: media?.obj.filename ?? null,
    mediaState: media ? 'pending' : null,
    payload: payloadOf(msg),
  });

  touchConversation(conv.id, { messageAt: timestamp, inbound: true, incrementUnread: true });
  broadcast({ type: 'message:new', payload: serializeMessage(row) });
  emitConversation(conv.id);

  // Descarga del adjunto en segundo plano: no bloquea el ack del webhook.
  if (media?.obj.id) {
    void fetchInboundMedia(row.id, media.obj.id);
  }

  // Confirmación de lectura automática, si está activada en ajustes.
  if (msg.id && getSetting('autoMarkRead') === 'true') {
    markAsRead(msg.id).catch((e) => console.warn('[webhook] markAsRead falló:', e.message));
  }

  // Respuestas automáticas.
  try {
    const choice = pickAutoReply({ conversationId: conv.id, waId, text: bodyOf(msg) });
    if (choice) {
      const result = await sender.sendText(waId, choice.text);
      if (result.error) {
        console.warn('[autoreply] no se pudo enviar:', result.error.message);
      }
    }
  } catch (e) {
    console.error('[autoreply] error:', (e as Error).message);
  }
}

function handleStatus(st: StatusUpdate): void {
  if (!st.id || !st.status) return;
  if (!claimEvent(`st:${st.id}:${st.status}`)) return;

  const err = st.errors?.[0];
  updateMessageStatus(st.id, st.status, {
    code: err?.code,
    message: err ? err.error_data?.details ?? err.message ?? err.title : undefined,
  });

  if (st.recipient_id) {
    const conv = getOrCreateConversation(st.recipient_id);
    emitConversation(conv.id);
  }
}

/** POST /webhook — recepción de eventos. Siempre respondemos 200 rápido. */
export async function receiveWebhook(req: Request, res: Response): Promise<void> {
  if (!signatureIsValid(req)) {
    console.warn('[webhook] firma inválida: evento descartado');
    res.sendStatus(401);
    return;
  }

  // Meta reintenta si no recibe 200 en pocos segundos.
  res.sendStatus(200);

  const body = req.body as WebhookBody;
  try {
    for (const entry of body.entry ?? []) {
      for (const change of entry.changes ?? []) {
        const value = change.value;
        if (!value) continue;

        for (const contact of value.contacts ?? []) {
          if (contact.wa_id) upsertContact(contact.wa_id, contact.profile?.name ?? null);
        }

        const nameByWaId = new Map<string, string | undefined>();
        for (const c of value.contacts ?? []) {
          if (c.wa_id) nameByWaId.set(c.wa_id, c.profile?.name);
        }

        for (const msg of value.messages ?? []) {
          await handleInbound(msg, msg.from ? nameByWaId.get(msg.from) : undefined);
        }
        for (const st of value.statuses ?? []) {
          handleStatus(st);
        }
        for (const e of value.errors ?? []) {
          console.error('[webhook] error reportado por Meta:', e.code, e.title, e.message);
        }
      }
    }
  } catch (err) {
    console.error('[webhook] fallo procesando evento:', err);
  }
}

export function webhookStatus() {
  return {
    configured: whatsappConfigured,
    signatureEnforced: Boolean(env.appSecret) || !env.allowUnsignedWebhooks,
    verifyTokenSet: Boolean(env.verifyToken),
  };
}
