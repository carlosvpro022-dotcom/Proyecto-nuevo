import { useState } from 'react';
import { Download, FileText, MapPin, Reply, ShoppingBag, SmilePlus, AlertTriangle } from 'lucide-react';
import { api, mediaUrl } from '../lib/api';
import { formatBytes, formatTime } from '../lib/format';
import { useStore } from '../lib/store';
import { StatusTicks } from './StatusTicks';
import type { Message } from '../types';

const QUICK_REACTIONS = ['👍', '❤️', '😂', '😮', '😢', '🙏'];

function MediaPending() {
  return (
    <div className="flex h-32 w-56 items-center justify-center rounded-lg bg-black/25 text-xs text-white/60">
      Descargando adjunto…
    </div>
  );
}

function MediaError() {
  return (
    <div className="flex h-20 w-56 items-center justify-center gap-2 rounded-lg bg-black/25 px-3 text-xs text-white/60">
      <AlertTriangle className="h-4 w-4" />
      No se pudo descargar el archivo
    </div>
  );
}

function MediaBody({ message }: { message: Message }) {
  const media = message.media;
  if (!media) return null;
  if (media.state === 'pending') return <MediaPending />;
  if (media.state === 'error' || !media.url) return <MediaError />;

  const src = mediaUrl(media.url);

  if (message.type === 'image' || message.type === 'sticker') {
    const isSticker = message.type === 'sticker';
    return (
      <a href={src} target="_blank" rel="noreferrer" className="block">
        <img
          src={src}
          alt={isSticker ? 'Sticker' : 'Imagen recibida'}
          loading="lazy"
          className={isSticker ? 'max-h-40 w-auto' : 'max-h-80 w-auto rounded-lg object-cover'}
        />
      </a>
    );
  }

  if (message.type === 'video') {
    return <video src={src} controls preload="metadata" className="max-h-80 w-full rounded-lg" />;
  }

  if (message.type === 'audio') {
    return <audio src={src} controls preload="metadata" className="w-64 max-w-full" />;
  }

  return (
    <a
      href={`${src}${src?.includes('?') ? '&' : '?'}download=1`}
      className="flex items-center gap-3 rounded-lg bg-black/20 p-3 transition hover:bg-black/30"
    >
      <FileText className="h-8 w-8 shrink-0 text-white/70" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm">{media.filename ?? 'Documento'}</span>
        <span className="text-[11px] text-white/60">
          {[media.mime?.split('/')[1]?.toUpperCase(), formatBytes(media.size)].filter(Boolean).join(' · ')}
        </span>
      </span>
      <Download className="h-4 w-4 shrink-0 text-white/70" />
    </a>
  );
}

function SpecialBody({ message }: { message: Message }) {
  const p = message.payload as Record<string, any> | null;

  if (message.type === 'location' && p) {
    const url = `https://www.google.com/maps?q=${p.latitude},${p.longitude}`;
    return (
      <a href={url} target="_blank" rel="noreferrer" className="flex items-start gap-2 text-sm underline-offset-2 hover:underline">
        <MapPin className="mt-0.5 h-4 w-4 shrink-0" />
        <span>
          {p.name ?? 'Ubicación compartida'}
          {p.address && <span className="block text-xs text-white/60">{p.address}</span>}
        </span>
      </a>
    );
  }

  if (message.type === 'order' && p) {
    const items: Array<Record<string, any>> = p.product_items ?? [];
    const total = items.reduce((sum, i) => sum + (i.item_price ?? 0) * (i.quantity ?? 0), 0);
    return (
      <div className="space-y-1.5 text-sm">
        <p className="flex items-center gap-2 font-medium">
          <ShoppingBag className="h-4 w-4" /> Pedido del catálogo
        </p>
        {items.map((i, n) => (
          <p key={n} className="text-xs text-white/80">
            {i.quantity}× {i.product_retailer_id} — {i.item_price} {i.currency}
          </p>
        ))}
        {total > 0 && (
          <p className="border-t border-white/15 pt-1.5 text-xs font-medium">
            Total: {total.toFixed(2)} {items[0]?.currency ?? ''}
          </p>
        )}
      </div>
    );
  }

  if (message.type === 'contacts' && Array.isArray(p)) {
    return (
      <div className="text-sm">
        {p.map((c: any, n: number) => (
          <p key={n}>{c?.name?.formatted_name ?? 'Contacto compartido'}</p>
        ))}
      </div>
    );
  }

  if (message.type === 'interactive' && p) {
    return (
      <p className="flex items-center gap-2 text-sm">
        <ShoppingBag className="h-4 w-4" />
        {message.body ?? 'Mensaje interactivo'}
      </p>
    );
  }

  return null;
}

