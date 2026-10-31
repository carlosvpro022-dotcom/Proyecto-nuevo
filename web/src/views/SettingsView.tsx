import { useEffect, useState } from 'react';
import { AlertTriangle, CheckCircle2, Copy, Info, KeyRound } from 'lucide-react';
import { api, getApiKey, setApiKey } from '../lib/api';
import { useStore, connectRealtime } from '../lib/store';
import type { Stats } from '../types';

function Toggle({
  label,
  help,
  checked,
  onChange,
}: {
  label: string;
  help?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-3 rounded-lg bg-wa-panel p-4">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="mt-0.5 h-4 w-4 accent-[#00a884]" />
      <span className="min-w-0">
        <span className="block text-sm">{label}</span>
        {help && <span className="mt-0.5 block text-xs text-wa-muted">{help}</span>}
      </span>
    </label>
  );
}

function StatCard({ label, value, tone }: { label: string; value: number; tone?: 'good' | 'bad' }) {
  return (
    <div className="rounded-lg bg-wa-panel p-4">
      <p className={`text-2xl ${tone === 'bad' && value > 0 ? 'text-red-400' : tone === 'good' ? 'text-wa-accent' : ''}`}>
        {value}
      </p>
      <p className="mt-0.5 text-xs text-wa-muted">{label}</p>
    </div>
  );
}

export function SettingsView() {
  const settings = useStore((s) => s.settings);
  const loadSettings = useStore((s) => s.loadSettings);
  const toast = useStore((s) => s.toast);
  const [stats, setStats] = useState<Stats | null>(null);
  const [keyInput, setKeyInput] = useState(getApiKey());
  const [profile, setProfile] = useState<Record<string, unknown> | null>(null);

  useEffect(() => {
    void loadSettings();
    api.stats().then(setStats).catch(() => undefined);
    api.profile().then((r) => setProfile(r.phone)).catch(() => undefined);
  }, [loadSettings]);

  async function update(key: string, value: string) {
    try {
      await api.saveSettings({ [key]: value });
      await loadSettings();
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  const values = settings?.settings ?? {};
  const webhookUrl = `${location.origin}/webhook`;

  return (
    <section className="flex min-w-0 flex-1 flex-col bg-wa-bg">
      <header className="border-b border-wa-border bg-wa-surface px-6 py-4">
        <h1 className="text-lg">Ajustes</h1>
      </header>

      <div className="min-h-0 flex-1 space-y-6 overflow-y-auto p-6">
        {/* Estado de la conexión */}
        <div className="rounded-lg bg-wa-panel p-5">
          <h2 className="mb-3 text-sm font-medium">Conexión con WhatsApp Business</h2>
          <dl className="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-2">
            <div className="flex items-center gap-2">
              {settings?.whatsapp.configured ? (
                <CheckCircle2 className="h-4 w-4 text-wa-accent" />
              ) : (
                <AlertTriangle className="h-4 w-4 text-amber-400" />
              )}
              <span className="text-wa-muted">Credenciales:</span>
              <span>{settings?.whatsapp.configured ? 'configuradas' : 'incompletas'}</span>
            </div>
            <div>
              <span className="text-wa-muted">Phone number ID: </span>
              <span>{settings?.whatsapp.phoneNumberId ?? '—'}</span>
            </div>
            <div>
              <span className="text-wa-muted">WABA ID: </span>
              <span>{settings?.whatsapp.wabaId ?? '—'}</span>
            </div>
            <div>
              <span className="text-wa-muted">Graph API: </span>
              <span>{settings?.whatsapp.graphVersion ?? '—'}</span>
            </div>
            <div>
              <span className="text-wa-muted">Firma de webhooks: </span>
              <span className={settings?.whatsapp.signatureEnforced ? 'text-wa-accent' : 'text-red-400'}>
                {settings?.whatsapp.signatureEnforced ? 'verificada' : 'SIN verificar'}
              </span>
            </div>
            <div>
              <span className="text-wa-muted">Número verificado: </span>
              <span>{String(profile?.verified_name ?? '—')}</span>
            </div>
            <div>
              <span className="text-wa-muted">Teléfono: </span>
              <span>{String(profile?.display_phone_number ?? '—')}</span>
            </div>
            <div>
              <span className="text-wa-muted">Calidad: </span>
              <span>{String(profile?.quality_rating ?? '—')}</span>
            </div>
          </dl>

          <div className="mt-4 flex flex-wrap items-center gap-2 rounded-lg bg-wa-surface p-3">
            <span className="text-xs text-wa-muted">URL del webhook para el panel de Meta:</span>
            <code className="rounded bg-black/30 px-2 py-1 text-xs">{webhookUrl}</code>
            <button
              onClick={() => {
                void navigator.clipboard.writeText(webhookUrl);
                toast('URL copiada', 'success');
              }}
              className="rounded p-1 text-wa-muted hover:text-wa-text"
              aria-label="Copiar URL"
            >
              <Copy className="h-3.5 w-3.5" />
            </button>
          </div>
          <p className="mt-2 text-[11px] text-wa-muted">
            En producción tiene que ser HTTPS y accesible desde internet. En local, expón el puerto con un túnel
            (ngrok, cloudflared) y pega esa URL en Meta.
          </p>
        </div>

        {settings && settings.warnings.length > 0 && (
          <div className="rounded-lg bg-amber-500/10 p-5">
            <h2 className="mb-2 flex items-center gap-2 text-sm font-medium text-amber-200">
              <AlertTriangle className="h-4 w-4" /> Avisos de configuración
            </h2>
            <ul className="list-disc space-y-1 pl-5 text-xs text-amber-200/90">
              {settings.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          </div>
        )}

        {/* Preferencias */}
        <div className="space-y-3">
          <h2 className="text-sm font-medium">Preferencias</h2>
          <Toggle
            label="Marcar como leído automáticamente"
            help="Envía la confirmación de lectura a WhatsApp en cuanto llega un mensaje, sin que abras el chat."
            checked={values.autoMarkRead === 'true'}
            onChange={(v) => void update('autoMarkRead', String(v))}
          />
          <Toggle
            label="Notificaciones del navegador"
            help="Avisa de los mensajes nuevos cuando el chat no está abierto."
            checked={values.notificationsEnabled === 'true'}
            onChange={async (v) => {
              if (v && typeof Notification !== 'undefined' && Notification.permission !== 'granted') {
                const perm = await Notification.requestPermission();
                if (perm !== 'granted') {
                  toast('El navegador ha denegado las notificaciones', 'error');
                  return;
                }
              }
              void update('notificationsEnabled', String(v));
            }}
          />
        </div>

        {/* API key */}
        <div className="rounded-lg bg-wa-panel p-5">
          <h2 className="mb-3 flex items-center gap-2 text-sm font-medium">
            <KeyRound className="h-4 w-4" /> Clave de acceso al panel
          </h2>
          <p className="mb-3 text-xs text-wa-muted">
            Debe coincidir con <code className="rounded bg-black/30 px-1">APP_API_KEY</code> del servidor. Se guarda
            en este navegador.
          </p>
          <div className="flex flex-wrap gap-2">
            <input
              type="password"
              value={keyInput}
              onChange={(e) => setKeyInput(e.target.value)}
              placeholder="APP_API_KEY"
              className="min-w-48 flex-1 rounded-lg bg-wa-surface-2 px-3 py-2 text-sm placeholder:text-wa-muted"
            />
            <button
              onClick={() => {
                setApiKey(keyInput.trim());
                connectRealtime();
                void loadSettings();
                toast('Clave guardada', 'success');
              }}
              className="rounded-lg bg-wa-accent px-4 py-2 text-sm font-medium text-black transition hover:brightness-110"
            >
              Guardar
            </button>
          </div>
        </div>

        {/* Métricas */}
        {stats && (
          <div>
            <h2 className="mb-3 text-sm font-medium">Actividad de los últimos 7 días</h2>
            <div className="grid gap-3 sm:grid-cols-3 lg:grid-cols-4">
              <StatCard label="Conversaciones" value={stats.conversations} />
              <StatCard label="Sin leer" value={stats.unread} />
              <StatCard label="Contactos" value={stats.contacts} />
              <StatCard label="Ventanas de 24 h abiertas" value={stats.openWindows} tone="good" />
              <StatCard label="Mensajes recibidos" value={stats.messagesIn7d} />
              <StatCard label="Mensajes enviados" value={stats.messagesOut7d} />
              <StatCard label="Envíos fallidos" value={stats.failed7d} tone="bad" />
            </div>

            {stats.byLabel.length > 0 && (
              <div className="mt-4 rounded-lg bg-wa-panel p-4">
                <h3 className="mb-2 text-xs text-wa-muted">Contactos por etiqueta</h3>
                <ul className="space-y-1.5">
                  {stats.byLabel.map((l) => (
                    <li key={l.name} className="flex items-center gap-2 text-sm">
                      <span className="h-2.5 w-2.5 rounded-full" style={{ background: l.color }} />
                      <span className="flex-1">{l.name}</span>
                      <span className="text-wa-muted">{l.n}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        )}

        {/* Limitaciones */}
        {settings && (
          <div className="rounded-lg bg-wa-surface/60 p-5">
            <h2 className="mb-2 flex items-center gap-2 text-sm font-medium">
              <Info className="h-4 w-4 text-wa-blue" /> Lo que la API oficial no permite
            </h2>
            <ul className="list-disc space-y-1.5 pl-5 text-xs leading-relaxed text-wa-muted">
              {settings.limitations.map((l) => (
                <li key={l}>{l}</li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </section>
  );
}
