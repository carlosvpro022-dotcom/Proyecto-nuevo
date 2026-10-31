import { X } from 'lucide-react';
import { useEffect, type ReactNode } from 'react';

export function Modal({
  title,
  children,
  onClose,
  wide = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={onClose}
      role="presentation"
    >
      <div
        className={`flex max-h-[85vh] w-full flex-col overflow-hidden rounded-lg bg-wa-panel shadow-2xl ${
          wide ? 'max-w-3xl' : 'max-w-md'
        }`}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={title}
      >
        <header className="flex items-center justify-between border-b border-wa-border px-5 py-3.5">
          <h2 className="text-base font-medium">{title}</h2>
          <button
            onClick={onClose}
            className="rounded-full p-1.5 text-wa-muted transition hover:bg-wa-surface-2 hover:text-wa-text"
            aria-label="Cerrar"
          >
            <X className="h-5 w-5" />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
      </div>
    </div>
  );
}