export function MessageBubble({ message, quoted }: { message: Message; quoted?: Message }) {
  const out = message.direction === 'out';
  const setReplyTo = useStore((s) => s.setReplyTo);
  const toast = useStore((s) => s.toast);
  const [showReactions, setShowReactions] = useState(false);

  async function react(emoji: string) {
    if (!message.wamId) {
      toast('Solo se puede reaccionar a mensajes ya entregados', 'error');
      return;
    }
    setShowReactions(false);
    try {
      await api.react(message.wamId, message.reaction === emoji ? '' : emoji);
    } catch (err) {
      toast((err as Error).message, 'error');
    }
  }

  if (message.type === 'system') {
    return (
      <div className="my-2 flex justify-center">
        <span className="rounded-lg bg-wa-surface/80 px-3 py-1 text-[11px] text-wa-muted">{message.body}</span>
      </div>
    );
  }

  const hasMedia = ['image', 'video', 'audio', 'document', 'sticker'].includes(message.type);
  const isSpecial = ['location', 'order', 'contacts', 'interactive'].includes(message.type);
  const bare = message.type === 'sticker';

  return (
    <div className={`group flex items-end gap-1.5 ${out ? 'justify-end' : 'justify-start'}`}>
      {out && (
        <div className="flex gap-0.5 opacity-0 transition group-hover:opacity-100">
          <button
            onClick={() => setShowReactions((v) => !v)}
            title="Reaccionar"
            className="rounded-full p-1.5 text-wa-muted hover:bg-wa-surface-2 hover:text-wa-text"
          >
            <SmilePlus className="h-4 w-4" />
          </button>
          <button
            onClick={() => setReplyTo(message)}
            title="Responder"
            className="rounded-full p-1.5 text-wa-muted hover:bg-wa-surface-2 hover:text-wa-text"
          >
            <Reply className="h-4 w-4" />
          </button>
        </div>
      )}

      <div className="relative max-w-[min(65ch,78%)]">
        {showReactions && (
          <div
            className={`absolute -top-10 z-10 flex gap-1 rounded-full bg-wa-surface-2 px-2 py-1.5 shadow-xl ${
              out ? 'right-0' : 'left-0'
            }`}
          >
            {QUICK_REACTIONS.map((e) => (
              <button key={e} onClick={() => void react(e)} className="text-lg transition hover:scale-125">
                {e}
              </button>
            ))}
          </div>
        )}

        <div
          className={`relative w-fit max-w-full rounded-lg text-[14.5px] leading-snug shadow-sm ${
            out ? 'ml-auto' : ''
          } ${
            bare
              ? ''
              : out
                ? 'bubble-tail-out rounded-tr-none bg-wa-out px-2.5 py-1.5'
                : 'bubble-tail-in rounded-tl-none bg-wa-in px-2.5 py-1.5'
          }`}
        >
          {quoted && (
            <div
              className={`mb-1.5 overflow-hidden rounded border-l-4 bg-black/20 px-2 py-1 text-xs ${
                quoted.direction === 'out' ? 'border-wa-accent' : 'border-wa-blue'
              }`}
            >
              <p className="font-medium text-wa-accent">{quoted.direction === 'out' ? 'Tú' : 'Contacto'}</p>
              <p className="line-clamp-2 text-white/70">{quoted.body ?? 'Adjunto'}</p>
            </div>
          )}

          {hasMedia && <MediaBody message={message} />}
          {isSpecial && <SpecialBody message={message} />}

          {message.type === 'template' && (
            <p className="mb-1 text-[11px] font-medium tracking-wide text-wa-accent uppercase">Plantilla enviada</p>
          )}

          {message.body && !isSpecial && (
            <p className={`whitespace-pre-wrap break-words ${hasMedia ? 'mt-1.5' : ''}`}>{message.body}</p>
          )}

          <div
            className={`mt-0.5 flex items-center justify-end gap-1 text-[11px] ${
              bare ? 'text-wa-muted' : 'text-white/55'
            }`}
          >
            <span>{formatTime(message.timestamp)}</span>
            {out && <StatusTicks status={message.status} />}
          </div>

          {message.reaction && (
            <span className="absolute -bottom-3 left-2 rounded-full bg-wa-surface-2 px-1.5 py-0.5 text-xs shadow">
              {message.reaction}
            </span>
          )}
        </div>

        {message.status === 'failed' && message.error && (
          <p className={`mt-1.5 flex items-start gap-1.5 text-[11px] text-red-400 ${out ? 'justify-end text-right' : ''}`}>
            <AlertTriangle className="mt-px h-3.5 w-3.5 shrink-0" />
            <span>
              {message.error.message}
              {message.error.code && <span className="text-red-400/70"> (código {message.error.code})</span>}
            </span>
          </p>
        )}
      </div>

      {!out && (
        <div className="flex gap-0.5 opacity-0 transition group-hover:opacity-100">
          <button
            onClick={() => setReplyTo(message)}
            title="Responder"
            className="rounded-full p-1.5 text-wa-muted hover:bg-wa-surface-2 hover:text-wa-text"
          >
            <Reply className="h-4 w-4" />
          </button>
          <button
            onClick={() => setShowReactions((v) => !v)}
            title="Reaccionar"
            className="rounded-full p-1.5 text-wa-muted hover:bg-wa-surface-2 hover:text-wa-text"
          >
            <SmilePlus className="h-4 w-4" />
          </button>
        </div>
      )}
    </div>
  );
}
