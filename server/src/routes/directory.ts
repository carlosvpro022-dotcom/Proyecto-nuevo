import { Router } from 'express';
import { z } from 'zod';
import { db } from '../db/index.js';
import { broadcast } from '../realtime/hub.js';
import {
  contactLabels,
  getOrCreateConversation,
  serializeConversation,
  conversationById,
  type ContactRow,
} from '../services/store.js';
import * as sender from '../services/sender.js';

export const directoryRouter = Router();

// ─── Contactos ──────────────────────────────────────────────────────────────

function serializeContact(c: ContactRow) {
  const conv = db.prepare('SELECT id FROM conversations WHERE wa_id = ?').get(c.wa_id) as
    | { id: number }
    | undefined;
  return {
    waId: c.wa_id,
    name: c.display_name || c.profile_name || `+${c.wa_id}`,
    displayName: c.display_name,
    profileName: c.profile_name,
    notes: c.notes,
    isBlocked: Boolean(c.is_blocked),
    labels: contactLabels(c.wa_id),
    conversationId: conv?.id ?? null,
  };
}

directoryRouter.get('/contacts', (req, res) => {
  const q = String(req.query.q ?? '').trim().toLowerCase();
  const labelId = req.query.labelId ? Number(req.query.labelId) : null;

  let sql = 'SELECT c.* FROM contacts c';
  const where: string[] = [];
  const params: Record<string, unknown> = {};

  if (labelId) {
    sql += ' JOIN contact_labels cl ON cl.wa_id = c.wa_id';
    where.push('cl.label_id = @labelId');
    params.labelId = labelId;
  }
  if (q) {
    where.push(
      `(LOWER(COALESCE(c.display_name,'')) LIKE @q OR LOWER(COALESCE(c.profile_name,'')) LIKE @q OR c.wa_id LIKE @q)`
    );
    params.q = `%${q}%`;
  }
  if (where.length) sql += ' WHERE ' + where.join(' AND ');
  sql += ' ORDER BY COALESCE(c.display_name, c.profile_name, c.wa_id) COLLATE NOCASE LIMIT 500';

  const rows = db.prepare(sql).all(params) as ContactRow[];
  res.json({ contacts: rows.map(serializeContact) });
});

directoryRouter.patch('/contacts/:waId', (req, res) => {
  const contact = db.prepare('SELECT * FROM contacts WHERE wa_id = ?').get(req.params.waId) as
    | ContactRow
    | undefined;
  if (!contact) return res.status(404).json({ error: 'Contacto no encontrado' });

  const parsed = z
    .object({
      displayName: z.string().trim().max(120).nullable().optional(),
      notes: z.string().max(4000).nullable().optional(),
      isBlocked: z.boolean().optional(),
    })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Datos inválidos' });
  const p = parsed.data;

  db.prepare(
    `UPDATE contacts SET
       display_name = CASE WHEN @hasName = 1 THEN @displayName ELSE display_name END,
       notes        = CASE WHEN @hasNotes = 1 THEN @notes ELSE notes END,
       is_blocked   = COALESCE(@isBlocked, is_blocked),
       updated_at   = @now
     WHERE wa_id = @waId`
  ).run({
    waId: contact.wa_id,
    hasName: p.displayName === undefined ? 0 : 1,
    displayName: p.displayName ?? null,
    hasNotes: p.notes === undefined ? 0 : 1,
    notes: p.notes ?? null,
    isBlocked: p.isBlocked === undefined ? null : p.isBlocked ? 1 : 0,
    now: Date.now(),
  });

  const updated = db.prepare('SELECT * FROM contacts WHERE wa_id = ?').get(contact.wa_id) as ContactRow;
  broadcast({ type: 'contact:update', payload: serializeContact(updated) });

  const conv = db.prepare('SELECT id FROM conversations WHERE wa_id = ?').get(contact.wa_id) as
    | { id: number }
    | undefined;
  if (conv) broadcast({ type: 'conversation:update', payload: serializeConversation(conversationById(conv.id)!) });

  res.json(serializeContact(updated));
});

// ─── Etiquetas ──────────────────────────────────────────────────────────────

directoryRouter.get('/labels', (_req, res) => {
  const labels = db
    .prepare(
      `SELECT l.*, (SELECT COUNT(*) FROM contact_labels cl WHERE cl.label_id = l.id) AS contact_count
         FROM labels l ORDER BY l.name COLLATE NOCASE`
    )
    .all();
  res.json({ labels });
});

