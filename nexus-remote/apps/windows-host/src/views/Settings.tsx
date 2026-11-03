/** Configuracion: servidor propio y datos del host. */

import { useState } from "react";

import type { HostSummary, SignalingProbe } from "../api";
import { Card, Notice } from "../components/ui";

export function Settings({
  summary,
  onSaveSignalingUrl,
  onCheckSignaling,
  probe,
  checking,
  busy,
  error,
}: {
  summary: HostSummary;
  onSaveSignalingUrl: (url: string) => void;
  onCheckSignaling: (enrollmentToken: string) => void;
  probe: SignalingProbe | null;
  checking: boolean;
  busy: boolean;
  error: string | null;
}) {
  const [url, setUrl] = useState(summary.signalingUrl ?? "");
  const [token, setToken] = useState("");
  const dirty = url.trim() !== (summary.signalingUrl ?? "");
  const configured = (summary.signalingUrl ?? "").length > 0;

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
            placeholder="wss://nexus.midominio.com/v1/signaling"
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

      <Card
        title="Comprobar la conexion"
        hint="Se conecta de verdad al servidor, se autentica y se declara disponible. No deja la conexion abierta."
      >
        {!configured && (
          <Notice tone="warn">
            Primero hay que guardar la direccion del servidor.
          </Notice>
        )}

        <div className="field" style={{ maxWidth: 560 }}>
          <label className="field__label" htmlFor="enrollment-token">
            Testigo de alta (solo la primera vez)
          </label>
          <input
            id="enrollment-token"
            className="input"
            type="password"
            value={token}
            placeholder="Valor de NEXUS_HOST_ENROLLMENT_TOKEN"
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setToken(event.target.value)}
          />
          <span className="field__hint">
            El servidor lo exige la primera vez que este PC se conecta, para que
            nadie pueda registrar su identificador antes que el. No se guarda en
            ningun sitio: se usa una vez y se descarta.
          </span>
        </div>

        <div className="row" style={{ marginTop: "var(--space-4)" }}>
          <button
            type="button"
            className="btn btn--primary"
            disabled={!configured || checking}
            onClick={() => onCheckSignaling(token)}
          >
            {checking ? "Comprobando..." : "Probar conexion"}
          </button>
        </div>

        {probe && (
          <div style={{ marginTop: "var(--space-4)" }} className="stack">
            <Notice tone={probe.connected ? (probe.hasRelay ? "accent" : "warn") : "danger"}>
              {probe.detail}
            </Notice>
            {probe.connected && (
              <table className="table">
                <tbody>
                  <tr>
                    <td style={{ color: "var(--text-secondary)", width: 280 }}>
                      Servidores de red recibidos
                    </td>
                    <td className="mono">{probe.iceServers}</td>
                  </tr>
                  <tr>
                    <td style={{ color: "var(--text-secondary)" }}>
                      Retransmision (TURN)
                    </td>
                    <td className="mono">{probe.hasRelay ? "si" : "no"}</td>
                  </tr>
                  <tr>
                    <td style={{ color: "var(--text-secondary)" }}>
                      Alcanzable desde otra red
                    </td>
                    <td className="mono">
                      {probe.supportsRemoteNetworks ? "si" : "no"}
                    </td>
                  </tr>
                </tbody>
              </table>
            )}
          </div>
        )}
      </Card>

      <Card title="Calidad de transmision" hint="Se aplicara cuando haya transmision de pantalla (etapa 5).">
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
