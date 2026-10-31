/**
 * Rellena la base de datos con una conversación de ejemplo realista, para ver
 * cómo se comporta el panel antes de tener credenciales de Meta.
 *
 *   npm run seed:demo -w server
 *
 * Es destructivo: vacía las tablas de chats y mensajes antes de escribir.
 * No toca nada en WhatsApp, solo la base de datos local.
 */
import Database from 'better-sqlite3';
import path from 'node:path';
import fs from 'node:fs';
import 'dotenv/config';
import { writeDemoFiles } from './make-demo-files.mjs';

const root = process.cwd();
const dbPath = path.resolve(root, process.env.DATABASE_PATH ?? './data/app.db');
const mediaDir = path.resolve(root, process.env.MEDIA_DIR ?? './data/media');

if (!fs.existsSync(dbPath)) {
  console.error(`No existe la base de datos en ${dbPath}. Arranca el servidor una vez para crearla.`);
  process.exit(1);
}

const db = new Database(dbPath);
db.pragma('foreign_keys = ON');
const files = writeDemoFiles(mediaDir);

const MIN = 60_000, HOUR = 60 * MIN, DAY = 24 * HOUR;
const now = Date.now();

db.exec('DELETE FROM messages; DELETE FROM conversations; DELETE FROM contacts; DELETE FROM broadcast_lists;');

const insContact = db.prepare(
  `INSERT INTO contacts (wa_id, profile_name, display_name, notes, is_blocked, created_at, updated_at)
   VALUES (@waId, @profileName, @displayName, @notes, 0, @now, @now)`
);
const insConv = db.prepare(
  `INSERT INTO conversations (wa_id, last_message_at, last_inbound_at, unread_count, is_archived, is_pinned, is_muted, created_at)
   VALUES (@waId, @lastAt, @lastInbound, @unread, @archived, @pinned, 0, @now)`
);
const insMsg = db.prepare(
  `INSERT INTO messages (wam_id, conversation_id, wa_id, direction, type, body, status, timestamp,
                         context_wam_id, reaction_emoji, media_id, media_path, media_mime, media_filename,
                         media_size, media_state, payload, created_at)
   VALUES (@wamId, @convId, @waId, @direction, @type, @body, @status, @ts,
           @replyTo, @reaction, @mediaId, @mediaPath, @mediaMime, @mediaFilename,
           @mediaSize, @mediaState, @payload, @ts)`
);

let wam = 0;
const nextWam = () => `wamid.DEMO${String(++wam).padStart(4, '0')}`;

function addChat({ waId, profileName, displayName, notes, pinned = 0, archived = 0, unread = 0, labels = [], messages }) {
  insContact.run({ waId, profileName, displayName, notes, now });
  const inboundTimes = messages.filter((m) => m.dir === 'in').map((m) => m.ts);
  const lastAt = Math.max(...messages.map((m) => m.ts));
  insConv.run({
    waId,
    lastAt,
    lastInbound: inboundTimes.length ? Math.max(...inboundTimes) : null,
    unread,
    archived,
    pinned,
    now,
  });
  const convId = db.prepare('SELECT id FROM conversations WHERE wa_id = ?').get(waId).id;

  const ids = {};
  for (const m of messages) {
    const id = nextWam();
    if (m.tag) ids[m.tag] = id;
    const size = m.mediaPath ? fs.statSync(path.join(mediaDir, m.mediaPath)).size : null;
    insMsg.run({
      wamId: id,
      convId,
      waId,
      direction: m.dir,
      type: m.type ?? 'text',
      body: m.body ?? null,
      status: m.status ?? (m.dir === 'in' ? 'received' : 'read'),
      ts: m.ts,
      replyTo: m.replyToTag ? ids[m.replyToTag] : null,
      reaction: m.reaction ?? null,
      mediaId: m.mediaPath ? `MEDIA_DEMO_${wam}` : null,
      mediaPath: m.mediaPath ?? null,
      mediaMime: m.mediaMime ?? null,
      mediaFilename: m.mediaFilename ?? null,
      mediaSize: size,
      mediaState: m.mediaPath ? 'ready' : null,
      payload: m.payload ? JSON.stringify(m.payload) : null,
    });
  }

  const labelIds = db.prepare('SELECT id, name FROM labels').all();
  const link = db.prepare('INSERT OR IGNORE INTO contact_labels (wa_id, label_id) VALUES (?, ?)');
  for (const name of labels) {
    const l = labelIds.find((x) => x.name === name);
    if (l) link.run(waId, l.id);
  }
  return convId;
}

