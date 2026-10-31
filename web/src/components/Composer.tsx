import { useRef, useState } from 'react';
import { Paperclip, SendHorizonal, Smile, X, FileText, Image as ImageIcon, MapPin, LayoutTemplate, ShoppingBag } from 'lucide-react';
import { api, ApiError } from '../lib/api';
import { useStore } from '../lib/store';
import { TemplatePicker } from './TemplatePicker';
import { ProductPicker } from './ProductPicker';
import type { Conversation } from '../types';

const EMOJIS = [
  '😀','😂','🙂','😉','😍','😘','😎','🤔','😐','😴','😭','😡','👍','👎','👌','🙏','👏','💪',
  '❤️','💔','🔥','✨','🎉','✅','❌','⚠️','📎','📞','📍','⏰','💰','🛒','📦','🚚','🙌','🤝',
];

export function Composer({ conv }: { conv: Conversation }) {
  const [text, setText] = useState('');
  const [sending, setSending] = useState(false);
  const [showEmoji, setShowEmoji] = useState(false);
  const [showAttach, setShowAttach] = useState(false);
  const [showTemplates, setShowTemplates] = useState(false);
  const [showProducts, setShowProducts] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const textRef = useRef<HTMLTextAreaElement>(null);

  const replyTo = useStore((s) => s.replyTo);
  const setReplyTo = useStore((s) => s.setReplyTo);
  const toast = useStore((s) => s.toast);

  function autoGrow() {
    const el = textRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }

  async function send() {
    const value = text.trim();
    if (!value || sending) return;
    setSending(true);
    try {
      await api.sendText(conv.id, value, replyTo?.wamId ?? undefined);
      setText('');
      setReplyTo(null);
      requestAnimationFrame(autoGrow);
    } catch (err) {
      if (err instanceof ApiError && err.windowClosed) {
        toast('La ventana de 24 h está cerrada: envía una plantilla aprobada.', 'error');
        setShowTemplates(true);
      } else {
        toast((err as Error).message, 'error');
      }
    } finally {
      setSending(false);
    }
  }

  async function sendFile(file: File) {
    setShowAttach(false);
    const toastId = `Subiendo ${file.name}…`;
    toast(toastId);
    try {
      await api.sendMedia(conv.id, file, undefined, replyTo?.wamId ?? undefined);
      setReplyTo(null);
    } catch (err) {
      toast((err as Error).message, 'error');
    }
    if (fileRef.current) fileRef.current.value = '';
  }

  async function shareLocation() {
    setShowAttach(false);
    if (!navigator.geolocation) {
      toast('Este navegador no permite compartir ubicación', 'error');
      return;
    }
    navigator.geolocation.getCurrentPosition(
      async (pos) => {
        try {
          await api.sendLocation(conv.id, {
            latitude: Number(pos.coords.latitude.toFixed(6)),
            longitude: Number(pos.coords.longitude.toFixed(6)),
          });
        } catch (err) {
          toast((err as Error).message, 'error');
        }
      },
      () => toast('No se pudo obtener la ubicación', 'error')
    );
  }

  return (
    <div className="border-t border-wa-border bg-wa-surface">
      {!conv.windowOpen && (
        <div className="flex flex-wrap items-center gap-2 bg-amber-500/10 px-4 py-2 text-xs text-amber-200">
          <span className="flex-1 min-w-[16rem]">
            Han pasado más de 24 h desde el último mensaje del contacto. Meta solo permite reanudar la conversación
            con una plantilla aprobada.
          </span>
          <button
            onClick={() => setShowTemplates(true)}
            className="rounded-full bg-amber-400 px-3 py-1 font-medium text-black transition hover:brightness-110"
          >
            Elegir plantilla
          </button>
        </div>
      )}

      {replyTo && (
        <div className="flex items-start gap-3 border-l-4 border-wa-accent bg-wa-panel/70 px-4 py-2">
          <div className="min-w-0 flex-1">
            <p className="text-xs font-medium text-wa-accent">
              Respondiendo a {replyTo.direction === 'out' ? 'tu mensaje' : conv.name}
            </p>
            <p className="truncate text-xs text-wa-muted">{replyTo.body ?? 'Adjunto'}</p>
          </div>
          <button onClick={() => setReplyTo(null)} aria-label="Cancelar respuesta" className="text-wa-muted hover:text-wa-text">
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      {showEmoji && (
        <div className="grid grid-cols-12 gap-1 border-b border-wa-border bg-wa-panel p-3">
          {EMOJIS.map((e) => (
            <button
              key={e}
              onClick={() => {
                setText((t) => t + e);
                textRef.current?.focus();
              }}
              className="rounded p-1 text-xl transition hover:bg-wa-surface-2"
            >
              {e}
            </button>
          ))}
        </div>
      )}

      <div className="relative flex items-end gap-2 px-3 py-2.5">
        <button
          onClick={() => {
            setShowEmoji((v) => !v);
            setShowAttach(false);
          }}
          title="Emojis"
          className={`rounded-full p-2 transition hover:bg-wa-surface-2 ${showEmoji ? 'text-wa-accent' : 'text-wa-muted'}`}
        >
          <Smile className="h-6 w-6" />
        </button>

        <div className="relative">
          <button
            onClick={() => {
              setShowAttach((v) => !v);
              setShowEmoji(false);
            }}
            title="Adjuntar"
            className={`rounded-full p-2 transition hover:bg-wa-surface-2 ${showAttach ? 'text-wa-accent' : 'text-wa-muted'}`}
          >
            <Paperclip className="h-6 w-6" />
          </button>

          {showAttach && (
            <div className="absolute bottom-14 left-0 z-20 w-52 overflow-hidden rounded-lg bg-wa-surface-2 py-1 shadow-xl">
              {[
                { label: 'Foto o vídeo', Icon: ImageIcon, accept: 'image/*,video/*' },
                { label: 'Documento', Icon: FileText, accept: '*/*' },
              ].map(({ label, Icon, accept }) => (
                <button
                  key={label}
                  onClick={() => {
                    if (fileRef.current) {
                      fileRef.current.accept = accept;
                      fileRef.current.click();
                    }
                  }}
                  className="flex w-full items-center gap-3 px-4 py-2.5 text-sm transition hover:bg-wa-surface"
                >
                  <Icon className="h-4 w-4 text-wa-muted" /> {label}
                </button>
              ))}
              <button
                onClick={() => void shareLocation()}
                className="flex w-full items-center gap-3 px-4 py-2.5 text-sm transition hover:bg-wa-surface"
              >
                <MapPin className="h-4 w-4 text-wa-muted" /> Ubicación actual
              </button>
              <button
                onClick={() => {
                  setShowAttach(false);
                  setShowProducts(true);
                }}
                className="flex w-full items-center gap-3 px-4 py-2.5 text-sm transition hover:bg-wa-surface"
              >
                <ShoppingBag className="h-4 w-4 text-wa-muted" /> Producto del catálogo
              </button>
            </div>
          )}
        </div>

        <button
          onClick={() => setShowTemplates(true)}
          title="Enviar plantilla"
          className="rounded-full p-2 text-wa-muted transition hover:bg-wa-surface-2"
        >
          <LayoutTemplate className="h-6 w-6" />
        </button>

        <textarea
          ref={textRef}
          value={text}
          rows={1}
          onChange={(e) => {
            setText(e.target.value);
            autoGrow();
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              void send();
            }
          }}
          placeholder={conv.windowOpen ? 'Escribe un mensaje' : 'Ventana cerrada: usa una plantilla'}
          className="max-h-40 min-h-[42px] flex-1 resize-none rounded-lg bg-wa-surface-2 px-4 py-2.5 text-[15px] placeholder:text-wa-muted"
        />

        <button
          onClick={() => void send()}
          disabled={!text.trim() || sending}
          title="Enviar"
          className="rounded-full bg-wa-accent p-2.5 text-black transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <SendHorizonal className="h-5 w-5" />
        </button>

        <input
          ref={fileRef}
          type="file"
          hidden
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) void sendFile(file);
          }}
        />
      </div>

      {showTemplates && <TemplatePicker conversationId={conv.id} onClose={() => setShowTemplates(false)} />}
      {showProducts && <ProductPicker conversationId={conv.id} onClose={() => setShowProducts(false)} />}
    </div>
  );
}
