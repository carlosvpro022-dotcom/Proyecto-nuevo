import { env, whatsappConfigured } from '../env.js';
import type { SendResult, TemplateSummary, CatalogProduct } from './types.js';

const GRAPH = 'https://graph.facebook.com';

export class WhatsAppError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: number,
    readonly subcode?: number,
    readonly details?: string
  ) {
    super(message);
    this.name = 'WhatsAppError';
  }

  /** La ventana de 24 h está cerrada: hay que usar una plantilla. */
  get isWindowClosed(): boolean {
    // 131047: "Message failed to send because more than 24 hours have passed"
    // 131051 / 470: variantes históricas de ventana expirada.
    return this.code === 131047 || this.code === 131051 || this.code === 470;
  }
}

function assertConfigured(): void {
  if (!whatsappConfigured) {
    throw new WhatsAppError(
      'WhatsApp no está configurado: define WHATSAPP_ACCESS_TOKEN y WHATSAPP_PHONE_NUMBER_ID en .env',
      503
    );
  }
}

interface GraphOptions {
  method?: 'GET' | 'POST' | 'DELETE';
  body?: unknown;
  query?: Record<string, string | undefined>;
  formData?: FormData;
}

async function graph<T>(endpoint: string, opts: GraphOptions = {}): Promise<T> {
  assertConfigured();
  const url = new URL(`${GRAPH}/${env.graphVersion}/${endpoint.replace(/^\//, '')}`);
  for (const [k, v] of Object.entries(opts.query ?? {})) {
    if (v !== undefined && v !== '') url.searchParams.set(k, v);
  }

  const headers: Record<string, string> = { Authorization: `Bearer ${env.accessToken}` };
  let body: string | FormData | undefined;

  if (opts.formData) {
    body = opts.formData;
  } else if (opts.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(opts.body);
  }

  const res = await fetch(url, { method: opts.method ?? (body ? 'POST' : 'GET'), headers, body });
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = { raw: text };
  }

  if (!res.ok) {
    const err = (parsed as { error?: Record<string, any> } | null)?.error ?? {};
    throw new WhatsAppError(
      err.message ?? `Graph API respondió ${res.status}`,
      res.status,
      err.code,
      err.error_subcode,
      err.error_data?.details
    );
  }
  return parsed as T;
}

// ─── Envío de mensajes ──────────────────────────────────────────────────────

type Payload = Record<string, unknown>;

function sendRaw(payload: Payload): Promise<SendResult> {
  return graph<SendResult>(`${env.phoneNumberId}/messages`, {
    method: 'POST',
    body: { messaging_product: 'whatsapp', recipient_type: 'individual', ...payload },
  });
}

function withContext(payload: Payload, replyTo?: string): Payload {
  return replyTo ? { ...payload, context: { message_id: replyTo } } : payload;
}

export function sendText(to: string, body: string, replyTo?: string, previewUrl = true): Promise<SendResult> {
  return sendRaw(withContext({ to, type: 'text', text: { preview_url: previewUrl, body } }, replyTo));
}

export type MediaKind = 'image' | 'video' | 'audio' | 'document' | 'sticker';

export function sendMedia(
  to: string,
  kind: MediaKind,
  source: { id?: string; link?: string },
  extra: { caption?: string; filename?: string } = {},
  replyTo?: string
): Promise<SendResult> {
  const media: Payload = {};
  if (source.id) media.id = source.id;
  if (source.link) media.link = source.link;
  // Audio y sticker no aceptan caption; document sí acepta filename.
  if (extra.caption && (kind === 'image' || kind === 'video' || kind === 'document')) {
    media.caption = extra.caption;
  }
  if (extra.filename && kind === 'document') media.filename = extra.filename;
  return sendRaw(withContext({ to, type: kind, [kind]: media }, replyTo));
}

export function sendTemplate(
  to: string,
  name: string,
  language: string,
  components?: unknown[]
): Promise<SendResult> {
  return sendRaw({
    to,
    type: 'template',
    template: {
      name,
      language: { code: language },
      ...(components && components.length ? { components } : {}),
    },
  });
}

export function sendReaction(to: string, messageId: string, emoji: string): Promise<SendResult> {
  // emoji vacío = retirar la reacción
  return sendRaw({ to, type: 'reaction', reaction: { message_id: messageId, emoji } });
}

export function sendLocation(
  to: string,
  loc: { latitude: number; longitude: number; name?: string; address?: string },
  replyTo?: string
): Promise<SendResult> {
  return sendRaw(withContext({ to, type: 'location', location: loc }, replyTo));
}

/** Mensaje con un único producto del catálogo. */
export function sendProduct(
  to: string,
  catalogId: string,
  retailerId: string,
  bodyText?: string
): Promise<SendResult> {
  return sendRaw({
    to,
    type: 'interactive',
    interactive: {
      type: 'product',
      ...(bodyText ? { body: { text: bodyText } } : {}),
      action: { catalog_id: catalogId, product_retailer_id: retailerId },
    },
  });
}