// ── Chat 1: solicitud de financiación en curso, con adjuntos y respuestas ──
addChat({
  waId: '34612345678',
  profileName: 'Laura Méndez',
  displayName: 'Laura Méndez',
  notes: 'Quiere financiar un Golf de ocasión. Pendiente de enviar el justificante de domicilio.',
  pinned: 1,
  labels: ['Nuevo cliente', 'Pedido pendiente'],
  messages: [
    { dir: 'in', ts: now - 2 * DAY - 3 * HOUR, body: 'Buenas tardes, vi vuestro anuncio. ¿Financiáis coches de segunda mano?' },
    { dir: 'out', ts: now - 2 * DAY - 2.8 * HOUR, body: '¡Hola Laura! Sí, financiamos vehículos de ocasión hasta 10 años de antigüedad. ¿Qué coche tienes en mente?' },
    { dir: 'in', ts: now - 2 * DAY - 2.5 * HOUR, type: 'image', body: 'Este de aquí, un Golf del 2019', mediaPath: files.coche, mediaMime: 'image/png', tag: 'foto' },
    { dir: 'out', ts: now - 2 * DAY - 2.2 * HOUR, body: 'Buena elección. ¿Qué importe necesitarías financiar y a cuántos meses?', replyToTag: 'foto' },
    { dir: 'in', ts: now - DAY - 5 * HOUR, body: 'Serían 14.500 €. A ser posible a 5 años para que la cuota no suba mucho.' },
    { dir: 'out', ts: now - DAY - 4.6 * HOUR, type: 'document', body: 'Te paso la propuesta con las condiciones. La cuota quedaría en 287,40 €/mes.', mediaPath: files.condiciones, mediaMime: 'application/pdf', mediaFilename: 'Condiciones de financiación.pdf', reaction: '👍' },
    { dir: 'in', ts: now - DAY - 4 * HOUR, body: 'Perfecto, me encaja. ¿Qué papeles necesitáis?' },
    { dir: 'out', ts: now - DAY - 3.8 * HOUR, body: 'DNI por ambas caras, las dos últimas nóminas y un justificante de domicilio.' },
    { dir: 'in', ts: now - 3 * HOUR, type: 'image', body: 'Aquí va la nómina de octubre', mediaPath: files.nomina, mediaMime: 'image/png' },
    { dir: 'out', ts: now - 2.8 * HOUR, body: 'Recibida, gracias. Me falta la de septiembre y el justificante de domicilio y ya lo mando a estudio.', status: 'read' },
    { dir: 'in', ts: now - 40 * MIN, body: 'Vale, esta tarde te las paso. ¿Cuánto suele tardar la respuesta?' },
    { dir: 'out', ts: now - 35 * MIN, body: 'En 24-48 h laborables tendrías la resolución.', status: 'delivered' },
  ],
});

// ── Chat 2: ventana de 24 h cerrada, requiere plantilla ──
addChat({
  waId: '34677889900',
  profileName: 'Carlos Ruiz',
  displayName: 'Carlos Ruiz',
  notes: 'Pidió presupuesto para reunificar deudas. No contesta desde hace dos días.',
  labels: ['Pedido pendiente'],
  messages: [
    { dir: 'in', ts: now - 3 * DAY, body: 'Hola, quería información para reunificar varios préstamos.' },
    { dir: 'out', ts: now - 3 * DAY + 20 * MIN, body: '¡Hola Carlos! Claro. ¿Cuántos préstamos son y qué importe total suman aproximadamente?' },
    { dir: 'in', ts: now - 2 * DAY - 6 * HOUR, body: 'Son tres, unos 22.000 € en total.' },
    { dir: 'out', ts: now - 2 * DAY - 5.5 * HOUR, body: 'Perfecto. Lo calculo y te digo algo mañana a primera hora.', status: 'read' },
  ],
});

// ── Chat 3: mensaje nuevo sin leer ──
addChat({
  waId: '34655443322',
  profileName: 'Ana Torres',
  displayName: null,
  notes: null,
  unread: 2,
  labels: ['Nuevo cliente'],
  messages: [
    { dir: 'in', ts: now - 18 * MIN, body: 'Buenos días, ¿seguís dando préstamos para autónomos?' },
    { dir: 'in', ts: now - 16 * MIN, body: 'Soy autónoma desde hace 3 años, por si sirve de algo' },
  ],
});

