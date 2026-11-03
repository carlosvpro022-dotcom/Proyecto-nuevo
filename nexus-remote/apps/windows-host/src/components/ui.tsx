/**
 * Piezas visuales compartidas.
 *
 * Son deliberadamente simples y sin estado: todo el estilo vive en
 * `styles.css`, de modo que la identidad visual se cambia en un sitio.
 */

import type { ReactNode } from "react";

import type { FeatureStatus } from "../api";

export type Tone = "ok" | "warn" | "danger" | "idle" | "accent";

export function Pill({ tone, children }: { tone: Tone; children: ReactNode }) {
  return <span className={`pill pill--${tone}`}>{children}</span>;
}

export function Card({
  title,
  hint,
  actions,
  children,
}: {
  title?: string;
  hint?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="card fade-in">
      {(title || actions) && (
        <header className="card__header">
          <div>
            {title && <h2 className="card__title">{title}</h2>}
            {hint && <p className="card__hint">{hint}</p>}
          </div>
          {actions && <div className="row">{actions}</div>}
        </header>
      )}
      <div className="card__body">{children}</div>
    </section>
  );
}

export function Stat({
  label,
  value,
  note,
}: {
  label: string;
  value: ReactNode;
  note?: string;
}) {
  return (
    <div className="stat">
      <span className="stat__label">{label}</span>
      <span className="stat__value">{value}</span>
      {note && <span className="stat__note">{note}</span>}
    </div>
  );
}

export function Notice({
  tone = "info",
  children,
}: {
  tone?: "info" | "warn" | "danger" | "accent";
  children: ReactNode;
}) {
  const className = tone === "info" ? "notice" : `notice notice--${tone}`;
  return (
    <div className={className} role={tone === "danger" ? "alert" : undefined}>
      <span className="notice__icon" aria-hidden="true">
        {tone === "danger" || tone === "warn" ? "!" : "i"}
      </span>
      <div>{children}</div>
    </div>
  );
}

/**
 * Pantalla de una funcion que todavia no existe.
 *
 * Dice exactamente que falta y en que etapa llega. Es la alternativa honesta a
 * una pantalla con botones que no hacen nada.
 */
export function Pending({
  title,
  stage,
  detail,
  dependsOn,
}: {
  title: string;
  stage: number;
  detail: string;
  dependsOn?: string;
}) {
  return (
    <div className="card fade-in">
      <div className="pending">
        <div className="pending__icon" aria-hidden="true">
          <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 7v5l3 2" strokeLinecap="round" />
          </svg>
        </div>
        <h2 className="pending__title">{title}</h2>
        <Pill tone="idle">Etapa {stage}</Pill>
        <p className="pending__text">{detail}</p>
        {dependsOn && (
          <p className="pending__text" style={{ color: "var(--text-muted)" }}>
            Depende de: {dependsOn}
          </p>
        )}
      </div>
    </div>
  );
}

export function FeatureRow({ feature }: { feature: FeatureStatus }) {
  return (
    <div className="feature">
      <div className="feature__body">
        <div className="feature__name">{feature.label}</div>
        <div className="feature__detail">{feature.detail}</div>
      </div>
      {feature.ready ? (
        <Pill tone="ok">Funciona</Pill>
      ) : (
        <Pill tone="idle">Etapa {feature.stage}</Pill>
      )}
    </div>
  );
}

export function Spinner({ label }: { label: string }) {
  return (
    <div className="row" role="status">
      <span className="spinner" aria-hidden="true" />
      <span style={{ color: "var(--text-secondary)", fontSize: 13 }}>{label}</span>
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="empty">{children}</p>;
}

/** Iconos de la barra lateral. Trazo de 1,6 px para que pesen poco a 18 px. */
export function Icon({ name }: { name: string }) {
  const paths: Record<string, ReactNode> = {
    dashboard: (
      <>
        <rect x="3" y="3" width="7" height="8" rx="1.5" />
        <rect x="14" y="3" width="7" height="5" rx="1.5" />
        <rect x="3" y="14" width="7" height="7" rx="1.5" />
        <rect x="14" y="11" width="7" height="10" rx="1.5" />
      </>
    ),
    devices: (
      <>
        <rect x="4" y="2.5" width="9" height="19" rx="2" />
        <rect x="15" y="8" width="5" height="9" rx="1.5" />
        <path d="M8.5 18.5h0.01" strokeLinecap="round" />
      </>
    ),
    connect: (
      <>
        <path d="M12 3v3M12 18v3M3 12h3M18 12h3" strokeLinecap="round" />
        <circle cx="12" cy="12" r="4" />
      </>
    ),
    desktop: (
      <>
        <rect x="2.5" y="4" width="19" height="12.5" rx="2" />
        <path d="M9 20h6M12 16.5V20" strokeLinecap="round" />
      </>
    ),
    files: (
      <>
        <path d="M3.5 7.5a2 2 0 0 1 2-2h3.2l2 2.5h7.8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />
      </>
    ),
    history: (
      <>
        <circle cx="12" cy="12" r="8.5" />
        <path d="M12 7.5V12l3.2 2" strokeLinecap="round" />
      </>
    ),
    security: (
      <>
        <path d="M12 2.8l7.5 3v6c0 4.4-3.1 8.4-7.5 9.4-4.4-1-7.5-5-7.5-9.4v-6z" />
        <path d="M9.2 12.2l2 2 3.6-3.8" strokeLinecap="round" />
      </>
    ),
    settings: (
      <>
        <circle cx="12" cy="12" r="3" />
        <path d="M12 2.5v2.2M12 19.3v2.2M4.8 4.8l1.6 1.6M17.6 17.6l1.6 1.6M2.5 12h2.2M19.3 12h2.2M4.8 19.2l1.6-1.6M17.6 6.4l1.6-1.6" strokeLinecap="round" />
      </>
    ),
  };

  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      aria-hidden="true"
    >
      {paths[name] ?? null}
    </svg>
  );
}
