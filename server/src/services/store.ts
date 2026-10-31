import { db } from '../db/index.js';
import { broadcast } from '../realtime/hub.js';

export interface MessageRow {
  id: number;
  wam_id: string | null;
  conversation_id: number;
  wa_id: string;
  direction: 'in' | 'out';
  type: string;
  body: string | null;
  status: string;
  error_code: string | null;
  error_message: string | null;
  timestamp: number;
  context_wam_id: string | null;
  reaction_emoji: string | null;
  reaction_target: string | null;
  media_id: string | null;
  media_path: string | null;
  media_mime: string | null;
  media_filename: string | null;
  media_size: number | null;
  media_state: string | null;
  payload: string | null;
  created_at: number;
}

export interface ConversationRow {
  id: number;
  wa_id: string;
  last_message_at: number | null;
  last_inbound_at: number | null;
  unread_count: number;
  is_archived: number;
  is_pinned: number;
  is_muted: number;
  created_at: number;
}

export interface ContactRow {
  wa_id: string;
  profile_name: string | null;
  display_name: string | null;
  notes: string | null;
  is_blocked: number;
  created_at: number;
  updated_at: number;
}

/** 24 h en milisegundos: duración de la ventana de servicio de Meta. */
export const SERVICE_WINDOW_MS = 24 * 60 * 60 * 1000;

export function upsertContact(waId: string, profileName?: string | null): ContactRow {
  const now = Date.now();
  db.prepare(
    `INSERT INTO contacts (wa_id, profile_name, created_at, updated_at)
     VALUES (@wa_id, @profile_name, @now, @now)
     ON CONFLICT(wa_id) DO UPDATE SET
       profile_name = COALESCE(NULLIF(excluded.profile_name, ''), contacts.profile_name),
       updated_at   = @now`
  ).run({ wa_id: waId, profile_name: profileName ?? null, now });
  return db.prepare('SELECT * FROM contacts WHERE wa_id = ?').get(waId) as ContactRow;
}

export function getOrCreateConversation(waId: string, profileName?: string | null): ConversationRow {
  upsertContact(waId, profileName);
  const existing = db.prepare('SELECT * FROM conversations WHERE wa_id = ?').get(waId) as
    | ConversationRow
    | undefined;
  if (existing) return existing;
  db.prepare('INSERT INTO conversations (wa_id, created_at) VALUES (?, ?)').run(waId, Date.now());
  return db.prepare('SELECT * FROM conversations WHERE wa_id = ?').get(waId) as ConversationRow;
}

export interface InsertMessageInput {
  wamId?: string | null;
  conversationId: number;
  waId: string;
  direction: 'in' | 'out';
  type: string;
  body?: string | null;
  status?: string;
  timestamp?: number;
  contextWamId?: string | null;
  reactionEmoji?: string | null;
  reactionTarget?: string | null;
  mediaId?: string | null;
  mediaPath?: string | null;
  mediaMime?: string | null;
  mediaFilename?: string | null;
  mediaSize?: number | null;
  mediaState?: string | null;
  payload?: unknown;
}

export function insertMessage(input: InsertMessageInput): MessageRow {
  const now = Date.now();
  const info = db
    .prepare(
      `INSERT INTO messages (
         wam_id, conversation_id, wa_id, direction, type, body, status, timestamp,
         context_wam_id, reaction_emoji, reaction_target,
         media_id, media_path, media_mime, media_filename, media_size, media_state,
         payload, created_at
       ) VALUES (
         @wamId, @conversationId, @waId, @direction, @type, @body, @status, @timestamp,
         @contextWamId, @reactionEmoji, @reactionTarget,
         @mediaId, @mediaPath, @mediaMime, @mediaFilename, @mediaSize, @mediaState,
         @payload, @createdAt
       )`
    )
    .run({
      wamId: input.wamId ?? null,
      conversationId: input.conversationId,
      waId: input.waId,
      direction: input.direction,
      type: input.type,
      body: input.body ?? null,
      status: input.status ?? (input.direction === 'in' ? 'received' : 'pending'),
      timestamp: input.timestamp ?? now,
      contextWamId: input.contextWamId ?? null,
      reactionEmoji: input.reactionEmoji ?? null,
      reactionTarget: input.reactionTarget ?? null,
      mediaId: input.mediaId ?? null,
      mediaPath: input.mediaPath ?? null,
      mediaMime: input.mediaMime ?? null,
      mediaFilename: input.mediaFilename ?? null,
      mediaSize: input.mediaSize ?? null,
      mediaState: input.mediaState ?? null,
      payload: input.payload === undefined ? null : JSON.stringify(input.payload),
      createdAt: now,
    });

  return db.prepare('SELECT * FROM messages WHERE id = ?').get(Number(info.lastInsertRowid)) as MessageRow;
}

export function touchConversation(
  conversationId: number,
  opts: { messageAt: number; inbound: boolean; incrementUnread?: boolean }
): void {
  db.prepare(
    `UPDATE conversations SET
       last_message_at = MAX(COALESCE(last_message_at, 0), @messageAt),
       last_inbound_at = CASE WHEN @inbound = 1
                              THEN MAX(COALESCE(last_inbound_at, 0), @messageAt)
                              ELSE last_inbound_at END,
       unread_count    = CASE WHEN @bump = 1 THEN unread_count + 1 ELSE unread_count END,
       is_archived     = CASE WHEN @inbound = 1 THEN 0 ELSE is_archived END
     WHERE id = @id`
  ).run({
    id: conversationId,
    messageAt: opts.messageAt,
    inbound: opts.inbound ? 1 : 0,
    bump: opts.incrementUnread ? 1 : 0,
  });
}

