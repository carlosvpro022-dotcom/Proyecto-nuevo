import { Router } from 'express';
import { z } from 'zod';
import * as wa from '../whatsapp/client.js';
import { WhatsAppError } from '../whatsapp/client.js';
import { conversationById } from '../services/store.js';
import * as sender from '../services/sender.js';

export const businessRouter = Router();

function fail(res: import('express').Response, err: unknown) {
  const e = err instanceof WhatsAppError ? err : new WhatsAppError((err as Error).message, 500);
  const status = e.status >= 400 && e.status < 600 ? e.status : 502;
  res.status(status).json({ error: e.message, code: e.code, details: e.details });
}

/** Caché simple: plantillas y catálogo cambian poco y la API tiene límites. */
function cache<T>(ttlMs: number) {
  let value: T | null = null;
  let at = 0;
  return {
    async get(load: () => Promise<T>, force = false): Promise<T> {
      if (!force && value !== null && Date.now() - at < ttlMs) return value;
      value = await load();
      at = Date.now();
      return value;
    },
    clear() {
      value = null;
      at = 0;
    },
  };
}

const templatesCache = cache<Awaited<ReturnType<typeof wa.listTemplates>>>(5 * 60 * 1000);
const catalogsCache = cache<Awaited<ReturnType<typeof wa.listCatalogs>>>(10 * 60 * 1000);

// ─── Plantillas ─────────────────────────────────────────────────────────────

businessRouter.get('/templates', async (req, res) => {
  try {
    const all = await templatesCache.get(() => wa.listTemplates(), req.query.refresh === '1');
    const onlyApproved = req.query.all !== '1';
    res.json({
      templates: onlyApproved ? all.filter((t) => t.status === 'APPROVED') : all,
    });
  } catch (err) {
    fail(res, err);
  }
});

// ─── Perfil del negocio y número ────────────────────────────────────────────

businessRouter.get('/profile', async (_req, res) => {
  try {
    const [profile, phone] = await Promise.all([
      wa.getBusinessProfile().catch(() => ({})),
      wa.getPhoneNumberInfo().catch(() => ({})),
    ]);
    res.json({ profile, phone });
  } catch (err) {
    fail(res, err);
  }
});

// ─── Catálogo ───────────────────────────────────────────────────────────────

businessRouter.get('/catalogs', async (req, res) => {
  try {
    const catalogs = await catalogsCache.get(() => wa.listCatalogs(), req.query.refresh === '1');
    res.json({ catalogs });
  } catch (err) {
    fail(res, err);
  }
});

businessRouter.get('/catalogs/:catalogId/products', async (req, res) => {
  try {
    const after = typeof req.query.after === 'string' ? req.query.after : undefined;
    const { products, next } = await wa.listProducts(req.params.catalogId, after);

    const q = String(req.query.q ?? '').trim().toLowerCase();
    const filtered = q
      ? products.filter(
          (p) =>
            p.name?.toLowerCase().includes(q) ||
            p.retailer_id?.toLowerCase().includes(q) ||
            p.description?.toLowerCase().includes(q)
        )
      : products;

    res.json({ products: filtered, next });
  } catch (err) {
    fail(res, err);
  }
});

/** Envía uno o varios productos del catálogo a una conversación. */
businessRouter.post('/conversations/:id/products', async (req, res) => {
  const conv = conversationById(Number(req.params.id));
  if (!conv) return res.status(404).json({ error: 'Conversación no encontrada' });

  const parsed = z
    .object({
      catalogId: z.string().min(1),
      retailerIds: z.array(z.string().min(1)).min(1).max(30),
      body: z.string().trim().max(1024).optional(),
      header: z.string().trim().max(60).optional(),
    })
    .safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: 'Faltan catalogId o retailerIds' });

  const { catalogId, retailerIds, body, header } = parsed.data;

  const result =
    retailerIds.length === 1
      ? await sender.sendProduct(conv.wa_id, catalogId, retailerIds[0], body)
      : await sender.sendProductList(
          conv.wa_id,
          catalogId,
          header ?? 'Catálogo',
          body ?? 'Estos son nuestros productos:',
          retailerIds
        );

  if (result.error) {
    return res.status(result.error.isWindowClosed ? 409 : 502).json({
      error: result.error.message,
      code: result.error.code,
      windowClosed: result.error.isWindowClosed,
      message: result.message,
    });
  }
  res.status(201).json({ message: result.message });
});
