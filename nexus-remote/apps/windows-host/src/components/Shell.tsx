/** Estructura de la ventana: barra lateral, cabecera y area de contenido. */

import type { ReactNode } from "react";

import { Icon, Pill } from "./ui";

export interface NavEntry {
  id: string;
  label: string;
  icon: string;
  badge?: string;
  ready: boolean;
}

export function Shell({
  nav,
  active,
  onNavigate,
  title,
  subtitle,
  headerActions,
  footer,
  children,
}: {
  nav: NavEntry[];
  active: string;
  onNavigate: (id: string) => void;
  title: string;
  subtitle: string;
  headerActions?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand__mark" aria-hidden="true">
            N
          </div>
          <div className="brand__text">
            <div className="brand__name">NEXUS REMOTE</div>
            <div className="brand__tag">Host de Windows</div>
          </div>
        </div>

        <nav className="nav" aria-label="Secciones">
          <div className="nav__label">Principal</div>
          {nav.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className="nav__item"
              aria-current={entry.id === active ? "page" : undefined}
              onClick={() => onNavigate(entry.id)}
              title={entry.ready ? entry.label : `${entry.label} (pendiente)`}
            >
              <Icon name={entry.icon} />
              <span>{entry.label}</span>
              {entry.badge && <span className="nav__badge">{entry.badge}</span>}
            </button>
          ))}
        </nav>

        {footer && <div className="sidebar__footer">{footer}</div>}
      </aside>

      <main className="main">
        <header className="topbar">
          <div>
            <h1 className="topbar__title">{title}</h1>
            <p className="topbar__subtitle">{subtitle}</p>
          </div>
          {headerActions && <div className="row">{headerActions}</div>}
        </header>
        <div className="content">{children}</div>
      </main>
    </div>
  );
}

export { Pill };
