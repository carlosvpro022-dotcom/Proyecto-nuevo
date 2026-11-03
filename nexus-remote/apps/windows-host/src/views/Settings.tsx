/** Configuracion: servidor propio y datos del host. */

import { useState } from "react";

import type { HostSummary } from "../api";
import { Card, Notice } from "../components/ui";

export function Settings({
  summary,
  onSaveSignalingUrl,
  busy,
  error,
}: {
  summary: HostSummary;
  onSaveSignalingUrl: (url: string) => void;
  busy: boolean;
  error: string | null;
}) {
  const [url, setUrl] = useState(summary.signalingUrl ?? "");
  const dirty = url.trim() !== (summary.signalingUrl ?? "");

  return (
    <div className="stack">
      <Card
        title="Servidor de senalizacion"
        hint="El servidor propio que permite a los dispositivos encontrar este PC desde cualquier red."
      >
        <div className="field" style={{ maxWidth: 560 }}>
          <label className="field__label" htmlFor="signaling-url">
            URL del servidor
          </label>
          <input
            id="signaling-url"
            className="input"
            type="text"
            value={url}
            placeholder="wss://nexus.midominio.com/signaling"
            spellCheck={false}
            onChange={(event) => setUrl(event.target.value)}
          />
          <span className="field__hint">
            Debe empezar por <span className="mono">wss://</span>. Sin TLS, la
            senalizacion expondria los identificadores de los dispositivos y los
            candidatos de red.
          </span>
        </div>

        {error && (
          <div style={{ marginTop: "var(--space-4)" }}>
            <Notice tone="danger">{error}</Notice>
          </div>
        )}

        <div className="row" style={{ marginTop: "var(--space-4)" }}>
          <button
            type="button"
            className="btn btn--primary"
            disabled={!dirty || busy}
            onClick={() => onSaveSignalingUrl(url)}
          >
            Guardar
          </button>
          {dirty && (
            <button
              type="button"
              className="btn btn--ghost"
              onClick={() => setUrl(summary.signalingUrl ?? "")}
            >
              Descartar cambios
            </button>
          )}
        </div>
      </Card>

      <Card title="Calidad de transmision" hint="Se aplicara cuando exista la sesion remota (etapa 3).">
        <Notice>
          Los perfiles de calidad (fluido, equilibrado, nitido y automatico) ya estan
          definidos y validados en el codigo compartido, con sus limites de
          fotogramas, tasa de bits y escala. Esta pantalla los expondra en cuanto
          haya una transmision que configurar.
        </Notice>
      </Card>

      <Card title="Identidad de este host">
        <table className="table">
          <tbody>
            <tr>
              <td style={{ color: "var(--text-secondary)", width: 240 }}>Identificador</td>
              <td className="mono">{summary.deviceId}</td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Huella corta</td>
              <td className="mono" style={{ color: "var(--accent-bright)" }}>
                {summary.fingerprint}
              </td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Huella completa (SHA-256)</td>
              <td className="mono" style={{ wordBreak: "break-all" }}>
                {summary.fingerprintFull}
              </td>
            </tr>
          </tbody>
        </table>
        <div style={{ marginTop: "var(--space-4)" }}>
          <Notice tone="warn">
            La clave privada de este host se guarda en disco sin cifrar. En Windows los
            permisos de archivo no la protegen de otro programa del mismo usuario:
            falta cifrarla con DPAPI, que esta recogido como tarea de la etapa 6.
          </Notice>
        </div>
      </Card>
    </div>
  );
}
