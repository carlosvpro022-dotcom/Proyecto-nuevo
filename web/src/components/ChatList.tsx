import { useState } from 'react';
import {
  Search,
  MessageSquarePlus,
  Pin,
  Archive,
  BellOff,
  Check,
  CheckCheck,
  Image as ImageIcon,
  FileText,
  Mic,
  Video,
  MapPin,
  ShoppingBag,
  X,
} from 'lucide-react';
import { useStore } from '../lib/store';
import { api } from '../lib/api';
import { formatListStamp } from '../lib/format';
import { Avatar } from './Avatar';
import { Modal } from './Modal';
import type { Conversation, Message } from '../types';

const FILTERS = [
  { id: 'all', label: 'Todos' },
  { id: 'unread', label: 'No leídos' },
  { id: 'archived', label: 'Archivados' },
] as const;

function PreviewIcon({ message }: { message: Message }) {
  const cls = 'h-3.5 w-3.5 shrink-0';
  switch (message.type) {
    case 'image':
    case 'sticker':
      return <ImageIcon className={cls} />;
    case 'video':
      return <Video className={cls} />;
    case 'audio':
      return <Mic className={cls} />;
    case 'document':
      return <FileText className={cls} />;
    case 'location':
      return <MapPin className={cls} />;
    case 'order':
    case 'interactive':
      return <ShoppingBag className={cls} />;
    default:
      return null;
  }
}

function previewText(message: Message): string {
  if (message.body) return message.body;
  switch (message.type) {
    case 'image':
      return 'Foto';
    case 'sticker':
      return 'Sticker';
    case 'video':
      return 'Vídeo';
    case 'audio':
      return 'Audio';
    case 'document':
      return message.media?.filename ?? 'Documento';
    case 'location':
      return 'Ubicación';
    case 'contacts':
      return 'Contacto';
    case 'order':
      return 'Pedido';
    default:
      return 'Mensaje';
  }
}

function Row({ conv }: { conv: Conversation }) {
  const activeId = useStore((s) => s.activeId);
  const select = useStore((s) => s.selectConversation);
  const patch = useStore((s) => s.patchConversation);
  const active = activeId === conv.id;
  const last = conv.lastMessage;

  return (
    <div
      onClick={() => void select(conv.id)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          void select(conv.id);
        }
      }}
      className={`group flex cursor-pointer items-center gap-3 px-3 py-2.5 transition ${
        active ? 'bg-wa-surface-2' : 'hover:bg-wa-surface'
      }`}
    >
      <Avatar name={conv.name} waId={conv.waId} />

      <div className="min-w-0 flex-1 border-b border-wa-border/60 pb-2.5 group-last:border-0">
        <div className="flex items-baseline justify-between gap-2">
          <span className="truncate text-[15px] text-wa-text">{conv.name}</span>
          <span className={`shrink-0 text-[11px] ${conv.unreadCount ? 'text-wa-accent' : 'text-wa-muted'}`}>
            {formatListStamp(conv.lastMessageAt)}
          </span>
        </div>

        <div className="mt-0.5 flex items-center gap-1.5 text-[13px] text-wa-muted">
          {last?.direction === 'out' &&
            (last.status === 'read' ? (
              <CheckCheck className="h-3.5 w-3.5 shrink-0 text-wa-blue" />
            ) : last.status === 'failed' ? (
              <X className="h-3.5 w-3.5 shrink-0 text-red-400" />
            ) : (
              <Check className="h-3.5 w-3.5 shrink-0" />
            ))}
          {last && <PreviewIcon message={last} />}
          <span className="truncate">{last ? previewText(last) : 'Sin mensajes todavía'}</span>

          <span className="ml-auto flex shrink-0 items-center gap-1.5">
            {conv.labels.slice(0, 3).map((l) => (
              <span
                key={l.id}
                title={l.name}
                className="h-2 w-2 rounded-full"
                style={{ background: l.color }}
              />
            ))}
            {conv.isMuted && <BellOff className="h-3.5 w-3.5" />}
            {conv.isPinned && <Pin className="h-3.5 w-3.5" />}
            {conv.unreadCount > 0 && (
              <span className="min-w-[20px] rounded-full bg-wa-accent-strong px-1.5 text-center text-[11px] font-bold text-black">
                {conv.unreadCount}
              </span>
            )}
          </span>
        </div>
      </div>

      <div className="flex shrink-0 flex-col gap-1 opacity-0 transition group-hover:opacity-100">
        <button
          onClick={(e) => {
            e.stopPropagation();
            void patch(conv.id, { isPinned: !conv.isPinned });
          }}
          title={conv.isPinned ? 'Dejar de fijar' : 'Fijar chat'}
          className="rounded p-1 text-wa-muted hover:bg-wa-surface-2 hover:text-wa-text"
        >
          <Pin className="h-3.5 w-3.5" />
        </button>
        <button
          onClick={(e) => {
            e.stopPropagation();
            void patch(conv.id, { isArchived: !conv.isArchived });
          }}
          title={conv.isArchived ? 'Desarchivar' : 'Archivar'}
          className="rounded p-1 text-wa-muted hover:bg-wa-surface-2 hover:text-wa-text"
        >
          <Archive className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}

