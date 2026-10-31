import 'dotenv/config';
import path from 'node:path';
import fs from 'node:fs';

function str(name: string, fallback = ''): string {
  return (process.env[name] ?? fallback).trim();
}

function bool(name: string, fallback = false): boolean {
  const v = str(name);
  if (!v) return fallback;
  return ['1', 'true', 'yes', 'si', 'sí'].includes(v.toLowerCase());
}

const root = path.resolve(process.cwd());

export const env = {
  port: Number(str('PORT', '3001')),

  accessToken: str('WHATSAPP_ACCESS_TOKEN'),
  phoneNumberId: str('WHATSAPP_PHONE_NUMBER_ID'),
  wabaId: str('WHATSAPP_BUSINESS_ACCOUNT_ID'),
  appSecret: str('META_APP_SECRET'),
  verifyToken: str('WEBHOOK_VERIFY_TOKEN'),
  graphVersion: str('GRAPH_API_VERSION', 'v23.0'),

  databasePath: path.resolve(root, str('DATABASE_PATH', './data/app.db')),
  mediaDir: path.resolve(root, str('MEDIA_DIR', './data/media')),

  apiKey: str('APP_API_KEY'),
  allowUnsignedWebhooks: bool('ALLOW_UNSIGNED_WEBHOOKS', false),
};

/** true cuando hay credenciales suficientes para hablar con Meta. */
export const whatsappConfigured = Boolean(env.accessToken && env.phoneNumberId);

export function ensureDirs(): void {
  fs.mkdirSync(path.dirname(env.databasePath), { recursive: true });
  fs.mkdirSync(env.mediaDir, { recursive: true });
}

/** Avisos de configuración que se muestran al arrancar y en /api/settings. */
export function configWarnings(): string[] {
  const w: string[] = [];
  if (!env.accessToken) w.push('Falta WHATSAPP_ACCESS_TOKEN: no se podrán enviar ni recibir mensajes reales.');
  if (!env.phoneNumberId) w.push('Falta WHATSAPP_PHONE_NUMBER_ID: no se puede enviar.');
  if (!env.wabaId) w.push('Falta WHATSAPP_BUSINESS_ACCOUNT_ID: plantillas y catálogo no estarán disponibles.');
  if (!env.appSecret && !env.allowUnsignedWebhooks)
    w.push('Falta META_APP_SECRET: los webhooks entrantes se rechazarán por firma inválida.');
  if (!env.verifyToken) w.push('Falta WEBHOOK_VERIFY_TOKEN: Meta no podrá verificar la URL del webhook.');
  if (!env.apiKey) w.push('APP_API_KEY vacía: la API queda abierta. No publiques esto en internet así.');
  if (env.appSecret === '' && env.allowUnsignedWebhooks)
    w.push('ALLOW_UNSIGNED_WEBHOOKS=true: cualquiera puede inyectar mensajes falsos. Solo para desarrollo.');
  return w;
}
