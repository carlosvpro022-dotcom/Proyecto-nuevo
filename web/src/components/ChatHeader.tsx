import { Archive, BellOff, Bell, Info, Pin, Search, Trash2, Clock } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';
import { formatWindowLeft } from '../lib/format';
import { Avatar } from './Avatar';
import type { Conversation } from '../types';

export function ChatHeader({
  conv,
  onToggleInfo,
}: {
  conv: Conversation;
  onToggleInfo: () => void;
}) {
  const patch = useStore((s) => s.patchConversation);
  const select = useStore((s) => s.selectConversation);
  const reload = useStore((s) => s.loadConversations);
  const toast = useStore((s) => s.toast);
  const setSearch = useStore((s) => s.setSearch);

  async function remove() {
    if (!confirm(`¿Borrar el historial local de ${conv.name}? Esto no borra nada en WhatsApp.`)) return;
    try {
      await api.deleteConversation(conv.id);
      await select(null);
      await reload();
      toast('Conversación borrada del panel', 'success');
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  return (
    <header className="flex items-center gap-3 border-b border-wa-border bg-wa-surface px-4 py-2">
      <button onClick={onToggleInfo} className="flex min-w-0 flex-1 items-center gap-3 text-left">
        <Avatar name={conv.name} waId={conv.waId} size={40} />
        <span className="min-w-0">
          <span className="block truncate text-[15px]">{conv.name}</span>
          <span className="flex items-center gap-1.5 text-[12px] text-wa-muted">
            +{conv.waId}
            <span className="text-wa-muted/50">·</span>
            <Clock className="h-3 w-3" />
            {conv.windowOpen ? (
              <span className="text-wa-accent">ventana abierta {formatWindowLeft(conv.windowClosesAt)}</span>
            ) : (
              <span className="text-amber-400">ventana cerrada — solo plantillas</span>
            )}
          </span>
        </span>
      </button>

      <div className="flex shrink-0 items-center gap-0.5 text-wa-muted">
        {conv.labels.map((l) => (
          <span
            key={l.id}
            className="mr-1 rounded-full px-2 py-0.5 text-[11px]"
            style={{ background: `${l.color}2a`, color: l.color }}
          >
            {l.name}
          </span>
        ))}
        <button onClick={() => setSearch(conv.name)} title="Buscar en los chats" className="rounded-full p-2 hover:bg-wa-surface-2 hover:text-wa-text">
          <Search className="h-5 w-5" />
        </button>
        <button
          onClick={() => void patch(conv.id, { isMuted: !conv.isMuted })}
          title={conv.isMuted ? 'Activar notificaciones' : 'Silenciar'}
          className="rounded-full p-2 hover:bg-wa-surface-2 hover:text-wa-text"
        >
          {conv.isMuted ? <BellOff className="h-5 w-5" /> : <Bell className="h-5 w-5" />}
        </button>
        <button
          onClick={() => void patch(conv.id, { isPinned: !conv.isPinned })}
          title={conv.isPinned ? 'Dejar de fijar' : 'Fijar'}
          className={`rounded-full p-2 hover:bg-wa-surface-2 hover:text-wa-text ${conv.isPinned ? 'text-wa-accent' : ''}`}
        >
          <Pin className="h-5 w-5" />
        </button>
        <button
          onClick={() => void patch(conv.id, { isArchived: !conv.isArchived })}
          title={conv.isArchived ? 'Desarchivar' : 'Archivar'}
          className="rounded-full p-2 hover:bg-wa-surface-2 hover:text-wa-text"
        >
          <Archive className="h-5 w-5" />
        </button>
        <button onClick={() => void remove()} title="Borrar del panel" className="rounded-full p-2 hover:bg-wa-surface-2 hover:text-red-400">
          <Trash2 className="h-5 w-5" />
        </button>
        <button onClick={onToggleInfo} title="Datos del contacto" className="rounded-full p-2 hover:bg-wa-surface-2 hover:text-wa-text">
          <Info className="h-5 w-5" />
        </button>
      </div>
    </header>
  );
}