directoryRouter.post('/labels', (req, res) => {
  const parsed = z
    .object({
      name: z.string().trim().min(1).max(40),
      color: z.string().regex(/^#[0-9a-fA-F]{6}$/).default('#25d366'),
    })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Nombre o color inválido' });

  try {
    const info = db
      .prepare('INSERT INTO labels (name, color, created_at) VALUES (?, ?, ?)')
      .run(parsed.data.name, parsed.data.color, Date.now());
    res.status(201).json(db.prepare('SELECT * FROM labels WHERE id = ?').get(Number(info.lastInsertRowid)));
  } catch {
    res.status(409).json({ error: 'Ya existe una etiqueta con ese nombre' });
  }
});

directoryRouter.patch('/labels/:id', (req, res) => {
  const parsed = z
    .object({
      name: z.string().trim().min(1).max(40).optional(),
      color: z.string().regex(/^#[0-9a-fA-F]{6}$/).optional(),
    })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Datos inválidos' });

  db.prepare('UPDATE labels SET name = COALESCE(?, name), color = COALESCE(?, color) WHERE id = ?').run(
    parsed.data.name ?? null,
    parsed.data.color ?? null,
    Number(req.params.id)
  );
  res.json(db.prepare('SELECT * FROM labels WHERE id = ?').get(Number(req.params.id)));
});

directoryRouter.delete('/labels/:id', (req, res) => {
  db.prepare('DELETE FROM labels WHERE id = ?').run(Number(req.params.id));
  res.json({ ok: true });
});

directoryRouter.put('/contacts/:waId/labels', (req, res) => {
  const parsed = z.object({ labelIds: z.array(z.number().int()) }).safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'labelIds debe ser un array de ids' });

  const waId = req.params.waId;
  const exists = db.prepare('SELECT 1 FROM contacts WHERE wa_id = ?').get(waId);
  if (!exists) return res.status(404).json({ error: 'Contacto no encontrado' });

  const tx = db.transaction(() => {
    db.prepare('DELETE FROM contact_labels WHERE wa_id = ?').run(waId);
    const ins = db.prepare('INSERT OR IGNORE INTO contact_labels (wa_id, label_id) VALUES (?, ?)');
    for (const id of parsed.data.labelIds) ins.run(waId, id);
  });
  tx();

  const conv = db.prepare('SELECT id FROM conversations WHERE wa_id = ?').get(waId) as
    | { id: number }
    | undefined;
  if (conv) broadcast({ type: 'conversation:update', payload: serializeConversation(conversationById(conv.id)!) });

  res.json({ labels: contactLabels(waId) });
});

// ─── Listas de difusión (sustituto de los grupos) ───────────────────────────
// La Cloud API no expone grupos de WhatsApp. Estas listas envían el mismo
// mensaje a cada miembro como conversación individual.

directoryRouter.get('/broadcasts', (_req, res) => {
  const lists = db.prepare('SELECT * FROM broadcast_lists ORDER BY name COLLATE NOCASE').all() as Array<{
    id: number;
    name: string;
    created_at: number;
  }>;
  const members = db.prepare('SELECT list_id, wa_id FROM broadcast_members').all() as Array<{
    list_id: number;
    wa_id: string;
  }>;
  res.json({
    lists: lists.map((l) => ({
      ...l,
      members: members.filter((m) => m.list_id === l.id).map((m) => m.wa_id),
    })),
  });
});

directoryRouter.post('/broadcasts', (req, res) => {
  const parsed = z
    .object({ name: z.string().trim().min(1).max(60), members: z.array(z.string()).default([]) })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Nombre inválido' });

  try {
    const info = db
      .prepare('INSERT INTO broadcast_lists (name, created_at) VALUES (?, ?)')
      .run(parsed.data.name, Date.now());
    const listId = Number(info.lastInsertRowid);
    const ins = db.prepare('INSERT OR IGNORE INTO broadcast_members (list_id, wa_id) VALUES (?, ?)');
    for (const waId of parsed.data.members) {
      getOrCreateConversation(waId);
      ins.run(listId, waId);
    }
    res.status(201).json({ id: listId, name: parsed.data.name, members: parsed.data.members });
  } catch {
    res.status(409).json({ error: 'Ya existe una lista con ese nombre' });
  }
});

directoryRouter.put('/broadcasts/:id/members', (req, res) => {
  const parsed = z.object({ members: z.array(z.string()) }).safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'members debe ser un array' });

  const listId = Number(req.params.id);
  const tx = db.transaction(() => {
    db.prepare('DELETE FROM broadcast_members WHERE list_id = ?').run(listId);
    const ins = db.prepare('INSERT OR IGNORE INTO broadcast_members (list_id, wa_id) VALUES (?, ?)');
    for (const waId of parsed.data.members) {
      getOrCreateConversation(waId);
      ins.run(listId, waId);
    }
  });
  tx();
  res.json({ ok: true, members: parsed.data.members });
});

directoryRouter.delete('/broadcasts/:id', (req, res) => {
  db.prepare('DELETE FROM broadcast_lists WHERE id = ?').run(Number(req.params.id));
  res.json({ ok: true });
});

/**
 * Envía a todos los miembros de la lista. Usa plantilla si se indica
 * (obligatorio fuera de la ventana de 24 h); si no, texto libre.
 */
directoryRouter.post('/broadcasts/:id/send', async (req, res) => {
  const listId = Number(req.params.id);
  const list = db.prepare('SELECT * FROM broadcast_lists WHERE id = ?').get(listId);
  if (!list) return res.status(404).json({ error: 'Lista no encontrada' });

  const parsed = z
    .object({
      text: z.string().trim().max(4096).optional(),
      template: z.object({ name: z.string(), language: z.string(), components: z.array(z.any()).optional() }).optional(),
    })
    .refine((v) => Boolean(v.text) || Boolean(v.template), 'Indica texto o plantilla')
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Indica un texto o una plantilla' });

  const members = db.prepare('SELECT wa_id FROM broadcast_members WHERE list_id = ?').all(listId) as Array<{
    wa_id: string;
  }>;

  const results: Array<{ waId: string; ok: boolean; error?: string }> = [];
  for (const { wa_id } of members) {
    const out = parsed.data.template
      ? await sender.sendTemplate(
          wa_id,
          parsed.data.template.name,
          parsed.data.template.language,
          parsed.data.template.components
        )
      : await sender.sendText(wa_id, parsed.data.text!);
    results.push({ waId: wa_id, ok: !out.error, error: out.error?.message });
    // Pausa breve: la Cloud API limita la tasa de envío por número.
    await new Promise((r) => setTimeout(r, 120));
  }

  res.json({
    sent: results.filter((r) => r.ok).length,
    failed: results.filter((r) => !r.ok).length,
    results,
  });
});
