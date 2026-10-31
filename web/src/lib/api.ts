import type {
  AutoReplyRule,
  BroadcastList,
  CatalogSummary,
  Contact,
  Conversation,
  Label,
  Message,
  Product,
  SettingsResponse,
  Stats,
  Template,
} from '../types';

const KEY_STORAGE = 'wa-manager-api-key';

export function getApiKey(): string {
  return localStorage.getItem(KEY_STORAGE) ?? '';
}

export function setApiKey(key: string): void {
  if (key) localStorage.setItem(KEY_STORAGE, key);
  else localStorage.removeItem(KEY_STORAGE);
}

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly data: Record<string, unknown> = {}
  ) {
    super(message);
  }
  get windowClosed(): boolean {
    return this.data.windowClosed === true || this.status === 409;
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  const key = getApiKey();
  if (key) headers.set('x-api-key', key);
  if (init.body && !(init.body instanceof FormData)) headers.set('Content-Type', 'application/json');

  const res = await fetch(`/api${path}`, { ...init, headers });
  const text = await res.text();
  let data: Record<string, unknown> = {};
  try {
    data = text ? JSON.parse(text) : {};
  } catch {
    data = { raw: text };
  }

  if (!res.ok) {
    throw new ApiError(String(data.error ?? `Error ${res.status}`), res.status, data);
  }
  return data as T;
}

const json = (body: unknown): RequestInit => ({ body: JSON.stringify(body) });

