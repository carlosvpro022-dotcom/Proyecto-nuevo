import { useEffect, useState } from 'react';
import { Megaphone, Plus, Send, Trash2, Users } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';
import { Modal } from '../components/Modal';
import type { BroadcastList, Contact, Template } from '../types';

export function BroadcastsView() {
  const [lists, setLists] = useState<BroadcastList[]>([]);
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState<BroadcastList | null>(null);
  const [sendingTo, setSendingTo] = useState<BroadcastList | null>(null);
  const toast = useStore((s) => s.toast);

  async function reload() {
    try {
      const [l, c] = await Promise.all([api.broadcasts(), api.contacts()]);
      setLists(l.lists);
      setContacts(c.contacts);
    } catch (err) {
      toast((err as Error).message, 'error');
    }
    api.templates().then((r) => setTemplates(r.templates)).catch(() => undefined);
  }

  useEffect(() => {
    void reload();
  }, []);

  return (
    <section className="flex min-w-0 flex-1 flex-col bg-wa-bg">
      <header className="flex items-start justify-between gap-4 border-b border-wa-border bg-wa-surface px-6 py-4">
        <div>
          <h1 className="text-lg">Listas de difusión</h1>
          <p className="mt-0.5 max-w-2xl text-xs text-wa-muted">
            La Cloud API oficial no da acceso a grupos de WhatsApp. Estas listas envían el mismo mensaje a cada
            miembro como conversación individual, igual que una difusión.
          </p>
        </div>
        <button
          onClick={() => setCreating(true)}
          className="flex shrink-0 items-center gap-2 rounded-lg bg-wa-accent px-4 py-2 text-sm font-medium text-black transition hover:brightness-110"
        >
          <Plus className="h-4 w-4" /> Nueva lista
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-6">
        {lists.length === 0 && (
          <div className="mx-auto max-w-md rounded-lg bg-wa-panel p-8 text-center">
            <Megaphone className="mx-auto h-10 w-10 text-wa-muted" />
            <p className="mt-3 text-sm text-wa-muted">Todavía no has creado ninguna lista.</p>
          </div>
        )}

        <ul className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {lists.map((l) => (
            <li key={l.id} className="rounded-lg bg-wa-panel p-4">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">{l.name}</p>
                  <p className="flex items-center gap-1.5 text-xs text-wa-muted">
                    <Users className="h-3.5 w-3.5" /> {l.members.length} destinatario(s)
                  </p>
                </div>
                <button
                  onClick={async () => {
                    if (!confirm(`¿Eliminar la lista "${l.name}"?`)) return;
                    await api.deleteBroadcast(l.id);
                    void reload();
                  }}
                  className="rounded p-1.5 text-wa-muted hover:text-red-400"
                  aria-label="Eliminar lista"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </div>

              <div className="mt-3 flex gap-2">
                <button
                  onClick={() => setEditing(l)}
                  className="flex-1 rounded-lg bg-wa-surface-2 py-2 text-xs transition hover:bg-wa-surface"
                >
                  Miembros
                </button>
                <button
                  onClick={() => setSendingTo(l)}
                  disabled={l.members.length === 0}
                  className="flex flex-1 items-center justify-center gap-1.5 rounded-lg bg-wa-accent py-2 text-xs font-medium text-black transition hover:brightness-110 disabled:opacity-40"
                >
                  <Send className="h-3.5 w-3.5" /> Enviar
                </button>
              </div>
            </li>
          ))}
        </ul>
      </div>

      {creating && (
        <CreateListDialog
          contacts={contacts}
          onClose={() => setCreating(false)}
          onDone={() => {
            setCreating(false);
            void reload();
          }}
        />
      )}

      {editing && (
        <MembersDialog
          list={editing}
          contacts={contacts}
          onClose={() => setEditing(null)}
          onDone={() => {
            setEditing(null);
            void reload();
          }}
        />
      )}

      {sendingTo && <SendDialog list={sendingTo} templates={templates} onClose={() => setSendingTo(null)} />}
    </section>
  );
}

function ContactChecklist({
  contacts,
  selected,
  onToggle,
}: {
  contacts: Contact[];
  selected: string[];
  onToggle: (waId: string) => void;
}) {
  const [q, setQ] = useState('');
  const filtered = contacts.filter(
    (c) => c.name.toLowerCase().includes(q.toLowerCase()) || c.waId.includes(q)
  );
  return (
    <div>
      <input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Filtrar contactos"
        className="mb-2 w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
      />
      <div className="max-h-64 space-y-1 overflow-y-auto pr-1">
        {filtered.map((c) => (
          <label key={c.waId} className="flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2 hover:bg-wa-surface">
            <input
              type="checkbox"
              checked={selected.includes(c.waId)}
              onChange={() => onToggle(c.waId)}
              className="h-4 w-4 accent-[#00a884]"
            />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm">{c.name}</span>
              <span className="text-xs text-wa-muted">+{c.waId}</span>
            </span>
          </label>
        ))}
        {filtered.length === 0 && <p className="py-4 text-center text-xs text-wa-muted">Sin resultados.</p>}
      </div>
    </div>
  );
}

