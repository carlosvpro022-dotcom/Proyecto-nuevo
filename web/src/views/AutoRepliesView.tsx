import { useEffect, useState } from 'react';
import { Bot, Plus, Trash2 } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';
import { Modal } from '../components/Modal';
import { hhmmToMinutes, minutesToHHMM } from '../lib/format';
import type { AutoReplyRule } from '../types';

const KIND_LABEL: Record<AutoReplyRule['kind'], string> = {
  welcome: 'Bienvenida',
  away: 'Fuera de horario',
  keyword: 'Palabra clave',
};

const KIND_HELP: Record<AutoReplyRule['kind'], string> = {
  welcome: 'Se envía una sola vez, en el primer mensaje que te escribe un contacto nuevo.',
  away: 'Se envía cuando llega un mensaje dentro del horario indicado. Como mucho, una vez por hora y contacto.',
  keyword: 'Se envía cuando el mensaje contiene la palabra indicada. Como mucho, una vez cada 5 minutos y contacto.',
};

export function AutoRepliesView() {
  const [rules, setRules] = useState<AutoReplyRule[]>([]);
  const [editing, setEditing] = useState<AutoReplyRule | 'new' | null>(null);
  const toast = useStore((s) => s.toast);

  async function reload() {
    try {
      setRules((await api.autoReplies()).rules);
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  useEffect(() => {
    void reload();
  }, []);

  async function toggle(rule: AutoReplyRule) {
    try {
      await api.updateAutoReply(rule.id, { enabled: rule.enabled !== 1 });
      void reload();
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  return (
    <section className="flex min-w-0 flex-1 flex-col bg-wa-bg">
      <header className="flex items-start justify-between gap-4 border-b border-wa-border bg-wa-surface px-6 py-4">
        <div>
          <h1 className="text-lg">Respuestas automáticas</h1>
          <p className="mt-0.5 max-w-2xl text-xs text-wa-muted">
            Se disparan solo con mensajes entrantes y siempre dentro de la ventana de 24 h, así que no consumen
            plantillas. Orden de prioridad: palabra clave → bienvenida → fuera de horario.
          </p>
        </div>
        <button
          onClick={() => setEditing('new')}
          className="flex shrink-0 items-center gap-2 rounded-lg bg-wa-accent px-4 py-2 text-sm font-medium text-black transition hover:brightness-110"
        >
          <Plus className="h-4 w-4" /> Nueva regla
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-6">
        {rules.length === 0 && (
          <div className="mx-auto max-w-md rounded-lg bg-wa-panel p-8 text-center">
            <Bot className="mx-auto h-10 w-10 text-wa-muted" />
            <p className="mt-3 text-sm text-wa-muted">No hay reglas configuradas.</p>
          </div>
        )}

        <ul className="space-y-3">
          {rules.map((r) => (
            <li key={r.id} className="rounded-lg bg-wa-panel p-4">
              <div className="flex flex-wrap items-start gap-3">
                <label className="mt-0.5 inline-flex shrink-0 cursor-pointer items-center">
                  <input
                    type="checkbox"
                    checked={r.enabled === 1}
                    onChange={() => void toggle(r)}
                    className="h-4 w-4 accent-[#00a884]"
                  />
                </label>

                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-medium">{r.name}</span>
                    <span className="rounded-full bg-wa-surface-2 px-2 py-0.5 text-[11px] text-wa-muted">
                      {KIND_LABEL[r.kind]}
                    </span>
                    {r.kind === 'keyword' && r.keyword && (
                      <span className="rounded-full bg-wa-blue/15 px-2 py-0.5 text-[11px] text-wa-blue">
                        «{r.keyword}»
                      </span>
                    )}
                    {r.kind === 'away' && r.active_from !== null && (
                      <span className="rounded-full bg-amber-400/15 px-2 py-0.5 text-[11px] text-amber-300">
                        {minutesToHHMM(r.active_from)} – {minutesToHHMM(r.active_to)}
                      </span>
                    )}
                    {r.enabled !== 1 && <span className="text-[11px] text-wa-muted">(desactivada)</span>}
                  </div>
                  <p className="mt-1.5 text-xs whitespace-pre-wrap text-wa-muted">{r.reply_text}</p>
                </div>

                <div className="flex shrink-0 gap-1">
                  <button
                    onClick={() => setEditing(r)}
                    className="rounded-lg bg-wa-surface-2 px-3 py-1.5 text-xs transition hover:bg-wa-surface"
                  >
                    Editar
                  </button>
                  <button
                    onClick={async () => {
                      if (!confirm(`¿Eliminar la regla "${r.name}"?`)) return;
                      await api.deleteAutoReply(r.id);
                      void reload();
                    }}
                    aria-label="Eliminar regla"
                    className="rounded-lg p-1.5 text-wa-muted transition hover:text-red-400"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      </div>

      {editing && (
        <RuleDialog
          rule={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
          onDone={() => {
            setEditing(null);
            void reload();
          }}
        />
      )}
    </section>
  );
}

function RuleDialog({
  rule,
  onClose,
  onDone,
}: {
  rule: AutoReplyRule | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const [name, setName] = useState(rule?.name ?? '');
  const [kind, setKind] = useState<AutoReplyRule['kind']>(rule?.kind ?? 'keyword');
  const [keyword, setKeyword] = useState(rule?.keyword ?? '');
  const [matchType, setMatchType] = useState<AutoReplyRule['match_type']>(rule?.match_type ?? 'contains');
  const [replyText, setReplyText] = useState(rule?.reply_text ?? '');
  const [from, setFrom] = useState(minutesToHHMM(rule?.active_from ?? null) || '20:00');
  const [to, setTo] = useState(minutesToHHMM(rule?.active_to ?? null) || '09:00');
  const [priority, setPriority] = useState(rule?.priority ?? 0);
  const [busy, setBusy] = useState(false);
  const toast = useStore((s) => s.toast);

  async function save() {
    if (!name.trim() || !replyText.trim()) {
      toast('El nombre y el texto de respuesta son obligatorios', 'error');
      return;
    }
    if (kind === 'keyword' && !keyword.trim()) {
      toast('Indica la palabra clave', 'error');
      return;
    }

    const payload: Record<string, unknown> = {
      name: name.trim(),
      kind,
      replyText: replyText.trim(),
      matchType,
      priority,
      keyword: kind === 'keyword' ? keyword.trim() : null,
      activeFrom: kind === 'away' ? hhmmToMinutes(from) : null,
      activeTo: kind === 'away' ? hhmmToMinutes(to) : null,
      enabled: rule ? rule.enabled === 1 : true,
    };

    setBusy(true);
    try {
      if (rule) await api.updateAutoReply(rule.id, payload);
      else await api.createAutoReply(payload);
      onDone();
    } catch (err) {
      toast((err as Error).message, 'error');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={rule ? 'Editar regla' : 'Nueva regla'} onClose={onClose}>
      <div className="space-y-4">
        <label className="block">
          <span className="mb-1.5 block text-xs text-wa-muted">Nombre</span>
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Saludo inicial"
            className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
          />
        </label>

        <div>
          <span className="mb-1.5 block text-xs text-wa-muted">Tipo</span>
          <div className="flex gap-2">
            {(['keyword', 'welcome', 'away'] as const).map((k) => (
              <button
                key={k}
                onClick={() => setKind(k)}
                className={`flex-1 rounded-lg py-2 text-xs transition ${
                  kind === k ? 'bg-wa-accent/20 text-wa-accent' : 'bg-wa-surface-2 text-wa-muted'
                }`}
              >
                {KIND_LABEL[k]}
              </button>
            ))}
          </div>
          <p className="mt-1.5 text-[11px] text-wa-muted">{KIND_HELP[kind]}</p>
        </div>

        {kind === 'keyword' && (
          <div className="grid grid-cols-2 gap-3">
            <label className="block">
              <span className="mb-1.5 block text-xs text-wa-muted">Palabra clave</span>
              <input
                value={keyword}
                onChange={(e) => setKeyword(e.target.value)}
                placeholder="precio"
                className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
              />
            </label>
            <label className="block">
              <span className="mb-1.5 block text-xs text-wa-muted">Coincidencia</span>
              <select
                value={matchType}
                onChange={(e) => setMatchType(e.target.value as AutoReplyRule['match_type'])}
                className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm"
              >
                <option value="contains">Contiene</option>
                <option value="equals">Es exactamente</option>
                <option value="starts_with">Empieza por</option>
              </select>
            </label>
          </div>
        )}

        {kind === 'away' && (
          <div className="grid grid-cols-2 gap-3">
            <label className="block">
              <span className="mb-1.5 block text-xs text-wa-muted">Desde</span>
              <input
                type="time"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
                className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm"
              />
            </label>
            <label className="block">
              <span className="mb-1.5 block text-xs text-wa-muted">Hasta</span>
              <input
                type="time"
                value={to}
                onChange={(e) => setTo(e.target.value)}
                className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm"
              />
            </label>
            <p className="col-span-2 text-[11px] text-wa-muted">
              Si la hora de inicio es mayor que la de fin, el rango cruza la medianoche.
            </p>
          </div>
        )}

        <label className="block">
          <span className="mb-1.5 block text-xs text-wa-muted">Respuesta</span>
          <textarea
            value={replyText}
            onChange={(e) => setReplyText(e.target.value)}
            rows={4}
            placeholder="¡Hola! Gracias por escribirnos. Te respondemos en breve."
            className="w-full resize-y rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
          />
        </label>

        <label className="block">
          <span className="mb-1.5 block text-xs text-wa-muted">Prioridad (mayor gana dentro del mismo tipo)</span>
          <input
            type="number"
            min={0}
            max={100}
            value={priority}
            onChange={(e) => setPriority(Number(e.target.value))}
            className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm"
          />
        </label>

        <button
          onClick={() => void save()}
          disabled={busy}
          className="w-full rounded-lg bg-wa-accent py-2.5 text-sm font-medium text-black transition hover:brightness-110 disabled:opacity-50"
        >
          {busy ? 'Guardando…' : 'Guardar regla'}
        </button>
      </div>
    </Modal>
  );
}
