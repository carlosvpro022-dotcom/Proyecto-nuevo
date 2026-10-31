import { Router } from 'express';
import fs from 'node:fs';
import multer from 'multer';
import { z } from 'zod';
import { db } from '../db/index.js';
import { broadcast } from '../realtime/hub.js';
import {
  conversationById,
  getOrCreateConversation,
  serializeConversation,
  serializeMessage,
  emitConversation,
  type ConversationRow,
  type MessageRow,
} from '../services/store.js';
import { mediaAbsolutePath, storeOutboundMedia, kindForMime, safeFilename } from '../services/media.js';
import * as sender from '../services/sender.js';
import * as wa from '../whatsapp/client.js';
import { WhatsAppError } from '../whatsapp/client.js';

export const conversationsRouter = Router();

const upload = multer({
  storage: multer.memoryStorage(),
  // La Cloud API tope en 100 MB para documentos/vídeo; cortamos antes.
  limits: { fileSize: 95 * 1024 * 1024 },
});

/** Normaliza un número a wa_id: solo dígitos, sin '+' ni separadores. */
function normalizeWaId(input: string): string {
  const digits = input.replace(/\D/g, '');
  return digits;
}

const waIdSchema = z
  .string()
  .min(6)
  .max(20)
  .transform(normalizeWaId)
  .refine((v) => /^\d{6,20}$/.test(v), 'Número inválido: usa formato internacional, p. ej. 34600111222');

// ─── Listado de conversaciones ──────────────────────────────────────────────

conversationsRouter.get('/conversations', (req, res) => {
  const q = String(req.query.q ?? '').trim().toLowerCase();
  const filter = String(req.query.filter ?? 'all'); // all | unread | archived | labelled
  const labelId = req.query.labelId ? Number(req.query.labelId) : null;

  let sql = `SELECT c.* FROM conversations c
             LEFT JOIN contacts ct ON ct.wa_id = c.wa_id`;
  const where: string[] = [];
  const params: Record<string, unknown> = {};

  if (filter === 'archived') where.push('c.is_archived = 1');
  else where.push('c.is_archived = 0');

  if (filter === 'unread') where.push('c.unread_count > 0');

  if (labelId) {
    sql += ' JOIN contact_labels cl ON cl.wa_id = c.wa_id';
    where.push('cl.label_id = @labelId');
    params.labelId = labelId;
  }

  if (q) {
    where.push(
      `(LOWER(COALESCE(ct.display_name,'')) LIKE @q
        OR LOWER(COALESCE(ct.profile_name,'')) LIKE @q
        OR c.wa_id LIKE @q
        OR EXISTS (SELECT 1 FROM messages m
                    WHERE m.conversation_id = c.id AND LOWER(COALESCE(m.body,'')) LIKE @q))`
    );
    params.q = `%${q}%`;
  }

  if (where.length) sql += ' WHERE ' + where.join(' AND ');
  sql += ' ORDER BY c.is_pinned DESC, COALESCE(c.last_message_at, c.created_at) DESC LIMIT 300';

  const rows = db.prepare(sql).all(params) as ConversationRow[];
  res.json({ conversations: rows.map(serializeConversation) });
});

conversationsRouter.get('/conversations/:id', (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });
  res.json(serializeConversation(conv));
});

/** Abre (o crea) una conversación a partir de un número. */
conversationsRouter.post('/conversations', (req, res) => {
  const parsed = z.object({ waId: waIdSchema, name: z.string().trim().max(120).optional() }).safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.issues[0].message });

  const conv = getOrCreateConversation(parsed.data.waId);
  if (parsed.data.name) {
    db.prepare('UPDATE contacts SET display_name = ?, updated_at = ? WHERE wa_id = ?').run(
      parsed.data.name,
      Date.now(),
      parsed.data.waId
    );
  }
  emitConversation(conv.id);
  res.status(201).json(serializeConversation(conversationById(conv.id)!));
});

/** Archivar, fijar, silenciar, marcar leído. */
conversationsRouter.patch('/conversations/:id', (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });

  const parsed = z
    .object({
      isArchived: z.boolean().optional(),
      isPinned: z.boolean().optional(),
      isMuted: z.boolean().optional(),
      markRead: z.boolean().optional(),
    })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Datos inválidos' });
  const p = parsed.data;

  db.prepare(
    `UPDATE conversations SET
       is_archived  = COALESCE(@isArchived, is_archived),
       is_pinned    = COALESCE(@isPinned, is_pinned),
       is_muted     = COALESCE(@isMuted, is_muted),
       unread_count = CASE WHEN @markRead = 1 THEN 0 ELSE unread_count END
     WHERE id = @id`
  ).run({
    id: conv.id,
    isArchived: p.isArchived === undefined ? null : p.isArchived ? 1 : 0,
    isPinned: p.isPinned === undefined ? null : p.isPinned ? 1 : 0,
    isMuted: p.isMuted === undefined ? null : p.isMuted ? 1 : 0,
    markRead: p.markRead ? 1 : 0,
  });

  // Confirmación de lectura real en WhatsApp para el último entrante.
  if (p.markRead) {
    const last = db
      .prepare(
        `SELECT wam_id FROM messages
          WHERE conversation_id = ? AND direction = 'in' AND wam_id IS NOT NULL
          ORDER BY timestamp DESC LIMIT 1`
      )
      .get(conv.id) as { wam_id: string } | undefined;
    if (last?.wam_id) {
      wa.markAsRead(last.wam_id).catch((e) => console.warn('[markAsRead]', e.message));
    }
  }

  const updated = conversationById(conv.id)!;
  broadcast({ type: 'conversation:update', payload: serializeConversation(updated) });
  res.json(serializeConversation(updated));
});

