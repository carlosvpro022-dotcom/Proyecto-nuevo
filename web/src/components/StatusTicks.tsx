import { Check, CheckCheck, Clock, AlertCircle } from 'lucide-react';
import type { MessageStatus } from '../types';

const LABEL: Record<MessageStatus, string> = {
  pending: 'Enviando',
  sent: 'Enviado',
  delivered: 'Entregado',
  read: 'Leído',
  failed: 'No se pudo enviar',
  received: 'Recibido',
};

export function StatusTicks({ status }: { status: MessageStatus }) {
  const common = 'h-4 w-4 shrink-0';
  const title = LABEL[status];

  if (status === 'pending') return <Clock className={`${common} text-white/50`} aria-label={title} />;
  if (status === 'failed') return <AlertCircle className={`${common} text-red-400`} aria-label={title} />;
  if (status === 'read') return <CheckCheck className={`${common} text-wa-blue`} aria-label={title} />;
  if (status === 'delivered') return <CheckCheck className={`${common} text-white/60`} aria-label={title} />;
  return <Check className={`${common} text-white/60`} aria-label={title} />;
}
