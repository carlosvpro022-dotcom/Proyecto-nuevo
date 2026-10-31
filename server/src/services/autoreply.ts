import { db, getSetting, setSetting } from '../db/index.js';

export interface AutoReplyRow {
  id: number;
  name: string;
  kind: 'welcome' | 'away' | 'keyword';
  enabled: number;
  keyword: string | null;
  match_type: 'contains' | 'equals' | 'starts_with';
  reply_text: string;
  active_from: number | null;
  active_to: number | null;
  priority: number;
  created_at: number;
}

/** Enfriamiento por regla y contacto, para no spamear ni entrar en bucles. */
const COOLDOWN_MS: Record<AutoReplyRow['kind'], number> = {
  welcome: Number.POSITIVE_INFINITY, // una sola vez por contacto
  away: 60 * 60 * 1000, // una vez por hora
  keyword: 5 * 60 * 1000, // una vez cada 5 min por regla
};

function minutesOfDay(d: Date): number {
  return d.getHours() * 60 + d.getMinutes();
}

function withinSchedule(rule: AutoReplyRow, now: Date): boolean {
  if (rule.active_from === null || rule.active_to === null) return true;
  const m = minutesOfDay(now);
  // Rango que cruza medianoche (p. ej. 20:00 -> 09:00).
  return rule.active_from <= rule.active_to
    ? m >= rule.active_from && m < rule.active_to
    : m >= rule.active_from || m < rule.active_to;
}

function keywordMatches(rule: AutoReplyRow, text: string): boolean {
  if (!rule.keyword) return false;
  const haystack = text.toLowerCase().trim();
  const needle = rule.keyword.toLowerCase().trim();
  if (!needle) return false;
  if (rule.match_type === 'equals') return haystack === needle;
  if (rule.match_type === 'starts_with') return haystack.startsWith(needle);
  return haystack.includes(needle);
}

function cooldownKey(rule: AutoReplyRow, waId: string): string {
  return `autoreply:${rule.kind}:${rule.id}:${waId}`;
}

function onCooldown(rule: AutoReplyRow, waId: string, now: number): boolean {
  const last = getSetting(cooldownKey(rule, waId));
  if (!last) return false;
  const elapsed = now - Number(last);
  return elapsed < COOLDOWN_MS[rule.kind];
}

function markFired(rule: AutoReplyRow, waId: string, now: number): void {
  setSetting(cooldownKey(rule, waId), String(now));
}

export function listRules(): AutoReplyRow[] {
  return db
    .prepare('SELECT * FROM auto_replies ORDER BY priority DESC, id ASC')
    .all() as AutoReplyRow[];
}

/** ¿Es el primer mensaje entrante de este contacto? */
function isFirstInbound(conversationId: number): boolean {
  const row = db
    .prepare(`SELECT COUNT(*) AS n FROM messages WHERE conversation_id = ? AND direction = 'in'`)
    .get(conversationId) as { n: number };
  return row.n <= 1; // el mensaje recién insertado ya cuenta
}

/**
 * Elige la respuesta automática aplicable a un mensaje entrante, o null.
 * Orden: palabra clave > bienvenida > ausencia.
 */
export function pickAutoReply(params: {
  conversationId: number;
  waId: string;
  text: string | null;
  now?: Date;
}): { rule: AutoReplyRow; text: string } | null {
  const now = params.now ?? new Date();
  const nowMs = now.getTime();
  const rules = listRules().filter((r) => r.enabled === 1);

  const order: AutoReplyRow['kind'][] = ['keyword', 'welcome', 'away'];
  for (const kind of order) {
    for (const rule of rules.filter((r) => r.kind === kind)) {
      if (kind === 'keyword') {
        if (!params.text || !keywordMatches(rule, params.text)) continue;
      } else if (kind === 'welcome') {
        if (!isFirstInbound(params.conversationId)) continue;
      } else if (kind === 'away') {
        if (!withinSchedule(rule, now)) continue;
      }
      if (onCooldown(rule, params.waId, nowMs)) continue;
      markFired(rule, params.waId, nowMs);
      return { rule, text: rule.reply_text };
    }
  }
  return null;
}