const STATUS_RANK: Record<string, number> = {
  pending: 0,
  sent: 1,
  received: 1,
  delivered: 2,
  read: 3,
  failed: 4,
};

/** Las actualizaciones de estado de Meta pueden llegar desordenadas: nunca retrocedemos. */
export function updateMessageStatus(
  wamId: string,
  status: string,
  error?: { code?: number | string; message?: string }
): MessageRow | null {
  const row = db.prepare('SELECT * FROM messages WHERE wam_id = ?').get(wamId) as MessageRow | undefined;
  if (!row) return null;

  const current = STATUS_RANK[row.status] ?? 0;
  const next = STATUS_RANK[status] ?? 0;
  if (next < current && status !== 'failed') return row;

  db.prepare('UPDATE messages SET status = ?, error_code = ?, error_message = ? WHERE id = ?').run(
    status,
    error?.code !== undefined ? String(error.code) : row.error_code,
    error?.message ?? row.error_message,
    row.id
  );
  const updated = db.prepare('SELECT * FROM messages WHERE id = ?').get(row.id) as MessageRow;
  broadcast({ type: 'message:update', payload: serializeMessage(updated) });
  return updated;
}

export function setMessageMedia(
  id: number,
  media: { path?: string | null; mime?: string | null; size?: number | null; state: string }
): MessageRow {
  db.prepare(
    `UPDATE messages SET media_path = COALESCE(?, media_path),
                         media_mime = COALESCE(?, media_mime),
                         media_size = COALESCE(?, media_size),
                         media_state = ?
     WHERE id = ?`
  ).run(media.path ?? null, media.mime ?? null, media.size ?? null, media.state, id);
  const row = db.prepare('SELECT * FROM messages WHERE id = ?').get(id) as MessageRow;
  broadcast({ type: 'message:update', payload: serializeMessage(row) });
  return row;
}

export function serializeMessage(row: MessageRow) {
  return {
    id: row.id,
    wamId: row.wam_id,
    conversationId: row.conversation_id,
    waId: row.wa_id,
    direction: row.direction,
    type: row.type,
    body: row.body,
    status: row.status,
    error: row.error_message ? { code: row.error_code, message: row.error_message } : null,
    timestamp: row.timestamp,
    replyTo: row.context_wam_id,
    reaction: row.reaction_emoji,
    reactionTarget: row.reaction_target,
    media: row.media_id || row.media_path
      ? {
          id: row.media_id,
          state: row.media_state ?? 'ready',
          mime: row.media_mime,
          filename: row.media_filename,
          size: row.media_size,
          url: row.media_path ? `/api/media/${row.id}` : null,
        }
      : null,
    payload: row.payload ? safeParse(row.payload) : null,
  };
}

function safeParse(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

export function contactLabels(waId: string) {
  return db
    .prepare(
      `SELECT l.id, l.name, l.color
         FROM labels l JOIN contact_labels cl ON cl.label_id = l.id
        WHERE cl.wa_id = ? ORDER BY l.name`
    )
    .all(waId) as Array<{ id: number; name: string; color: string }>;
}

export function serializeConversation(conv: ConversationRow) {
  const contact = db.prepare('SELECT * FROM contacts WHERE wa_id = ?').get(conv.wa_id) as
    | ContactRow
    | undefined;
  const last = db
    .prepare('SELECT * FROM messages WHERE conversation_id = ? ORDER BY timestamp DESC, id DESC LIMIT 1')
    .get(conv.id) as MessageRow | undefined;

  const windowClosesAt = conv.last_inbound_at ? conv.last_inbound_at + SERVICE_WINDOW_MS : null;

  return {
    id: conv.id,
    waId: conv.wa_id,
    name: contact?.display_name || contact?.profile_name || `+${conv.wa_id}`,
    profileName: contact?.profile_name ?? null,
    displayName: contact?.display_name ?? null,
    notes: contact?.notes ?? null,
    isBlocked: Boolean(contact?.is_blocked),
    labels: contactLabels(conv.wa_id),
    lastMessageAt: conv.last_message_at,
    lastInboundAt: conv.last_inbound_at,
    unreadCount: conv.unread_count,
    isArchived: Boolean(conv.is_archived),
    isPinned: Boolean(conv.is_pinned),
    isMuted: Boolean(conv.is_muted),
    // La ventana de servicio de 24 h: fuera de ella solo se pueden enviar plantillas.
    windowOpen: windowClosesAt !== null && windowClosesAt > Date.now(),
    windowClosesAt,
    lastMessage: last ? serializeMessage(last) : null,
  };
}

export function conversationById(id: number): ConversationRow | undefined {
  return db.prepare('SELECT * FROM conversations WHERE id = ?').get(id) as ConversationRow | undefined;
}

export function emitConversation(conversationId: number): void {
  const conv = conversationById(conversationId);
  if (conv) broadcast({ type: 'conversation:update', payload: serializeConversation(conv) });
}
