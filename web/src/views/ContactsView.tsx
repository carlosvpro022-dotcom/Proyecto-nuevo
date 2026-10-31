import { useEffect, useState } from 'react';
import { MessageSquare, Search } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';
import { Avatar } from '../components/Avatar';
import type { Contact } from '../types';

export function ContactsView() {
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [query, setQuery] = useState('');
  const [labelId, setLabelId] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);

  const labels = useStore((s) => s.labels);
  const setView = useStore((s) => s.setView);
  const select = useStore((s) => s.selectConversation);
  const openConv = useStore((s) => s.loadConversations);
  const toast = useStore((s) => s.toast);

  useEffect(() => {
    const id = setTimeout(() => {
      setLoading(true);
      api
        .contacts({ q: query || undefined, labelId: labelId ?? undefined })
        .then((r) => setContacts(r.contacts))
        .catch((e) => toast((e as Error).message, 'error'))
        .finally(() => setLoading(false));
    }, 200);
    return () => clearTimeout(id);
  }, [query, labelId, toast]);

  async function openChat(contact: Contact) {
    try {
      const conv = contact.conversationId
        ? { id: contact.conversationId }
        : await api.openConversation(contact.waId);
      await openConv();
      setView('chats');
      await select(conv.id);
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  return (
    <section className="flex min-w-0 flex-1 flex-col bg-wa-bg">
      <header className="border-b border-wa-border bg-wa-surface px-6 py-4">
        <h1 className="text-lg">Contactos</h1>
        <p className="mt-0.5 text-xs text-wa-muted">
          Se crean solos cuando alguien escribe a tu número, o al abrir un chat nuevo.
        </p>
      </header>

      <div className="flex flex-wrap items-center gap-2 border-b border-wa-border px-6 py-3">
        <div className="flex min-w-60 flex-1 items-center gap-2 rounded-lg bg-wa-surface px-3">
          <Search className="h-4 w-4 text-wa-muted" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Buscar por nombre o número"
            className="w-full bg-transparent py-2 text-sm placeholder:text-wa-muted"
          />
        </div>
        <button
          onClick={() => setLabelId(null)}
          className={`rounded-full px-3 py-1 text-xs ${labelId === null ? 'bg-wa-accent/20 text-wa-accent' : 'bg-wa-surface text-wa-muted'}`}
        >
          Todas
        </button>
        {labels.map((l) => (
          <button
            key={l.id}
            onClick={() => setLabelId(labelId === l.id ? null : l.id)}
            className="rounded-full px-3 py-1 text-xs"
            style={{
              background: labelId === l.id ? `${l.color}33` : 'var(--color-wa-surface)',
              color: labelId === l.id ? l.color : 'var(--color-wa-muted)',
            }}
          >
            {l.name}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading && <p className="p-6 text-sm text-wa-muted">Cargando…</p>}
        {!loading && contacts.length === 0 && <p className="p-6 text-sm text-wa-muted">No hay contactos todavía.</p>}

        <ul className="divide-y divide-wa-border/60">
          {contacts.map((c) => (
            <li key={c.waId} className="flex items-center gap-3 px-6 py-3 transition hover:bg-wa-surface/50">
              <Avatar name={c.name} waId={c.waId} size={42} />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm">{c.name}</p>
                <p className="text-xs text-wa-muted">+{c.waId}</p>
                {c.notes && <p className="mt-0.5 truncate text-xs text-wa-muted/80 italic">{c.notes}</p>}
              </div>
              <div className="flex shrink-0 flex-wrap justify-end gap-1">
                {c.labels.map((l) => (
                  <span key={l.id} className="rounded-full px-2 py-0.5 text-[11px]" style={{ background: `${l.color}2a`, color: l.color }}>
                    {l.name}
                  </span>
                ))}
              </div>
              <button
                onClick={() => void openChat(c)}
                title="Abrir chat"
                className="shrink-0 rounded-full p-2 text-wa-muted transition hover:bg-wa-surface-2 hover:text-wa-accent"
              >
                <MessageSquare className="h-4 w-4" />
              </button>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