conversationsRouter.delete('/conversations/:id', (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });
  db.prepare('DELETE FROM conversations WHERE id = ?').run(conv.id);
  res.json({ ok: true });
});

// ─── Mensajes ───────────────────────────────────────────────────────────────

conversationsRouter.get('/conversations/:id/messages', (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });

  const limit = Math.min(Number(req.query.limit ?? 60), 200);
  const before = req.query.before ? Number(req.query.before) : null;

  const rows = db
    .prepare(
      `SELECT * FROM messages
        WHERE conversation_id = @id ${before ? 'AND id < @before' : ''}
        ORDER BY timestamp DESC, id DESC LIMIT @limit`
    )
    .all({ id: conv.id, before, limit }) as MessageRow[];

  res.json({
    messages: rows.reverse().map(serializeMessage),
    hasMore: rows.length === limit,
  });
});

function sendResponse(res: import('express').Response, result: Awaited<ReturnType<typeof sender.sendText>>) {
  if (result.error) {
    return res.status(result.error.isWindowClosed ? 409 : 502).json({
      error: result.error.message,
      code: result.error.code,
      windowClosed: result.error.isWindowClosed,
      message: result.message,
    });
  }
  return res.status(201).json({ message: result.message });
}

/** Envío de texto. */
conversationsRouter.post('/conversations/:id/messages', async (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });

  const parsed = z
    .object({ text: z.string().trim().min(1).max(4096), replyTo: z.string().optional() })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'El texto es obligatorio (máx. 4096 caracteres)' });

  const result = await sender.sendText(conv.wa_id, parsed.data.text, parsed.data.replyTo);
  sendResponse(res, result);
});

/** Envío de archivo: se sube a Meta y se manda referenciando el media id. */
conversationsRouter.post('/conversations/:id/media', upload.single('file'), async (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });
  if (!req.file) return res.status(400).json({ error: 'Falta el archivo' });

  const caption = typeof req.body.caption === 'string' ? req.body.caption.slice(0, 1024) : undefined;
  const replyTo = typeof req.body.replyTo === 'string' ? req.body.replyTo : undefined;
  const mimeType = req.file.mimetype || 'application/octet-stream';
  const kindOverride = typeof req.body.kind === 'string' ? req.body.kind : null;
  const kind = (kindOverride as wa.MediaKind) || kindForMime(mimeType);
  const filename = safeFilename(req.file.originalname, 'archivo');

  try {
    const mediaId = await wa.uploadMedia(req.file.buffer, mimeType, filename);
    const stored = await storeOutboundMedia(req.file.buffer, mimeType, filename);
    const result = await sender.sendMedia(
      conv.wa_id,
      kind,
      mediaId,
      {
        caption,
        filename,
        localPath: stored.relativePath,
        mime: mimeType,
        size: stored.size,
      },
      replyTo
    );
    sendResponse(res, result);
  } catch (err) {
    const e = err instanceof WhatsAppError ? err : new WhatsAppError((err as Error).message, 500);
    res.status(e.status >= 400 && e.status < 600 ? e.status : 502).json({ error: e.message, code: e.code });
  }
});

/** Envío de plantilla (único camino válido fuera de la ventana de 24 h). */
conversationsRouter.post('/conversations/:id/template', async (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });

  const parsed = z
    .object({
      name: z.string().min(1),
      language: z.string().min(2),
      components: z.array(z.any()).optional(),
    })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Plantilla inválida' });

  const result = await sender.sendTemplate(
    conv.wa_id,
    parsed.data.name,
    parsed.data.language,
    parsed.data.components
  );
  sendResponse(res, result);
});

/** Envío de ubicación. */
conversationsRouter.post('/conversations/:id/location', async (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });

  const parsed = z
    .object({
      latitude: z.number().min(-90).max(90),
      longitude: z.number().min(-180).max(180),
      name: z.string().optional(),
      address: z.string().optional(),
    })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Coordenadas inválidas' });

  const result = await sender.sendLocation(conv.wa_id, parsed.data);
  sendResponse(res, result);
});

/** Reacción a un mensaje (emoji vacío = quitar). */
conversationsRouter.post('/messages/:wamId/reaction', async (req, res) => {
  const row = db.prepare('SELECT * FROM messages WHERE wam_id = ?').get(req.params.wamId) as
    | MessageRow
    | undefined;
  if (!row) return res.status(404).json({ error: 'Mensaje no encontrado' });

  const emoji = typeof req.body?.emoji === 'string' ? req.body.emoji : '';
  try {
    await sender.sendReaction(row.wa_id, row.wam_id!, emoji);
    res.json({ ok: true });
  } catch (err) {
    const e = err instanceof WhatsAppError ? err : new WhatsAppError((err as Error).message, 500);
    res.status(502).json({ error: e.message, code: e.code });
  }
});

/** Sirve un adjunto ya descargado desde el disco local. */
conversationsRouter.get('/media/:messageId', (req, res) => {
  const row = db.prepare('SELECT * FROM messages WHERE id = ?').get(Number(req.params.messageId)) as
    | MessageRow
    | undefined;
  if (!row?.media_path) return res.status(404).json({ error: 'Archivo no disponible' });

  let abs: string;
  try {
    abs = mediaAbsolutePath(row.media_path);
  } catch {
    return res.status(400).json({ error: 'Ruta inválida' });
  }
  if (!fs.existsSync(abs)) return res.status(404).json({ error: 'Archivo no encontrado en disco' });

  if (row.media_mime) res.type(row.media_mime);
  if (row.media_filename && String(req.query.download) === '1') {
    res.setHeader('Content-Disposition', `attachment; filename="${encodeURIComponent(row.media_filename)}"`);
  }
  res.sendFile(abs);
});
