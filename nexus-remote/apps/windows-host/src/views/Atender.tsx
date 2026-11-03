/**
 * Control del bucle que atiende sesiones.
 *
 * Es el interruptor principal del producto: mientras no este en marcha, el
 * equipo no es localizable y ningun dispositivo puede abrir sesion. Por eso
 * dice en todo momento que esta pasando, y no solo si esta encendido.
 */

import { useState } from "react";

import { formatMoment, type RuntimeStatus } from "../api";
import { Card, Notice, Pill } from "../components/ui";

export function Atender({
  status,
  configured,
  busy,
  onStart,
  onStop,
}: {
  status: RuntimeStatus | null;
  configured: boolean;
  busy: boolean;
  onStart: (enrollmentToken: string) => void;
  onStop: () => void;
}) {
  const [token, setToken] = useState("");
  const marcha = status?.running ?? false;

  return (
    <Card
      title="Atender sesiones"
      hint="Mientras no este en marcha, este equipo no es localizable y nadie puede conectarse."
      actions={
        <Pill tone={estadoTono(status)}>{estadoTexto(status)}</Pill>
      }
    >
      {!configured && (
        <Notice tone="warn">
          Falta configurar la direccion del servidor en <em>Configuracion</em>.
        </Notice>
      )}

      {status && (
        <div style={{ marginTop: configured ? 0 : "var(--space-4)" }}>
          <Notice tone={detalleTono(status)}>{status.detail}</Notice>
        </div>
      )}

      {status?.activeSession && (
        <table className="table" style={{ marginTop: "var(--space-4)" }}>
          <tbody>
            <tr>
              <td style={{ color: "var(--text-secondary)", width: 200 }}>Dispositivo</td>
              <td className="mono">{status.activeSession.controller}</td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Desde</td>
              <td>{formatMoment(status.activeSession.startedAtMs)}</td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Puede</td>
              <td>
                {status.activeSession.permissions.length === 0
                  ? "nada"
                  : status.activeSession.permissions.join(", ")}
              </td>
            </tr>
          </tbody>
        </table>
      )}

      {!marcha && (
        <div className="field" style={{ maxWidth: 560, marginTop: "var(--space-4)" }}>
          <label className="field__label" htmlFor="atender-token">
            Testigo de alta (solo la primera vez)
          </label>
          <input
            id="atender-token"
            className="input"
            type="password"
            value={token}
            placeholder="Valor de NEXUS_HOST_ENROLLMENT_TOKEN"
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setToken(event.target.value)}
          />
        </div>
      )}

      <div className="row" style={{ marginTop: "var(--space-4)" }}>
        {marcha ? (
          <button type="button" className="btn" disabled={busy} onClick={onStop}>
            Dejar de atender
          </button>
        ) : (
          <button
            type="button"
            className="btn btn--primary"
            disabled={!configured || busy}
            onClick={() => onStart(token)}
          >
            Empezar a atender
          </button>
        )}
        {status && (status.sessionsServed > 0 || status.sessionsRefused > 0) && (
          <span className="field__hint">
            {status.sessionsServed} sesion(es) atendida(s), {status.sessionsRefused}{" "}
            rechazada(s) desde que arranco.
          </span>
        )}
      </div>
    </Card>
  );
}

function estadoTexto(status: RuntimeStatus | null): string {
  if (!status?.running) return "Detenido";
  if (status.activeSession) return "Sesion en curso";
  return status.available ? "Disponible" : "Conectando";
}

function estadoTono(status: RuntimeStatus | null): "ok" | "warn" | "idle" | "accent" {
  if (!status?.running) return "idle";
  if (status.activeSession) return "accent";
  return status.available ? "ok" : "warn";
}

function detalleTono(status: RuntimeStatus): "warn" | "accent" | undefined {
  if (!status.running) return undefined;
  if (!status.available) return "warn";
  return status.hasRelay ? "accent" : "warn";
}
