import { avatarColor, initials } from '../lib/format';

export function Avatar({ name, waId, size = 48 }: { name: string; waId: string; size?: number }) {
  return (
    <div
      className="flex shrink-0 items-center justify-center rounded-full font-medium text-white/90 select-none"
      style={{
        width: size,
        height: size,
        background: avatarColor(waId),
        fontSize: size * 0.36,
      }}
      aria-hidden
    >
      {initials(name) || '?'}
    </div>
  );
}
