const time = new Intl.DateTimeFormat('es-ES', { hour: '2-digit', minute: '2-digit' });
const dayMonth = new Intl.DateTimeFormat('es-ES', { day: '2-digit', month: '2-digit', year: '2-digit' });
const fullDay = new Intl.DateTimeFormat('es-ES', { weekday: 'long', day: 'numeric', month: 'long' });

export function formatTime(ts: number): string {
  return time.format(new Date(ts));
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Etiqueta corta para la lista de chats: hora hoy, "Ayer", o fecha. */
export function formatListStamp(ts: number | null): string {
  if (!ts) return '';
  const now = new Date();
  const then = new Date(ts);
  const days = Math.round((startOfDay(now) - startOfDay(then)) / 86_400_000);
  if (days === 0) return time.format(then);
  if (days === 1) return 'Ayer';
  if (days < 7) return then.toLocaleDateString('es-ES', { weekday: 'long' });
  return dayMonth.format(then);
}

/** Separador de día dentro del chat. */
export function formatDayDivider(ts: number): string {
  const now = new Date();
  const then = new Date(ts);
  const days = Math.round((startOfDay(now) - startOfDay(then)) / 86_400_000);
  if (days === 0) return 'HOY';
  if (days === 1) return 'AYER';
  return fullDay.format(then).toUpperCase();
}

export function sameDay(a: number, b: number): boolean {
  return startOfDay(new Date(a)) === startOfDay(new Date(b));
}

export function formatBytes(bytes: number | null): string {
  if (!bytes) return '';
  const units = ['B', 'KB', 'MB', 'GB'];
  let i = 0;
  let n = bytes;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${units[i]}`;
}

/** Cuenta atrás de la ventana de 24 h. */
export function formatWindowLeft(closesAt: number | null): string {
  if (!closesAt) return 'sin ventana abierta';
  const ms = closesAt - Date.now();
  if (ms <= 0) return 'ventana cerrada';
  const hours = Math.floor(ms / 3_600_000);
  const minutes = Math.floor((ms % 3_600_000) / 60_000);
  return hours > 0 ? `${hours} h ${minutes} min` : `${minutes} min`;
}

export function minutesToHHMM(min: number | null): string {
  if (min === null) return '';
  return `${String(Math.floor(min / 60)).padStart(2, '0')}:${String(min % 60).padStart(2, '0')}`;
}

export function hhmmToMinutes(value: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!m) return null;
  return Number(m[1]) * 60 + Number(m[2]);
}

/** Color estable a partir del número, para el avatar. */
export function avatarColor(seed: string): string {
  const palette = ['#6a7a86', '#8c6f4f', '#4f7a8c', '#7a4f6f', '#4f8c66', '#8c7a4f', '#6f4f8c'];
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  return palette[hash % palette.length];
}

export function initials(name: string): string {
  const clean = name.replace(/^\+/, '').trim();
  if (/^\d+$/.test(clean)) return clean.slice(-2);
  // Solo letras y dígitos: "María (tienda)" -> "MT", no "M(".
  const words = clean.split(/\s+/).map((w) => w.replace(/[^\p{L}\p{N}]/gu, '')).filter(Boolean);
  return words.slice(0, 2).map((w) => w[0].toUpperCase()).join('') || clean.slice(0, 2).toUpperCase();
}
