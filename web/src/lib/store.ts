import { create } from 'zustand';
import { api, getApiKey } from './api';
import type { Conversation, Label, Message, SettingsResponse } from '../types';

export type View = 'chats' | 'contacts' | 'labels' | 'broadcasts' | 'catalog' | 'autoreplies' | 'settings';
export type ChatFilter = 'all' | 'unread' | 'archived';

interface Toast {
  id: number;
  text: string;
  kind: 'info' | 'error' | 'success';
}

interface State {
  view: View;
  conversations: Conversation[];
  activeId: number | null;
  messages: Record<number, Message[]>;
  hasMore: Record<number, boolean>;
  labels: Label[];
  settings: SettingsResponse | null;
  search: string;
  filter: ChatFilter;
  labelFilter: number | null;
  loadingChats: boolean;
  loadingMessages: boolean;
  connected: boolean;
  toasts: Toast[];
  replyTo: Message | null;

  setView: (v: View) => void;
  setSearch: (q: string) => void;
  setFilter: (f: ChatFilter) => void;
  setLabelFilter: (id: number | null) => void;
  setReplyTo: (m: Message | null) => void;

  loadConversations: () => Promise<void>;
  loadLabels: () => Promise<void>;
  loadSettings: () => Promise<void>;
  selectConversation: (id: number | null) => Promise<void>;
  loadOlder: (id: number) => Promise<void>;
  patchConversation: (id: number, patch: Parameters<typeof api.updateConversation>[1]) => Promise<void>;

  applyEvent: (event: { type: string; payload: unknown }) => void;
  setConnected: (v: boolean) => void;
  toast: (text: string, kind?: Toast['kind']) => void;
  dismissToast: (id: number) => void;
}

function sortConversations(list: Conversation[]): Conversation[] {
  return [...list].sort((a, b) => {
    if (a.isPinned !== b.isPinned) return a.isPinned ? -1 : 1;
    return (b.lastMessageAt ?? 0) - (a.lastMessageAt ?? 0);
  });
}

let toastSeq = 0;