/** Mensaje con varios productos agrupados en secciones. */
export function sendProductList(
  to: string,
  catalogId: string,
  header: string,
  bodyText: string,
  sections: Array<{ title: string; product_items: Array<{ product_retailer_id: string }> }>
): Promise<SendResult> {
  return sendRaw({
    to,
    type: 'interactive',
    interactive: {
      type: 'product_list',
      header: { type: 'text', text: header },
      body: { text: bodyText },
      action: { catalog_id: catalogId, sections },
    },
  });
}

/** Marca como leído y, opcionalmente, muestra "escribiendo…" al contacto. */
export async function markAsRead(messageId: string, typing = false): Promise<void> {
  await graph(`${env.phoneNumberId}/messages`, {
    method: 'POST',
    body: {
      messaging_product: 'whatsapp',
      status: 'read',
      message_id: messageId,
      ...(typing ? { typing_indicator: { type: 'text' } } : {}),
    },
  });
}

// ─── Multimedia ─────────────────────────────────────────────────────────────

export async function uploadMedia(bytes: Buffer, mime: string, filename: string): Promise<string> {
  const form = new FormData();
  form.append('messaging_product', 'whatsapp');
  form.append('type', mime);
  form.append('file', new Blob([new Uint8Array(bytes)], { type: mime }), filename);
  const res = await graph<{ id?: string }>(`${env.phoneNumberId}/media`, { method: 'POST', formData: form });
  if (!res.id) throw new WhatsAppError('Meta no devolvió un id de media tras la subida', 502);
  return res.id;
}

export interface MediaMeta {
  url: string;
  mime_type: string;
  sha256: string;
  file_size: number;
  id: string;
}

export function getMediaMeta(mediaId: string): Promise<MediaMeta> {
  return graph<MediaMeta>(mediaId, { query: { phone_number_id: env.phoneNumberId } });
}

/** El CDN de Meta exige el mismo Bearer token que el resto de la API. */
export async function downloadMedia(url: string): Promise<Buffer> {
  assertConfigured();
  const res = await fetch(url, { headers: { Authorization: `Bearer ${env.accessToken}` } });
  if (!res.ok) {
    throw new WhatsAppError(`No se pudo descargar el archivo (${res.status})`, res.status);
  }
  return Buffer.from(await res.arrayBuffer());
}

// ─── Plantillas ─────────────────────────────────────────────────────────────

export async function listTemplates(): Promise<TemplateSummary[]> {
  if (!env.wabaId) throw new WhatsAppError('Falta WHATSAPP_BUSINESS_ACCOUNT_ID para leer plantillas', 503);
  const res = await graph<{ data?: TemplateSummary[] }>(`${env.wabaId}/message_templates`, {
    query: { limit: '200', fields: 'id,name,language,status,category,components' },
  });
  return res.data ?? [];
}

// ─── Perfil y catálogo ──────────────────────────────────────────────────────

export interface BusinessProfile {
  about?: string;
  address?: string;
  description?: string;
  email?: string;
  profile_picture_url?: string;
  websites?: string[];
  vertical?: string;
}

export async function getBusinessProfile(): Promise<BusinessProfile> {
  const res = await graph<{ data?: BusinessProfile[] }>(`${env.phoneNumberId}/whatsapp_business_profile`, {
    query: {
      fields: 'about,address,description,email,profile_picture_url,websites,vertical',
    },
  });
  return res.data?.[0] ?? {};
}

export async function getPhoneNumberInfo(): Promise<Record<string, unknown>> {
  return graph<Record<string, unknown>>(env.phoneNumberId, {
    query: {
      fields:
        'display_phone_number,verified_name,quality_rating,code_verification_status,platform_type,throughput',
    },
  });
}

export interface CatalogSummary {
  id: string;
  name: string;
  product_count?: number;
}

/** Catálogos de comercio conectados a la WABA. */
export async function listCatalogs(): Promise<CatalogSummary[]> {
  if (!env.wabaId) throw new WhatsAppError('Falta WHATSAPP_BUSINESS_ACCOUNT_ID para leer el catálogo', 503);
  const res = await graph<{ data?: CatalogSummary[] }>(`${env.wabaId}/product_catalogs`, {
    query: { fields: 'id,name,product_count', limit: '25' },
  });
  return res.data ?? [];
}

export async function listProducts(catalogId: string, after?: string): Promise<{
  products: CatalogProduct[];
  next?: string;
}> {
  const res = await graph<{ data?: CatalogProduct[]; paging?: { cursors?: { after?: string }; next?: string } }>(
    `${catalogId}/products`,
    {
      query: {
        fields: 'id,retailer_id,name,description,price,currency,availability,image_url,url',
        limit: '100',
        after,
      },
    }
  );
  return {
    products: res.data ?? [],
    next: res.paging?.next ? res.paging.cursors?.after : undefined,
  };
}
