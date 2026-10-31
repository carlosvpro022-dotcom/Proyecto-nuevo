import { useState } from 'react';
import { Plus, Trash2 } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';

const PRESET_COLORS = ['#25d366', '#34b7f1', '#f6b23c', '#e8655b', '#a78bfa', '#8696a0'];

export function LabelsView() {
  const labels = useStore((s) => s.labels);
  const loadLabels = useStore((s) => s.loadLabels);
  const reloadChats = useStore((s) => s.loadConversations);
  const toast = useStore((s) => s.toast);

  const [name, setName] = useState('');
  const [color, setColor] = useState(PRESET_COLORS[0]);

  async function create() {
    if (!name.trim()) return;
    try {
      await api.createLabel(name.trim(), color);
      setName('');
      await loadLabels();
      toast('Etiqueta creada', 'success');
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  async function remove(id: number, labelName: string) {
    if (!confirm(`¿Eliminar la etiqueta "${labelName}"? Se quitará de todos los contactos.`)) return;
    try {
      await api.deleteLabel(id);
      await loadLabels();
      await reloadChats();
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  async function recolor(id: number, newColor: string) {
    try {
      await api.updateLabel(id, { color: newColor });
      await loadLabels();
      await reloadChats();
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  return (
    <section className="flex min-w-0 flex-1 flex-col bg-wa-bg">
      <header className="border-b border-wa-border bg-wa-surface px-6 py-4">
        <h1 className="text-lg">Etiquetas</h1>
        <p className="mt-0.5 text-xs text-wa-muted">Clasifica contactos para filtrar chats y segmentar envíos.</p>
      </header>

      <div className="min-h-0 flex-1 space-y-6 overflow-y-auto p-6">
        <div className="flex flex-wrap items-end gap-3 rounded-lg bg-wa-panel p-4">
          <label className="min-w-48 flex-1">
            <span className="mb-1.5 block text-xs text-wa-muted">Nueva etiqueta</span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && void create()}
              placeholder="Cliente VIP"
              className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
            />
          </label>
          <div>
            <span className="mb-1.5 block text-xs text-wa-muted">Color</span>
            <div className="flex gap-1.5">
              {PRESET_COLORS.map((c) => (
                <button
                  key={c}
                  onClick={() => setColor(c)}
                  aria-label={`Color ${c}`}
                  className={`h-8 w-8 rounded-full transition ${color === c ? 'ring-2 ring-white ring-offset-2 ring-offset-wa-panel' : ''}`}
                  style={{ background: c }}
                />
              ))}
            </div>
          </div>
          <button
            onClick={() => void create()}
            className="flex items-center gap-2 rounded-lg bg-wa-accent px-4 py-2 text-sm font-medium text-black transition hover:brightness-110"
          >
            <Plus className="h-4 w-4" /> Crear
          </button>
        </div>

        <ul className="space-y-2">
          {labels.map((l) => (
            <li key={l.id} className="flex items-center gap-3 rounded-lg bg-wa-panel px-4 py-3">
              <span className="h-4 w-4 shrink-0 rounded-full" style={{ background: l.color }} />
              <span className="flex-1 text-sm">{l.name}</span>
              <span className="text-xs text-wa-muted">{l.contact_count ?? 0} contacto(s)</span>
              <div className="flex gap-1">
                {PRESET_COLORS.map((c) => (
                  <button
                    key={c}
                    onClick={() => void recolor(l.id, c)}
                    aria-label={`Cambiar a ${c}`}
                    className="h-4 w-4 rounded-full opacity-60 transition hover:opacity-100"
                    style={{ background: c }}
                  />
                ))}
              </div>
              <button
                onClick={() => void remove(l.id, l.name)}
                aria-label="Eliminar etiqueta"
                className="rounded p-1.5 text-wa-muted transition hover:bg-wa-surface-2 hover:text-red-400"
              >
                <Trash2 className="h-4 w-4" />
              </button>
            </li>
          ))}
          {labels.length === 0 && <p className="text-sm text-wa-muted">Todavía no hay etiquetas.</p>}
        </ul>
      </div>
    </section>
  );
}
