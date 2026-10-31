import express, { type NextFunction, type Request, type Response } from 'express';
import cors from 'cors';
import http from 'node:http';
import path from 'node:path';
import fs from 'node:fs';
import { env, configWarnings, whatsappConfigured } from './env.js';
import { pruneProcessedEvents } from './db/index.js';
import { attachRealtime } from './realtime/hub.js';
import { verifyWebhook, receiveWebhook } from './whatsapp/webhook.js';
import { conversationsRouter } from './routes/conversations.js';
import { directoryRouter } from './routes/directory.js';
import { businessRouter } from './routes/business.js';
import { settingsRouter } from './routes/settings.js';

const app = express();
app.set('trust proxy', true);
app.disable('x-powered-by');

app.use(cors({ origin: true, credentials: false }));

// Guardamos el cuerpo crudo: la firma de Meta se calcula sobre los bytes exactos.
app.use(
  express.json({
    limit: '5mb',
    verify: (req, _res, buf) => {
      (req as Request & { rawBody?: Buffer }).rawBody = Buffer.from(buf);
    },
  })
);
app.use(express.urlencoded({ extended: true }));

// ─── Webhook de Meta (sin API key: lo protege la firma HMAC) ────────────────
app.get('/webhook', verifyWebhook);
app.post('/webhook', receiveWebhook);

app.get('/health', (_req, res) => {
  res.json({ ok: true, whatsapp: whatsappConfigured, uptime: Math.round(process.uptime()) });
});

// ─── API del panel ──────────────────────────────────────────────────────────
function requireApiKey(req: Request, res: Response, next: NextFunction): void {
  if (!env.apiKey) {
    next();
    return;
  }
  const provided = req.get('x-api-key') ?? (typeof req.query.key === 'string' ? req.query.key : '');
  if (provided === env.apiKey) {
    next();
    return;
  }
  res.status(401).json({ error: 'API key inválida o ausente' });
}

app.use('/api', requireApiKey, conversationsRouter, directoryRouter, businessRouter, settingsRouter);

// ─── Frontend compilado (si existe) ─────────────────────────────────────────
const webDist = path.resolve(process.cwd(), '../web/dist');
if (fs.existsSync(webDist)) {
  app.use(express.static(webDist));
  app.get(/^(?!\/api|\/webhook|\/ws|\/health).*/, (_req, res) => {
    res.sendFile(path.join(webDist, 'index.html'));
  });
}

// ─── Manejo de errores ──────────────────────────────────────────────────────
app.use((err: Error & { code?: string }, _req: Request, res: Response, _next: NextFunction) => {
  if (err.code === 'LIMIT_FILE_SIZE') {
    res.status(413).json({ error: 'El archivo supera el límite de 95 MB de la Cloud API' });
    return;
  }
  console.error('[error]', err);
  res.status(500).json({ error: err.message || 'Error interno' });
});

const server = http.createServer(app);
attachRealtime(server);

setInterval(pruneProcessedEvents, 6 * 60 * 60 * 1000).unref();

server.listen(env.port, () => {
  console.log(`\n  Gestor de WhatsApp escuchando en http://localhost:${env.port}`);
  console.log(`  Webhook:  POST http://localhost:${env.port}/webhook`);
  console.log(`  Realtime: ws://localhost:${env.port}/ws\n`);

  const warnings = configWarnings();
  if (warnings.length) {
    console.log('  Avisos de configuración:');
    for (const w of warnings) console.log(`   • ${w}`);
    console.log('');
  }
});

for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => {
    server.close(() => process.exit(0));
  });
}
