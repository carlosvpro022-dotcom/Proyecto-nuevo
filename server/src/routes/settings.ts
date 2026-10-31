import { Router } from 'express';
import { z } from 'zod';
import { db, getSetting, setSetting } from '../db/index.js';
import { env, whatsappConfigured, configWarnings } from '../env.js';
import { connectedClients } from '../realtime/hub.js';
import { webhookStatus } from '../whatsapp/webhook.js';
import type { AutoReplyRow } from '../services/autoreply.js';

export const settingsRouter = Router();

const SETTING_KEYS = ['autoMarkRead', 'notificationsEnabled', 'soundEnabled', 'defaultCatalogId'] as const;

settingsRouter.get('/settings', (_req, res) => {
  const values: Record<string, string | null> = {};
  for (const key of SETTING_KEYS) values[key] = getSetting(key);

  res.json({
    settings: values,
    whatsapp: {
      ...webhookStatus(),
      configured: whatsappConfigured,
      phoneNumberId: env.phoneNumberId ? `…${env.phoneNumberId.slice(-4)}` : null,
      wabaId: env.wabaId ? `…${env.wabaId.slice(-4)}` : null,
      graphVersion: env.graphVersion,
    },
    warnings: configWarnings(),
    realtimeClients: connectedClients(),
    // Limitaciones reales de la API oficial, para mostrarlas en la UI.
    limitations: [
      'La Cloud API no soporta grupos de WhatsApp: usa listas de difusión.',
      'Fuera de la ventana de 24 h solo se pueden enviar plantillas aprobadas.',
      'El historial anterior a la conexión del webhook no se puede recuperar.',
    ],
  });
});

settingsRouter.put('/settings', (req, res) => {
  const parsed = z.record(z.string(), z.string()).safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Formato inválido' });

  for (const [key, value] of Object.entries(parsed.data)) {
    if ((SETTING_KEYS as readonly string[]).includes(key)) setSetting(key, value);
  }
  const values: Record<string, string | null> = {};
  for (const key of SETTING_KEYS) values[key] = getSetting(key);
  res.json({ settings: values });
});

// ─── Respuestas automáticas ─────────────────────────────────────────────────

settingsRouter.get('/auto-replies', (_req, res) => {
  const rules = db.prepare('SELECT * FROM auto_replies ORDER BY priority DESC, id ASC').all() as AutoReplyRow[];
  res.json({ rules });
});

const ruleSchema = z.object({
  name: z.string().trim().min(1).max(80),
  kind: z.enum(['welcome', 'away', 'keyword']),
  enabled: z.boolean().default(true),
  keyword: z.string().trim().max(120).nullable().optional(),
  matchType: z.enum(['contains', 'equals', 'starts_with']).default('contains'),
  replyText: z.string().trim().min(1).max(4096),
  activeFrom: z.number().int().min(0).max(1439).nullable().optional(),
  activeTo: z.number().int().min(0).max(1439).nullable().optional(),
  priority: z.number().int().min(0).max(100).default(0),
});

settingsRouter.post('/auto-replies', (req, res) => {
  const parsed = ruleSchema.safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.issues[0].message });
  const p = parsed.data;

  if (p.kind === 'keyword' && !p.keyword) {
    return res.status(400).json({ error: 'Las reglas por palabra clave necesitan una palabra' });
  }

  const info = db
    .prepare(
      `INSERT INTO auto_replies (name, kind, enabled, keyword, match_type, reply_text, active_from, active_to, priority, created_at)
       VALUES (@name, @kind, @enabled, @keyword, @matchType, @replyText, @activeFrom, @activeTo, @priority, @now)`
    )
    .run({
      name: p.name,
      kind: p.kind,
      enabled: p.enabled ? 1 : 0,
      keyword: p.keyword ?? null,
      matchType: p.matchType,
      replyText: p.replyText,
      activeFrom: p.activeFrom ?? null,
      activeTo: p.activeTo ?? null,
      priority: p.priority,
      now: Date.now(),
    });

  res.status(201).json(db.prepare('SELECT * FROM auto_replies WHERE id = ?').get(Number(info.lastInsertRowid)));
});

settingsRouter.patch('/auto-replies/:id', (req, res) => {
  const parsed = ruleSchema.partial().safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Datos inválidos' });
  const p = parsed.data;
  const id = Number(req.params.id);

  const existing = db.prepare('SELECT * FROM auto_replies WHERE id = ?').get(id);
  if (!existing) return res.status(404).json({ error: 'Regla no encontrada' });

  db.prepare(
    `UPDATE auto_replies SET
       name        = COALESCE(@name, name),
       kind        = COALESCE(@kind, kind),
       enabled     = COALESCE(@enabled, enabled),
       keyword     = CASE WHEN @hasKeyword = 1 THEN @keyword ELSE keyword END,
       match_type  = COALESCE(@matchType, match_type),
       reply_text  = COALESCE(@replyText, reply_text),
       active_from = CASE WHEN @hasFrom = 1 THEN @activeFrom ELSE active_from END,
       active_to   = CASE WHEN @hasTo = 1 THEN @activeTo ELSE active_to END,
       priority    = COALESCE(@priority, priority)
     WHERE id = @id`
  ).run({
    id,
    name: p.name ?? null,
    kind: p.kind ?? null,
    enabled: p.enabled === undefined ? null : p.enabled ? 1 : 0,
    hasKeyword: p.keyword === undefined ? 0 : 1,
    keyword: p.keyword ?? null,
    matchType: p.matchType ?? null,
    replyText: p.replyText ?? null,
    hasFrom: p.activeFrom === undefined ? 0 : 1,
    activeFrom: p.activeFrom ?? null,
    hasTo: p.activeTo === undefined ? 0 : 1,
    activeTo: p.activeTo ?? null,
    priority: p.priority ?? null,
  });

  res.json(db.prepare('SELECT * FROM auto_replies WHERE id = ?').get(id));
});

settingsRouter.delete('/auto-replies/:id', (req, res) => {
  db.prepare('DELETE FROM auto_replies WHERE id = ?').run(Number(req.params.id));
  res.json({ ok: true });
});

// ─── Métricas ───────────────────────────────────────────────────────────────

settingsRouter.get('/stats', (_req, res) => {
  const since = Date.now() - 7 * 24 * 60 * 60 * 1000;
  const row = (sql: string, params: unknown[] = []) => db.prepare(sql).get(...params) as Record<string, number>;

  res.json({
    conversations: row('SELECT COUNT(*) AS n FROM conversations').n,
    unread: row('SELECT COALESCE(SUM(unread_count),0) AS n FROM conversations').n,
    contacts: row('SELECT COUNT(*) AS n FROM contacts').n,
    messagesIn7d: row(`SELECT COUNT(*) AS n FROM messages WHERE direction='in' AND timestamp >= ?`, [since]).n,
    messagesOut7d: row(`SELECT COUNT(*) AS n FROM messages WHERE direction='out' AND timestamp >= ?`, [since]).n,
    failed7d: row(`SELECT COUNT(*) AS n FROM messages WHERE status='failed' AND timestamp >= ?`, [since]).n,
    openWindows: row(
      'SELECT COUNT(*) AS n FROM conversations WHERE last_inbound_at IS NOT NULL AND last_inbound_at > ?',
      [Date.now() - 24 * 60 * 60 * 1000]
    ).n,
    byLabel: db
      .prepare(
        `SELECT l.name, l.color, COUNT(cl.wa_id) AS n
           FROM labels l LEFT JOIN contact_labels cl ON cl.label_id = l.id
          GROUP BY l.id ORDER BY n DESC`
      )
      .all(),
  });
});
