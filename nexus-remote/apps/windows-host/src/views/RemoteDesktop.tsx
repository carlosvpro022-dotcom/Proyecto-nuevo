/**
 * Escritorio remoto: motor de captura y control, y las pantallas detectadas.
 *
 * Lo que ya existe (etapa 2) se muestra con datos reales. Lo que falta para
 * poder ver de verdad el escritorio desde el iPad (etapa 3) se dice sin
 * rodeos, en lugar de enseñar un reproductor vacio.
 */

import {
  formatResolution,
  type EngineStatus,
  type FeatureStatus,
  type MonitorInfo,
} from "../api";
import { Card, Notice, Pending, Pill } from "../components/ui";

export function RemoteDesktop({
  features,
  engine,
  monitors,
  monitorsError,
}: {
  features: FeatureStatus[];
  engine: EngineStatus | null;
  monitors: MonitorInfo[];
  monitorsError: string | null;
}) {
  const capture = features.find((f) => f.key === "capture");
  const input = features.find((f) => f.key === "input");
  const webrtc = features.find((f) => f.key === "webrtc");

  return (
    <div className="stack">
      {engine && !engine.capture.working && (
        <Notice tone="warn">
          <strong>Este equipo no puede capturar la pantalla.</strong>{" "}
          {engine.capture.detail}
        </Notice>
      )}

      <Card
        title="Motor de escritorio remoto"
        hint="Comprobado en este equipo, ahora mismo."
      >
        <div className="feature-list">
          <EngineRow
            name="Captura de pantalla"
            check={engine?.capture ?? null}
            note={capture?.detail}
          />
          <EngineRow
            name="Control de raton y teclado"
            check={engine?.input ?? null}
            note={input?.detail}
          />
        </div>
      </Card>

      <Card
        title="Pantallas detectadas"
        hint="Cada una se puede elegir como origen de la transmision."
      >
        {monitorsError ? (
          <Notice tone="warn">{monitorsError}</Notice>
        ) : monitors.length === 0 ? (
          <Notice>No se detecto ninguna pantalla.</Notice>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Pantalla</th>
                <th>Resolucion</th>
                <th>Posicion</th>
                <th>Refresco</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {monitors.map((monitor) => (
                <tr key={monitor.index}>
                  <td>
                    <div style={{ fontWeight: 560 }}>{monitor.name}</div>
                    <div style={{ color: "var(--text-muted)", fontSize: 11.5 }}>
                      Indice {monitor.index}
                    </div>
                  </td>
                  <td className="mono">{formatResolution(monitor)}</td>
                  <td className="mono" style={{ color: "var(--text-secondary)" }}>
                    {monitor.origin_x}, {monitor.origin_y}
                  </td>
                  <td style={{ color: "var(--text-secondary)" }}>
                    {monitor.refresh_hz ? `${monitor.refresh_hz} Hz` : "—"}
                  </td>
                  <td>{monitor.is_primary && <Pill tone="accent">Principal</Pill>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      <Pending
        title="Ver y controlar el escritorio desde el iPad"
        stage={webrtc?.stage ?? 3}
        detail={
          webrtc?.detail ??
          "Falta la negociacion ICE y los canales de medios y datos."
        }
        dependsOn="Servidor de senalizacion en marcha y servidor TURN desplegado"
      />
    </div>
  );
}

function EngineRow({
  name,
  check,
  note,
}: {
  name: string;
  check: { working: boolean; detail: string } | null;
  note?: string;
}) {
  return (
    <div className="feature">
      <div className="feature__body">
        <div className="feature__name">{name}</div>
        <div className="feature__detail">{check ? check.detail : "Sin comprobar"}</div>
        {note && (
          <div style={{ color: "var(--text-muted)", fontSize: 11, marginTop: 2 }}>{note}</div>
        )}
      </div>
      {check ? (
        <Pill tone={check.working ? "ok" : "danger"}>
          {check.working ? "Disponible" : "No disponible"}
        </Pill>
      ) : (
        <Pill tone="idle">Sin datos</Pill>
      )}
    </div>
  );
}
