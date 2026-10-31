import { useEffect, useState } from 'react';
import { Ban, Check, Save, X } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';
import { formatWindowLeft } from '../lib/format';
import { Avatar } from './Avatar';
import type { Conversation } from '../types';

export function ContactPanel({ conv, onClose }: { conv: Conversation; onClose: () => void }) {
  const labels = useStore((s) => s.labels);
  const reload = useStore((s) => s.loadConversations);
  const toast = useStore((s) => s.toast);

  const [name, setName] = useState(conv.displayName ?? '');
  const [notes, setNotes] = useState(conv.notes ?? '');
  const [selected, setSelected] = useState<number[]>(conv.labels.map((l) => l.id));
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setName(conv.displayName ?? '');
    setNotes(conv.notes ?? '');
    setSelected(conv.labels.map((l) => l.id));
  }, [conv.id, conv.displayName, conv.notes, conv.labels]);

  async function save() {
    setSaving(true);
    try {
      await api.updateContact(conv.waId, { displayName: name.trim() || null, notes: notes.trim() || null });
      await api.setContactLabels(conv.waId, selected);
      await reload();
      toast('Contacto actualizado', 'success');
    } catch (err) {
      toast((err as Error).message, 'error');
    } finally {
      setSaving(false);
    }
  }

  async function toggleBlock() {
    try {
      await api.updateContact(conv.waId, { isBlocked: !conv.isBlocked });
      await reload();
      toast(conv.isBlocked ? 'Contacto desbloqueado en el panel' : 'Contacto marcado como bloqueado en el panel');
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  return (
    <aside className="flex w-[340px] shrink-0 flex-col border-l border-wa-border bg-wa-panel">
      <header className="flex items-center gap-4 border-b border-wa-border px-4 py-3.5">
        <button onClick={onClose} aria-label="Cerrar panel" className="text-wa-muted hover:text-wa-text">
          <X className="h-5 w-5" />
        </button>
        <h2 className="text-base">Datos del contacto</h2>
      </header>

      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-5">
        <div className="flex flex-col items-center gap-2 text-center">
          <Avatar name={conv.name} waId={conv.waId} size={96} />
          <p className="text-lg">{conv.name}</p>
          <p className="text-sm text-wa-muted">+{conv.waId}</p>
          {conv.profileName && conv.profileName !== conv.name && (
            <p className="text-xs text-wa-muted">Nombre en WhatsApp: {conv.profileName}</p>
          )}
        </div>

        <div className={`rounded-lg p-3 text-xs ${conv.windowOpen ? 'bg-wa-accent/10 text-wa-accent' : 'bg-amber-500/10 text-amber-200'}`}>
          <p className="font-medium">Ventana de servicio de 24 h</p>
          <p className="mt-1 opacity-90">
            {conv.windowOpen
              ? `Abierta. Queda ${formatWindowLeft(conv.windowClosesAt)} para enviar mensajes libres.`
              : 'Cerrada. Solo se pueden enviar plantillas aprobadas hasta que el contacto escriba de nuevo.'}
          </p>
        </div>

        <label className="block">
          <span className="mb-1.5 block text-xs text-wa-muted">Nombre guardado</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={conv.profileName ?? 'Sin nombre'}
            className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
          />
        </label>

        <div>
          <span className="mb-2 block text-xs text-wa-muted">Etiquetas</span>
          <div className="flex flex-wrap gap-1.5">
            {labels.map((l) => {
              const on = selected.includes(l.id);
              return (
                <button
                  key={l.id}
                  onClick={() => setSelected((s) => (on ? s.filter((i) => i !== l.id) : [...s, l.id]))}
                  className="flex items-center gap-1.5 rounded-full px-3 py-1 text-xs transition"
                  style={{
                    background: on ? `${l.color}33` : 'var(--color-wa-surface-2)',
                    color: on ? l.color : 'var(--color-wa-muted)',
                  }}
                >
                  {on && <Check className="h-3 w-3" />}
                  {l.name}
                </button>
              );
            })}
            {labels.length === 0 && <p className="text-xs text-wa-muted">Crea etiquetas desde la sección Etiquetas.</p>}
          </div>
        </div>

        <label className="block">
          <span className="mb-1.5 block text-xs text-wa-muted">Notas internas</span>
          <textarea
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            rows={5}
            placeholder="Pedido pendiente, preferencias, incidencias…"
            className="w-full resize-y rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
          />
        </label>

        <button
          onClick={() => void save()}
          disabled={saving}
          className="flex w-full items-center justify-center gap-2 rounded-lg bg-wa-accent py-2.5 text-sm font-medium text-black transition hover:brightness-110 disabled:opacity-50"
        >
          <Save className="h-4 w-4" />
          {saving ? 'Guardando…' : 'Guardar cambios'}
        </button>

        <button
          onClick={() => void toggleBlock()}
          className={`flex w-full items-center justify-center gap-2 rounded-lg py-2.5 text-sm transition ${
            conv.isBlocked ? 'bg-wa-surface-2 text-wa-text' : 'bg-red-500/10 text-red-300 hover:bg-red-500/20'
          }`}
        >
          <Ban className="h-4 w-4" />
          {conv.isBlocked ? 'Quitar marca de bloqueado' : 'Marcar como bloqueado'}
        </button>
        <p className="text-[11px] leading-relaxed text-wa-muted">
          El bloqueo es una marca interna de este panel. La Cloud API no permite bloquear números en WhatsApp.
        </p>
      </div>
    </aside>
  );
}
