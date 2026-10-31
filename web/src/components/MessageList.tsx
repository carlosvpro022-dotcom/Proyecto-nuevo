import { useEffect, useLayoutEffect, useRef } from 'react';
import { useStore } from '../lib/store';
import { formatDayDivider, sameDay } from '../lib/format';
import { MessageBubble } from './MessageBubble';
import type { Message } from '../types';

/**
 * Referencia estable: zustand v5 compara por identidad, así que devolver un
 * array nuevo desde el selector provocaría un bucle infinito de renders.
 */
const EMPTY: Message[] = [];

export function MessageList({ conversationId }: { conversationId: number }) {
  const messages = useStore((s) => s.messages[conversationId] ?? EMPTY);
  const hasMore = useStore((s) => s.hasMore[conversationId] ?? false);
  const loadOlder = useStore((s) => s.loadOlder);
  const loading = useStore((s) => s.loadingMessages);

  const scrollRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastCount = useRef(0);
  const pinnedToBottom = useRef(true);

  // Solo autoscroll si el usuario ya estaba abajo: no le robamos la lectura.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const grew = messages.length > lastCount.current;
    lastCount.current = messages.length;
    if (grew && pinnedToBottom.current) {
      bottomRef.current?.scrollIntoView({ block: 'end' });
    }
  }, [messages]);

  // Al cambiar de chat, siempre abajo del todo.
  useEffect(() => {
    lastCount.current = 0;
    pinnedToBottom.current = true;
    requestAnimationFrame(() => bottomRef.current?.scrollIntoView({ block: 'end' }));
  }, [conversationId]);

  /*
   * Las imágenes y los vídeos no tienen altura hasta que cargan, así que el
   * primer scroll se queda corto. Observamos el alto real del contenido y
   * volvemos a bajar mientras el usuario siga pegado al final.
   */
  useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      if (pinnedToBottom.current) bottomRef.current?.scrollIntoView({ block: 'end' });
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [conversationId]);

  /*
   * El anclaje al final se decide por el gesto del usuario, no por la posición
   * del scroll: al cargar una imagen el contenido crece de golpe y la posición
   * mentiría durante unos milisegundos, desanclando la vista sin que nadie la
   * haya tocado.
   */
  function onUserGesture() {
    const el = scrollRef.current;
    if (!el) return;
    pinnedToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
  }

  function onScroll() {
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollTop < 80 && hasMore && !loading) {
      const prevHeight = el.scrollHeight;
      void loadOlder(conversationId).then(() => {
        requestAnimationFrame(() => {
          el.scrollTop = el.scrollHeight - prevHeight;
        });
      });
    }
  }

  const byWamId = new Map<string, Message>();
  for (const m of messages) if (m.wamId) byWamId.set(m.wamId, m);

  return (
    <div
      ref={scrollRef}
      onScroll={onScroll}
      onWheel={onUserGesture}
      onTouchMove={onUserGesture}
      onMouseDown={onUserGesture}
      onKeyDown={onUserGesture}
      className="chat-wallpaper min-h-0 flex-1 overflow-y-auto px-4 py-4 md:px-12"
    >
      {hasMore && (
        <div className="mb-4 flex justify-center">
          <button
            onClick={() => void loadOlder(conversationId)}
            className="rounded-full bg-wa-surface px-4 py-1.5 text-xs text-wa-muted transition hover:text-wa-text"
          >
            Cargar mensajes anteriores
          </button>
        </div>
      )}

      {messages.length === 0 && !loading && (
        <div className="flex h-full items-center justify-center">
          <p className="max-w-sm rounded-lg bg-wa-surface/70 px-4 py-3 text-center text-sm text-wa-muted">
            Aún no hay mensajes en este chat.
          </p>
        </div>
      )}

      <div ref={contentRef} className="mx-auto flex max-w-4xl flex-col gap-2">
        {messages.map((m, i) => {
          const prev = messages[i - 1];
          const showDivider = !prev || !sameDay(prev.timestamp, m.timestamp);
          return (
            <div key={m.id} className="contents">
              {showDivider && (
                <div className="my-3 flex justify-center">
                  <span className="rounded-lg bg-wa-surface/90 px-3 py-1 text-[11px] font-medium tracking-wide text-wa-muted">
                    {formatDayDivider(m.timestamp)}
                  </span>
                </div>
              )}
              <MessageBubble message={m} quoted={m.replyTo ? byWamId.get(m.replyTo) : undefined} />
            </div>
          );
        })}
      </div>

      <div ref={bottomRef} className="h-1" />
    </div>
  );
}
