import { useEffect, useMemo, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';
import { Modal } from './Modal';
import type { Template } from '../types';

/** Extrae los {{1}}, {{2}}… de una plantilla para pedir sus valores. */
function placeholders(text: string): number[] {
  const found = new Set<number>();
  for (const m of text.matchAll(/\{\{(\d+)\}\}/g)) found.add(Number(m[1]));
  return [...found].sort((a, b) => a - b);
}

function bodyText(t: Template): string {
  return t.components?.find((c) => c.type === 'BODY')?.text ?? '';
}

function headerText(t: Template): string {
  const h = t.components?.find((c) => c.type === 'HEADER');
  return h?.format === 'TEXT' ? (h.text ?? '') : '';
}

export function TemplatePicker({
  conversationId,
  onClose,
}: {
  conversationId: number;
  onClose: () => void;
}) {
  const [templates, setTemplates] = useState<Template[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Template | null>(null);
  const [bodyVars, setBodyVars] = useState<Record<number, string>>({});
  const [headerVars, setHeaderVars] = useState<Record<number, string>>({});
  const [sending, setSending] = useState(false);
  const [query, setQuery] = useState('');
  const toast = useStore((s) => s.toast);

  async function load(refresh = false) {
    setLoading(true);
    setError(null);
    try {
      const res = await api.templates(refresh);
      setTemplates(res.templates);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  const filtered = useMemo(
    () => templates.filter((t) => t.name.toLowerCase().includes(query.toLowerCase())),
    [templates, query]
  );

  const bodyParams = selected ? placeholders(bodyText(selected)) : [];
  const headerParams = selected ? placeholders(headerText(selected)) : [];

  async function send() {
    if (!selected) return;
    const missing = [...bodyParams.filter((n) => !bodyVars[n]?.trim()), ...headerParams.filter((n) => !headerVars[n]?.trim())];
    if (missing.length) {
      toast('Rellena todas las variables de la plantilla', 'error');
      return;
    }

    const components: unknown[] = [];
    if (headerParams.length) {
      components.push({
        type: 'header',
        parameters: headerParams.map((n) => ({ type: 'text', text: headerVars[n] })),
      });
    }
    if (bodyParams.length) {
      components.push({
        type: 'body',
        parameters: bodyParams.map((n) => ({ type: 'text', text: bodyVars[n] })),
      });
    }

    setSending(true);
    try {
      await api.sendTemplate(conversationId, selected.name, selected.language, components);
      toast('Plantilla enviada', 'success');
      onClose();
    } catch (err) {
      toast((err as Error).message, 'error');
    } finally {
      setSending(false);
    }
  }

  const preview = selected
    ? bodyText(selected).replace(/\{\{(\d+)\}\}/g, (_, n) => bodyVars[Number(n)] || `{{${n}}}`)
    : '';

  return (
    <Modal title="Enviar plantilla" onClose={onClose} wide>
      {loading && <p className="py-8 text-center text-sm text-wa-muted">Cargando plantillas desde Meta…</p>}

      {error && (
        <div className="rounded-lg bg-red-500/10 p-4 text-sm text-red-300">
          <p className="font-medium">No se pudieron cargar las plantillas</p>
          <p className="mt-1 text-xs text-red-300/80">{error}</p>
          <p className="mt-2 text-xs text-red-300/70">
            Revisa que WHATSAPP_BUSINESS_ACCOUNT_ID esté configurado y que el token tenga permiso
            <code className="mx-1 rounded bg-black/30 px-1">whatsapp_business_management</code>.
          </p>
        </div>
      )}

      {!loading && !error && (
        <div className="grid gap-4 md:grid-cols-[1fr_1.1fr]">
          <div className="flex min-h-0 flex-col">
            <div className="mb-2 flex gap-2">
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Buscar plantilla"
                className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
              />
              <button
                onClick={() => void load(true)}
                title="Recargar desde Meta"
                className="rounded-lg bg-wa-surface-2 px-2.5 text-wa-muted hover:text-wa-text"
              >
                <RefreshCw className="h-4 w-4" />
              </button>
            </div>

            <div className="max-h-80 space-y-1 overflow-y-auto pr-1">
              {filtered.length === 0 && (
                <p className="py-6 text-center text-xs text-wa-muted">
                  No hay plantillas aprobadas. Créalas en el Administrador de WhatsApp de Meta.
                </p>
              )}
              {filtered.map((t) => (
                <button
                  key={`${t.id}-${t.language}`}
                  onClick={() => {
                    setSelected(t);
                    setBodyVars({});
                    setHeaderVars({});
                  }}
                  className={`w-full rounded-lg px-3 py-2 text-left transition ${
                    selected?.id === t.id && selected.language === t.language
                      ? 'bg-wa-accent/20 text-wa-text'
                      : 'bg-wa-surface hover:bg-wa-surface-2'
                  }`}
                >
                  <span className="block truncate text-sm">{t.name}</span>
                  <span className="text-[11px] text-wa-muted">
                    {t.language} · {t.category}
                  </span>
                </button>
              ))}
            </div>
          </div>

          <div>
            {!selected && <p className="py-8 text-center text-sm text-wa-muted">Elige una plantilla para previsualizarla.</p>}

            {selected && (
              <div className="space-y-3">
                <div className="rounded-lg bg-wa-out/60 p-3 text-sm whitespace-pre-wrap">
                  {headerText(selected) && (
                    <p className="mb-1.5 font-semibold">
                      {headerText(selected).replace(/\{\{(\d+)\}\}/g, (_, n) => headerVars[Number(n)] || `{{${n}}}`)}
                    </p>
                  )}
                  {preview || <span className="text-wa-muted">(sin cuerpo de texto)</span>}
                </div>

                {headerParams.map((n) => (
                  <label key={`h${n}`} className="block">
                    <span className="mb-1 block text-xs text-wa-muted">Variable de cabecera {`{{${n}}}`}</span>
                    <input
                      value={headerVars[n] ?? ''}
                      onChange={(e) => setHeaderVars((v) => ({ ...v, [n]: e.target.value }))}
                      className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm"
                    />
                  </label>
                ))}

                {bodyParams.map((n) => (
                  <label key={`b${n}`} className="block">
                    <span className="mb-1 block text-xs text-wa-muted">Variable {`{{${n}}}`}</span>
                    <input
                      value={bodyVars[n] ?? ''}
                      onChange={(e) => setBodyVars((v) => ({ ...v, [n]: e.target.value }))}
                      className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm"
                    />
                  </label>
                ))}

                <button
                  onClick={() => void send()}
                  disabled={sending}
                  className="w-full rounded-lg bg-wa-accent py-2.5 text-sm font-medium text-black transition hover:brightness-110 disabled:opacity-50"
                >
                  {sending ? 'Enviando…' : 'Enviar plantilla'}
                </button>
              </div>
            )}
          </div>
        </div>
      )}
    </Modal>
  );
}
