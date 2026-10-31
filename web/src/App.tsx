import { useEffect, useState } from 'react';
import { MessageSquare } from 'lucide-react';
import { connectRealtime, useStore } from './lib/store';
import { NavRail } from './components/NavRail';
import { ChatList } from './components/ChatList';
import { ChatHeader } from './components/ChatHeader';
import { MessageList } from './components/MessageList';
import { Composer } from './components/Composer';
import { ContactPanel } from './components/ContactPanel';
import { Toasts } from './components/Toasts';
import { ContactsView } from './views/ContactsView';
import { LabelsView } from './views/LabelsView';
import { BroadcastsView } from './views/BroadcastsView';
import { CatalogView } from './views/CatalogView';
import { AutoRepliesView } from './views/AutoRepliesView';
import { SettingsView } from './views/SettingsView';

function EmptyChat() {
  const settings = useStore((s) => s.settings);

  return (
    <section className="chat-wallpaper flex min-w-0 flex-1 flex-col items-center justify-center gap-4 border-b-4 border-wa-accent px-8 text-center">
      <MessageSquare className="h-20 w-20 text-wa-surface-2" strokeWidth={1} />
      <div>
        <h2 className="text-2xl font-light text-wa-text">Gestor de WhatsApp Business</h2>
        <p className="mt-2 max-w-md text-sm text-wa-muted">
          Elige una conversación para empezar. Los mensajes nuevos llegan solos por webhook, sin recargar la página.
        </p>
      </div>
      {settings && !settings.whatsapp.configured && (
        <div className="max-w-md rounded-lg bg-amber-500/10 p-4 text-left text-xs text-amber-200">
          <p className="font-medium">Faltan credenciales de WhatsApp</p>
          <p className="mt-1 opacity-90">
            Rellena el archivo <code className="rounded bg-black/30 px-1">.env</code> con el token, el phone number
            ID y el WABA ID. Tienes los detalles en Ajustes.
          </p>
        </div>
      )}
    </section>
  );
}

function ChatsView() {
  const activeId = useStore((s) => s.activeId);
  const conv = useStore((s) => s.conversations.find((c) => c.id === s.activeId) ?? null);
  const [showInfo, setShowInfo] = useState(false);

  return (
    <>
      <ChatList />
      {activeId && conv ? (
        <>
          <section className="flex min-w-0 flex-1 flex-col">
            <ChatHeader conv={conv} onToggleInfo={() => setShowInfo((v) => !v)} />
            <MessageList conversationId={conv.id} />
            <Composer conv={conv} />
          </section>
          {showInfo && <ContactPanel conv={conv} onClose={() => setShowInfo(false)} />}
        </>
      ) : (
        <EmptyChat />
      )}
    </>
  );
}

export default function App() {
  const view = useStore((s) => s.view);
  const loadConversations = useStore((s) => s.loadConversations);
  const loadLabels = useStore((s) => s.loadLabels);
  const loadSettings = useStore((s) => s.loadSettings);

  useEffect(() => {
    void loadConversations();
    void loadLabels();
    void loadSettings();
    connectRealtime();
  }, [loadConversations, loadLabels, loadSettings]);

  // Refresco de respaldo: si el WebSocket cae, la lista no se queda congelada.
  useEffect(() => {
    const id = setInterval(() => {
      if (!useStore.getState().connected) void loadConversations();
    }, 30_000);
    return () => clearInterval(id);
  }, [loadConversations]);

  return (
    <div className="flex h-full overflow-hidden">
      <NavRail />
      {view === 'chats' && <ChatsView />}
      {view === 'contacts' && <ContactsView />}
      {view === 'labels' && <LabelsView />}
      {view === 'broadcasts' && <BroadcastsView />}
      {view === 'catalog' && <CatalogView />}
      {view === 'autoreplies' && <AutoRepliesView />}
      {view === 'settings' && <SettingsView />}
      <Toasts />
    </div>
  );
}