export const api = {
  // Conversaciones
  conversations(params: { q?: string; filter?: string; labelId?: number } = {}) {
    const qs = new URLSearchParams();
    if (params.q) qs.set('q', params.q);
    if (params.filter) qs.set('filter', params.filter);
    if (params.labelId) qs.set('labelId', String(params.labelId));
    return request<{ conversations: Conversation[] }>(`/conversations?${qs}`);
  },
  conversation: (id: number) => request<Conversation>(`/conversations/${id}`),
  openConversation: (waId: string, name?: string) =>
    request<Conversation>('/conversations', { method: 'POST', ...json({ waId, name }) }),
  updateConversation: (
    id: number,
    patch: { isArchived?: boolean; isPinned?: boolean; isMuted?: boolean; markRead?: boolean }
  ) => request<Conversation>(`/conversations/${id}`, { method: 'PATCH', ...json(patch) }),
  deleteConversation: (id: number) => request<{ ok: true }>(`/conversations/${id}`, { method: 'DELETE' }),

  // Mensajes
  messages: (id: number, before?: number) =>
    request<{ messages: Message[]; hasMore: boolean }>(
      `/conversations/${id}/messages?limit=60${before ? `&before=${before}` : ''}`
    ),
  sendText: (id: number, text: string, replyTo?: string) =>
    request<{ message: Message }>(`/conversations/${id}/messages`, { method: 'POST', ...json({ text, replyTo }) }),
  sendMedia: (id: number, file: File, caption?: string, replyTo?: string) => {
    const form = new FormData();
    form.append('file', file);
    if (caption) form.append('caption', caption);
    if (replyTo) form.append('replyTo', replyTo);
    return request<{ message: Message }>(`/conversations/${id}/media`, { method: 'POST', body: form });
  },
  sendTemplate: (id: number, name: string, language: string, components?: unknown[]) =>
    request<{ message: Message }>(`/conversations/${id}/template`, {
      method: 'POST',
      ...json({ name, language, components }),
    }),
  sendLocation: (id: number, loc: { latitude: number; longitude: number; name?: string }) =>
    request<{ message: Message }>(`/conversations/${id}/location`, { method: 'POST', ...json(loc) }),
  react: (wamId: string, emoji: string) =>
    request<{ ok: true }>(`/messages/${encodeURIComponent(wamId)}/reaction`, { method: 'POST', ...json({ emoji }) }),

  // Contactos y etiquetas
  contacts: (params: { q?: string; labelId?: number } = {}) => {
    const qs = new URLSearchParams();
    if (params.q) qs.set('q', params.q);
    if (params.labelId) qs.set('labelId', String(params.labelId));
    return request<{ contacts: Contact[] }>(`/contacts?${qs}`);
  },
  updateContact: (
    waId: string,
    patch: { displayName?: string | null; notes?: string | null; isBlocked?: boolean }
  ) => request<Contact>(`/contacts/${waId}`, { method: 'PATCH', ...json(patch) }),
  labels: () => request<{ labels: Label[] }>('/labels'),
  createLabel: (name: string, color: string) => request<Label>('/labels', { method: 'POST', ...json({ name, color }) }),
  updateLabel: (id: number, patch: { name?: string; color?: string }) =>
    request<Label>(`/labels/${id}`, { method: 'PATCH', ...json(patch) }),
  deleteLabel: (id: number) => request<{ ok: true }>(`/labels/${id}`, { method: 'DELETE' }),
  setContactLabels: (waId: string, labelIds: number[]) =>
    request<{ labels: Label[] }>(`/contacts/${waId}/labels`, { method: 'PUT', ...json({ labelIds }) }),

  // Difusión
  broadcasts: () => request<{ lists: BroadcastList[] }>('/broadcasts'),
  createBroadcast: (name: string, members: string[]) =>
    request<BroadcastList>('/broadcasts', { method: 'POST', ...json({ name, members }) }),
  setBroadcastMembers: (id: number, members: string[]) =>
    request<{ ok: true }>(`/broadcasts/${id}/members`, { method: 'PUT', ...json({ members }) }),
  deleteBroadcast: (id: number) => request<{ ok: true }>(`/broadcasts/${id}`, { method: 'DELETE' }),
  sendBroadcast: (id: number, payload: { text?: string; template?: { name: string; language: string } }) =>
    request<{ sent: number; failed: number; results: Array<{ waId: string; ok: boolean; error?: string }> }>(
      `/broadcasts/${id}/send`,
      { method: 'POST', ...json(payload) }
    ),

  // Negocio
  templates: (refresh = false) =>
    request<{ templates: Template[] }>(`/templates${refresh ? '?refresh=1' : ''}`),
  profile: () => request<{ profile: Record<string, unknown>; phone: Record<string, unknown> }>('/profile'),
  catalogs: (refresh = false) => request<{ catalogs: CatalogSummary[] }>(`/catalogs${refresh ? '?refresh=1' : ''}`),
  products: (catalogId: string, q?: string) =>
    request<{ products: Product[]; next?: string }>(
      `/catalogs/${catalogId}/products${q ? `?q=${encodeURIComponent(q)}` : ''}`
    ),
  sendProducts: (conversationId: number, catalogId: string, retailerIds: string[], body?: string) =>
    request<{ message: Message }>(`/conversations/${conversationId}/products`, {
      method: 'POST',
      ...json({ catalogId, retailerIds, body }),
    }),

  // Ajustes
  settings: () => request<SettingsResponse>('/settings'),
  saveSettings: (values: Record<string, string>) =>
    request<{ settings: Record<string, string | null> }>('/settings', { method: 'PUT', ...json(values) }),
  autoReplies: () => request<{ rules: AutoReplyRule[] }>('/auto-replies'),
  createAutoReply: (rule: Record<string, unknown>) =>
    request<AutoReplyRule>('/auto-replies', { method: 'POST', ...json(rule) }),
  updateAutoReply: (id: number, patch: Record<string, unknown>) =>
    request<AutoReplyRule>(`/auto-replies/${id}`, { method: 'PATCH', ...json(patch) }),
  deleteAutoReply: (id: number) => request<{ ok: true }>(`/auto-replies/${id}`, { method: 'DELETE' }),
  stats: () => request<Stats>('/stats'),
};

/** Las descargas de media también necesitan la API key. */
export function mediaUrl(path: string | null): string | undefined {
  if (!path) return undefined;
  const key = getApiKey();
  return key ? `${path}${path.includes('?') ? '&' : '?'}key=${encodeURIComponent(key)}` : path;
}
