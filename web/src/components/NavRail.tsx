import {
  MessageSquare,
  Users,
  Tags,
  Megaphone,
  ShoppingBag,
  Bot,
  Settings as SettingsIcon,
  Wifi,
  WifiOff,
} from 'lucide-react';
import { useStore, type View } from '../lib/store';

const ITEMS: Array<{ view: View; label: string; Icon: typeof MessageSquare }> = [
  { view: 'chats', label: 'Chats', Icon: MessageSquare },
  { view: 'contacts', label: 'Contactos', Icon: Users },
  { view: 'labels', label: 'Etiquetas', Icon: Tags },
  { view: 'broadcasts', label: 'Difusión', Icon: Megaphone },
  { view: 'catalog', label: 'Catálogo', Icon: ShoppingBag },
  { view: 'autoreplies', label: 'Respuestas automáticas', Icon: Bot },
];

export function NavRail() {
  const view = useStore((s) => s.view);
  const setView = useStore((s) => s.setView);
  const connected = useStore((s) => s.connected);
  const unread = useStore((s) => s.conversations.reduce((n, c) => n + c.unreadCount, 0));

  return (
    <nav className="flex w-16 flex-col items-center gap-1 border-r border-wa-border bg-wa-panel py-3">
      {ITEMS.map(({ view: v, label, Icon }) => (
        <button
          key={v}
          onClick={() => setView(v)}
          title={label}
          aria-label={label}
          aria-current={view === v ? 'page' : undefined}
          className={`relative rounded-lg p-3 transition ${
            view === v ? 'bg-wa-surface-2 text-wa-accent' : 'text-wa-muted hover:bg-wa-surface hover:text-wa-text'
          }`}
        >
          <Icon className="h-5 w-5" />
          {v === 'chats' && unread > 0 && (
            <span className="absolute top-1.5 right-1.5 min-w-[18px] rounded-full bg-wa-accent-strong px-1 text-[10px] leading-[18px] font-bold text-black">
              {unread > 99 ? '99+' : unread}
            </span>
          )}
        </button>
      ))}

      <div className="mt-auto flex flex-col items-center gap-2">
        <span
          title={connected ? 'Conectado en tiempo real' : 'Sin conexión: reintentando…'}
          className={connected ? 'text-wa-accent' : 'text-red-400'}
        >
          {connected ? <Wifi className="h-4 w-4" /> : <WifiOff className="h-4 w-4" />}
        </span>
        <button
          onClick={() => setView('settings')}
          title="Ajustes"
          aria-label="Ajustes"
          className={`rounded-lg p-3 transition ${
            view === 'settings' ? 'bg-wa-surface-2 text-wa-accent' : 'text-wa-muted hover:bg-wa-surface hover:text-wa-text'
          }`}
        >
          <SettingsIcon className="h-5 w-5" />
        </button>
      </div>
    </nav>
  );
}