function NewChatDialog({ onClose }: { onClose: () => void }) {
  const [number, setNumber] = useState('');
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const select = useStore((s) => s.selectConversation);
  const reload = useStore((s) => s.loadConversations);
  const toast = useStore((s) => s.toast);

  async function submit() {
    const digits = number.replace(/\D/g, '');
    if (digits.length < 8) {
      toast('Escribe el número con prefijo de país, por ejemplo 34600111222', 'error');
      return;
    }
    setBusy(true);
    try {
      const conv = await api.openConversation(digits, name.trim() || undefined);
      await reload();
      await select(conv.id);
      onClose();
    } catch (err) {
      toast((err as Error).message, 'error');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title="Nuevo chat" onClose={onClose}>
      <div className="space-y-4">
        <label className="block">
          <span className="mb-1.5 block text-xs text-wa-muted">Número con prefijo de país (sin + ni espacios)</span>
          <input
            autoFocus
            value={number}
            onChange={(e) => setNumber(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && void submit()}
            placeholder="34600111222"
            inputMode="numeric"
            className="w-full rounded-lg bg-wa-surface-2 px-3 py-2.5 text-sm placeholder:text-wa-muted"
          />
        </label>
        <label className="block">
          <span className="mb-1.5 block text-xs text-wa-muted">Nombre (opcional)</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && void submit()}
            placeholder="María García"
            className="w-full rounded-lg bg-wa-surface-2 px-3 py-2.5 text-sm placeholder:text-wa-muted"
          />
        </label>

        <p className="rounded-lg bg-wa-surface/60 p-3 text-xs leading-relaxed text-wa-muted">
          Si este contacto no te ha escrito en las últimas 24 h, el primer mensaje tiene que ser una{' '}
          <strong className="text-wa-text">plantilla aprobada</strong>. Es una regla de Meta, no de esta app.
        </p>

        <button
          onClick={() => void submit()}
          disabled={busy}
          className="w-full rounded-lg bg-wa-accent py-2.5 text-sm font-medium text-black transition hover:brightness-110 disabled:opacity-50"
        >
          {busy ? 'Abriendo…' : 'Abrir chat'}
        </button>
      </div>
    </Modal>
  );
}

export function ChatList() {
  const conversations = useStore((s) => s.conversations);
  const search = useStore((s) => s.search);
  const setSearch = useStore((s) => s.setSearch);
  const filter = useStore((s) => s.filter);
  const setFilter = useStore((s) => s.setFilter);
  const labels = useStore((s) => s.labels);
  const labelFilter = useStore((s) => s.labelFilter);
  const setLabelFilter = useStore((s) => s.setLabelFilter);
  const loading = useStore((s) => s.loadingChats);
  const [newChat, setNewChat] = useState(false);

  return (
    <aside className="flex w-full max-w-[400px] min-w-[300px] flex-col border-r border-wa-border bg-wa-panel">
      <header className="flex items-center justify-between px-4 py-3">
        <h1 className="text-lg font-medium">Chats</h1>
        <button
          onClick={() => setNewChat(true)}
          title="Nuevo chat"
          className="rounded-full p-2 text-wa-muted transition hover:bg-wa-surface-2 hover:text-wa-text"
        >
          <MessageSquarePlus className="h-5 w-5" />
        </button>
      </header>

      <div className="px-3 pb-2">
        <div className="flex items-center gap-2 rounded-lg bg-wa-surface px-3 py-1.5">
          <Search className="h-4 w-4 shrink-0 text-wa-muted" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Buscar chat o mensaje"
            className="w-full bg-transparent py-1 text-sm placeholder:text-wa-muted"
          />
          {search && (
            <button onClick={() => setSearch('')} aria-label="Limpiar búsqueda" className="text-wa-muted">
              <X className="h-4 w-4" />
            </button>
          )}
        </div>
      </div>

      <div className="flex flex-wrap gap-1.5 px-3 pb-2.5">
        {FILTERS.map((f) => (
          <button
            key={f.id}
            onClick={() => setFilter(f.id)}
            className={`rounded-full px-3 py-1 text-xs transition ${
              filter === f.id ? 'bg-wa-accent/20 text-wa-accent' : 'bg-wa-surface text-wa-muted hover:text-wa-text'
            }`}
          >
            {f.label}
          </button>
        ))}
        {labels.map((l) => (
          <button
            key={l.id}
            onClick={() => setLabelFilter(labelFilter === l.id ? null : l.id)}
            className={`flex items-center gap-1.5 rounded-full px-3 py-1 text-xs transition ${
              labelFilter === l.id ? 'text-wa-text' : 'bg-wa-surface text-wa-muted hover:text-wa-text'
            }`}
            style={labelFilter === l.id ? { background: `${l.color}33`, color: l.color } : undefined}
          >
            <span className="h-2 w-2 rounded-full" style={{ background: l.color }} />
            {l.name}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading && conversations.length === 0 && (
          <p className="px-4 py-8 text-center text-sm text-wa-muted">Cargando…</p>
        )}
        {!loading && conversations.length === 0 && (
          <div className="px-6 py-10 text-center text-sm text-wa-muted">
            <p>No hay conversaciones{search ? ' que coincidan' : ''}.</p>
            {!search && (
              <p className="mt-2 text-xs">
                Aparecerán aquí en cuanto alguien escriba a tu número de WhatsApp Business, o cuando abras un chat
                nuevo.
              </p>
            )}
          </div>
        )}
        {conversations.map((c) => (
          <Row key={c.id} conv={c} />
        ))}
      </div>

      {newChat && <NewChatDialog onClose={() => setNewChat(false)} />}
    </aside>
  );
}