// ── Chat 4: envío fallido, para ver el estado de error ──
addChat({
  waId: '34699112233',
  profileName: 'Miguel Sanz',
  displayName: 'Miguel Sanz',
  notes: 'Número dado de baja. Confirmar teléfono nuevo.',
  labels: ['Incidencia'],
  messages: [
    { dir: 'in', ts: now - 5 * DAY, body: 'Ya he firmado el contrato, muchas gracias por todo' },
    { dir: 'out', ts: now - 5 * DAY + 10 * MIN, body: '¡Enhorabuena Miguel! Cualquier cosa nos escribes.', reaction: '🙏' },
    { dir: 'out', ts: now - 6 * HOUR, type: 'template', body: 'Plantilla: recordatorio_cuota', status: 'failed', payload: { template: 'recordatorio_cuota', language: 'es' } },
  ],
});

// ── Chat 5: cliente cerrado y archivado ──
addChat({
  waId: '34911223344',
  profileName: 'Comercial Vega S.L.',
  displayName: 'Comercial Vega S.L.',
  notes: 'Operación cerrada en septiembre. Renovación en marzo.',
  archived: 1,
  labels: ['Pago recibido', 'Cerrado'],
  messages: [
    { dir: 'in', ts: now - 20 * DAY, body: 'Confirmamos la transferencia de la primera cuota.' },
    { dir: 'out', ts: now - 20 * DAY + 30 * MIN, body: 'Recibida y aplicada. Os mandamos el justificante por correo.' },
  ],
});

// Marcar el estado de error del envío fallido con un motivo real de Meta.
db.prepare(
  `UPDATE messages SET error_code = '131047',
                       error_message = 'Message failed to send because more than 24 hours have passed since the customer last replied to this number'
   WHERE status = 'failed'`
).run();

// ── Respuestas automáticas de ejemplo ──
db.exec('DELETE FROM auto_replies');
const insRule = db.prepare(
  `INSERT INTO auto_replies (name, kind, enabled, keyword, match_type, reply_text, active_from, active_to, priority, created_at)
   VALUES (@name, @kind, @enabled, @keyword, @matchType, @reply, @from, @to, @priority, @now)`
);
insRule.run({ name: 'Consulta de cuota', kind: 'keyword', enabled: 1, keyword: 'cuota', matchType: 'contains',
  reply: 'Para calcularte la cuota necesito el importe y el plazo que te interesa. ¿Me los dices y te paso el número exacto?',
  from: null, to: null, priority: 20, now });
insRule.run({ name: 'Documentación', kind: 'keyword', enabled: 1, keyword: 'documentación', matchType: 'contains',
  reply: 'La documentación habitual es: DNI por ambas caras, las dos últimas nóminas y un justificante de domicilio.',
  from: null, to: null, priority: 15, now });
insRule.run({ name: 'Bienvenida', kind: 'welcome', enabled: 1, keyword: null, matchType: 'contains',
  reply: '¡Hola! Gracias por escribir a Torque Financiero. Cuéntanos qué necesitas y te atendemos enseguida.',
  from: null, to: null, priority: 10, now });
insRule.run({ name: 'Fuera de horario', kind: 'away', enabled: 1, keyword: null, matchType: 'contains',
  reply: 'Gracias por tu mensaje. Nuestro horario es de lunes a viernes, de 9:00 a 19:00. Te respondemos en cuanto abramos.',
  from: 19 * 60, to: 9 * 60, priority: 5, now });

// ── Lista de difusión de ejemplo ──
const listId = db.prepare('INSERT INTO broadcast_lists (name, created_at) VALUES (?, ?)').run('Renovaciones 1er trimestre', now).lastInsertRowid;
const insMember = db.prepare('INSERT OR IGNORE INTO broadcast_members (list_id, wa_id) VALUES (?, ?)');
for (const waId of ['34612345678', '34677889900', '34911223344']) insMember.run(listId, waId);

const counts = {
  contactos: db.prepare('SELECT COUNT(*) n FROM contacts').get().n,
  conversaciones: db.prepare('SELECT COUNT(*) n FROM conversations').get().n,
  mensajes: db.prepare('SELECT COUNT(*) n FROM messages').get().n,
  reglas: db.prepare('SELECT COUNT(*) n FROM auto_replies').get().n,
};
console.log('Datos de demostración cargados:', counts);
console.log('Arranca el servidor y abre el panel para verlos.');