function CreateListDialog({
  contacts,
  onClose,
  onDone,
}: {
  contacts: Contact[];
  onClose: () => void;
  onDone: () => void;
}) {
  const [name, setName] = useState('');
  const [members, setMembers] = useState<string[]>([]);
  const toast = useStore((s) => s.toast);

  return (
    <Modal title="Nueva lista de difusión" onClose={onClose}>
      <div className="space-y-4">
        <input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Nombre de la lista"
          className="w-full rounded-lg bg-wa-surface-2 px-3 py-2.5 text-sm placeholder:text-wa-muted"
        />
        <ContactChecklist
          contacts={contacts}
          selected={members}
          onToggle={(waId) => setMembers((m) => (m.includes(waId) ? m.filter((x) => x !== waId) : [...m, waId]))}
        />
        <button
          onClick={async () => {
            if (!name.trim()) return toast('Pon un nombre a la lista', 'error');
            try {
              await api.createBroadcast(name.trim(), members);
              onDone();
            } catch (err) {
              toast((err as Error).message, 'error');
            }
          }}
          className="w-full rounded-lg bg-wa-accent py-2.5 text-sm font-medium text-black transition hover:brightness-110"
        >
          Crear lista ({members.length})
        </button>
      </div>
    </Modal>
  );
}

function MembersDialog({
  list,
  contacts,
  onClose,
  onDone,
}: {
  list: BroadcastList;
  contacts: Contact[];
  onClose: () => void;
  onDone: () => void;
}) {
  const [members, setMembers] = useState<string[]>(list.members);
  const toast = useStore((s) => s.toast);

  return (
    <Modal title={`Miembros de ${list.name}`} onClose={onClose}>
      <div className="space-y-4">
        <ContactChecklist
          contacts={contacts}
          selected={members}
          onToggle={(waId) => setMembers((m) => (m.includes(waId) ? m.filter((x) => x !== waId) : [...m, waId]))}
        />
        <button
          onClick={async () => {
            try {
              await api.setBroadcastMembers(list.id, members);
              onDone();
            } catch (err) {
              toast((err as Error).message, 'error');
            }
          }}
          className="w-full rounded-lg bg-wa-accent py-2.5 text-sm font-medium text-black transition hover:brightness-110"
        >
          Guardar ({members.length})
        </button>
      </div>
    </Modal>
  );
}

function SendDialog({ list, templates, onClose }: { list: BroadcastList; templates: Template[]; onClose: () => void }) {
  const [mode, setMode] = useState<'template' | 'text'>('template');
  const [text, setText] = useState('');
  const [templateKey, setTemplateKey] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ sent: number; failed: number } | null>(null);
  const toast = useStore((s) => s.toast);

  async function send() {
    setBusy(true);
    try {
      let payload: { text?: string; template?: { name: string; language: string } };
      if (mode === 'template') {
        const t = templates.find((x) => `${x.name}|${x.language}` === templateKey);
        if (!t) {
          toast('Elige una plantilla', 'error');
          setBusy(false);
          return;
        }
        payload = { template: { name: t.name, language: t.language } };
      } else {
        if (!text.trim()) {
          toast('Escribe el mensaje', 'error');
          setBusy(false);
          return;
        }
        payload = { text: text.trim() };
      }
      const res = await api.sendBroadcast(list.id, payload);
      setResult(res);
      toast(`Enviados ${res.sent}, fallidos ${res.failed}`, res.failed ? 'error' : 'success');
    } catch (err) {
      toast((err as Error).message, 'error');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={`Enviar a ${list.name}`} onClose={onClose}>
      <div className="space-y-4">
        <p className="rounded-lg bg-wa-surface/60 p-3 text-xs leading-relaxed text-wa-muted">
          {list.members.length} destinatario(s). Para los contactos que no te hayan escrito en las últimas 24 h,
          solo llegará el envío si usas una <strong className="text-wa-text">plantilla aprobada</strong>.
        </p>

        <div className="flex gap-2">
          {(['template', 'text'] as const).map((m) => (
            <button
              key={m}
              onClick={() => setMode(m)}
              className={`flex-1 rounded-lg py-2 text-xs transition ${
                mode === m ? 'bg-wa-accent/20 text-wa-accent' : 'bg-wa-surface-2 text-wa-muted'
              }`}
            >
              {m === 'template' ? 'Plantilla (recomendado)' : 'Texto libre'}
            </button>
          ))}
        </div>

        {mode === 'template' ? (
          <select
            value={templateKey}
            onChange={(e) => setTemplateKey(e.target.value)}
            className="w-full rounded-lg bg-wa-surface-2 px-3 py-2.5 text-sm"
          >
            <option value="">Elige una plantilla…</option>
            {templates.map((t) => (
              <option key={`${t.id}-${t.language}`} value={`${t.name}|${t.language}`}>
                {t.name} ({t.language})
              </option>
            ))}
          </select>
        ) : (
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            rows={4}
            placeholder="Mensaje para los destinatarios con ventana abierta"
            className="w-full resize-y rounded-lg bg-wa-surface-2 px-3 py-2.5 text-sm placeholder:text-wa-muted"
          />
        )}

        {result && (
          <p className="rounded-lg bg-wa-surface p-3 text-xs">
            Enviados: <strong className="text-wa-accent">{result.sent}</strong> · Fallidos:{' '}
            <strong className={result.failed ? 'text-red-400' : ''}>{result.failed}</strong>
          </p>
        )}

        <button
          onClick={() => void send()}
          disabled={busy}
          className="w-full rounded-lg bg-wa-accent py-2.5 text-sm font-medium text-black transition hover:brightness-110 disabled:opacity-50"
        >
          {busy ? 'Enviando…' : 'Enviar ahora'}
        </button>
      </div>
    </Modal>
  );
}