export const useStore = create<State>((set, get) => ({
  view: 'chats',
  conversations: [],
  activeId: null,
  messages: {},
  hasMore: {},
  labels: [],
  settings: null,
  search: '',
  filter: 'all',
  labelFilter: null,
  loadingChats: false,
  loadingMessages: false,
  connected: false,
  toasts: [],
  replyTo: null,

  setView: (view) => set({ view }),
  setSearch: (search) => {
    set({ search });
    void get().loadConversations();
  },
  setFilter: (filter) => {
    set({ filter });
    void get().loadConversations();
  },
  setLabelFilter: (labelFilter) => {
    set({ labelFilter });
    void get().loadConversations();
  },
  setReplyTo: (replyTo) => set({ replyTo }),

  async loadConversations() {
    set({ loadingChats: true });
    try {
      const { search, filter, labelFilter } = get();
      const res = await api.conversations({
        q: search || undefined,
        filter,
        labelId: labelFilter ?? undefined,
      });
      set({ conversations: sortConversations(res.conversations) });
    } catch (err) {
      get().toast((err as Error).message, 'error');
    } finally {
      set({ loadingChats: false });
    }
  },

  async loadLabels() {
    try {
      const res = await api.labels();
      set({ labels: res.labels });
    } catch {
      /* silencioso: las etiquetas no son críticas para el arranque */
    }
  },

  async loadSettings() {
    try {
      set({ settings: await api.settings() });
    } catch (err) {
      get().toast((err as Error).message, 'error');
    }
  },

  async selectConversation(id) {
    set({ activeId: id, replyTo: null });
    if (id === null) return;

    set({ loadingMessages: true });
    try {
      const res = await api.messages(id);
      set((s) => ({
        messages: { ...s.messages, [id]: res.messages },
        hasMore: { ...s.hasMore, [id]: res.hasMore },
      }));
      const conv = get().conversations.find((c) => c.id === id);
      if (conv && conv.unreadCount > 0) {
        await get().patchConversation(id, { markRead: true });
      }
    } catch (err) {
      get().toast((err as Error).message, 'error');
    } finally {
      set({ loadingMessages: false });
    }
  },

  async loadOlder(id) {
    const current = get().messages[id] ?? [];
    if (current.length === 0) return;
    try {
      const res = await api.messages(id, current[0].id);
      set((s) => ({
        messages: { ...s.messages, [id]: [...res.messages, ...(s.messages[id] ?? [])] },
        hasMore: { ...s.hasMore, [id]: res.hasMore },
      }));
    } catch (err) {
      get().toast((err as Error).message, 'error');
    }
  },

  async patchConversation(id, patch) {
    try {
      const updated = await api.updateConversation(id, patch);
      set((s) => ({
        conversations: sortConversations(s.conversations.map((c) => (c.id === id ? updated : c))),
      }));
      // Archivar saca el chat de la lista activa.
      if (patch.isArchived !== undefined) void get().loadConversations();
    } catch (err) {
      get().toast((err as Error).message, 'error');
    }
  },

  applyEvent(event) {
    if (event.type === 'message:new') {
      const msg = event.payload as Message;
      set((s) => {
        const list = s.messages[msg.conversationId];
        if (!list) return {}; // conversación no abierta: basta con el evento de conversación
        if (list.some((m) => m.id === msg.id)) return {};
        return { messages: { ...s.messages, [msg.conversationId]: [...list, msg] } };
      });

      // Notificación del navegador para entrantes de otros chats.
      const { activeId, settings } = get();
      if (
        msg.direction === 'in' &&
        msg.conversationId !== activeId &&
        settings?.settings.notificationsEnabled === 'true' &&
        typeof Notification !== 'undefined' &&
        Notification.permission === 'granted'
      ) {
        const conv = get().conversations.find((c) => c.id === msg.conversationId);
        new Notification(conv?.name ?? 'Nuevo mensaje', { body: msg.body ?? 'Adjunto recibido' });
      }
      return;
    }

    if (event.type === 'message:update') {
      const msg = event.payload as Message;
      set((s) => {
        const list = s.messages[msg.conversationId];
        if (!list) return {};
        return {
          messages: { ...s.messages, [msg.conversationId]: list.map((m) => (m.id === msg.id ? msg : m)) },
        };
      });
      return;
    }

    if (event.type === 'conversation:update') {
      const conv = event.payload as Conversation;
      set((s) => {
        const exists = s.conversations.some((c) => c.id === conv.id);
        const next = exists
          ? s.conversations.map((c) => (c.id === conv.id ? conv : c))
          : [conv, ...s.conversations];
        // Un chat archivado no debe aparecer en la vista normal.
        const filtered =
          get().filter === 'archived' ? next.filter((c) => c.isArchived) : next.filter((c) => !c.isArchived);
        return { conversations: sortConversations(filtered) };
      });
      return;
    }

    if (event.type === 'contact:update') {
      void get().loadConversations();
    }
  },

  setConnected: (connected) => set({ connected }),

  toast(text, kind = 'info') {
    const id = ++toastSeq;
    set((s) => ({ toasts: [...s.toasts, { id, text, kind }] }));
    setTimeout(() => get().dismissToast(id), kind === 'error' ? 7000 : 3500);
  },

  dismissToast(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
}));

// ─── WebSocket con reconexión exponencial ───────────────────────────────────

let socket: WebSocket | null = null;
let retries = 0;
let reconnectTimer: number | undefined;

export function connectRealtime(): void {
  if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) return;

  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  const key = getApiKey();
  const url = `${proto}://${location.host}/ws${key ? `?key=${encodeURIComponent(key)}` : ''}`;

  socket = new WebSocket(url);

  socket.onopen = () => {
    retries = 0;
    useStore.getState().setConnected(true);
  };

  socket.onmessage = (ev) => {
    try {
      useStore.getState().applyEvent(JSON.parse(ev.data));
    } catch {
      /* mensaje malformado: ignorar */
    }
  };

  const scheduleReconnect = () => {
    useStore.getState().setConnected(false);
    socket = null;
    window.clearTimeout(reconnectTimer);
    const delay = Math.min(1000 * 2 ** retries, 30_000);
    retries++;
    reconnectTimer = window.setTimeout(connectRealtime, delay);
  };

  socket.onclose = scheduleReconnect;
  socket.onerror = () => socket?.close();
}
