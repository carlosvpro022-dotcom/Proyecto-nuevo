import { X } from 'lucide-react';
import { useStore } from '../lib/store';

const STYLES = {
  info: 'bg-wa-surface-2 text-wa-text',
  success: 'bg-wa-accent text-black',
  error: 'bg-red-500/95 text-white',
} as const;

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);

  return (
    <div className="pointer-events-none fixed bottom-5 left-1/2 z-[60] flex -translate-x-1/2 flex-col gap-2">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`pointer-events-auto flex max-w-md animate-fade-up items-start gap-3 rounded-lg px-4 py-2.5 text-sm shadow-xl ${STYLES[t.kind]}`}
          role="status"
        >
          <span className="flex-1">{t.text}</span>
          <button onClick={() => dismiss(t.id)} aria-label="Cerrar aviso" className="opacity-70 hover:opacity-100">
            <X className="h-4 w-4" />
          </button>
        </div>
      ))}
    </div>
  );
}
