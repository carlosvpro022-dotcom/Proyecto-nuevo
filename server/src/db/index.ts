import Database from 'better-sqlite3';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { env, ensureDirs } from '../env.js';

ensureDirs();

export const db = new Database(env.databasePath);
db.pragma('journal_mode = WAL');
db.pragma('foreign_keys = ON');

const here = path.dirname(fileURLToPath(import.meta.url));
const schema = fs.readFileSync(path.join(here, 'schema.sql'), 'utf8');
db.exec(schema);

seedDefaults();

function seedDefaults(): void {
  const now = Date.now();
  const labelCount = db.prepare('SELECT COUNT(*) AS n FROM labels').get() as { n: number };
  if (labelCount.n === 0) {
    const insert = db.prepare('INSERT INTO labels (name, color, created_at) VALUES (?, ?, ?)');
    const defaults: Array<[string, string]> = [
      ['Nuevo cliente', '#25d366'],
      ['Pedido pendiente', '#f6b23c'],
      ['Pago recibido', '#34b7f1'],
      ['Incidencia', '#e8655b'],
      ['Cerrado', '#8696a0'],
    ];
    const tx = db.transaction(() => {
      for (const [name, color] of defaults) insert.run(name, color, now);
    });
    tx();
  }
}

/** Marca una clave de evento como procesada. Devuelve false si ya existía. */
export function claimEvent(key: string): boolean {
  try {
    db.prepare('INSERT INTO processed_events (key, created_at) VALUES (?, ?)').run(key, Date.now());
    return true;
  } catch {
    return false;
  }
}

/** Limpieza periódica de la tabla de idempotencia (7 días). */
export function pruneProcessedEvents(): void {
  const cutoff = Date.now() - 7 * 24 * 60 * 60 * 1000;
  db.prepare('DELETE FROM processed_events WHERE created_at < ?').run(cutoff);
}

export function getSetting(key: string): string | null {
  const row = db.prepare('SELECT value FROM settings WHERE key = ?').get(key) as { value: string } | undefined;
  return row ? row.value : null;
}

export function setSetting(key: string, value: string): void {
  db.prepare(
    'INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value'
  ).run(key, value);
}
