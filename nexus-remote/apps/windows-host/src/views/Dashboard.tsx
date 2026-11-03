/** Panel principal: estado real del host y que funciones estan listas. */

import {
  formatDuration,
  formatMemory,
  type FeatureStatus,
  type HostSummary,
  type SystemStatus,
} from "../api";
import { Card, FeatureRow, Notice, Pill, Stat } from "../components/ui";

export function Dashboard({
  summary,
  status,
  features,
}: {
  summary: HostSummary;
  status: SystemStatus;
  features: FeatureStatus[];
}) {
  const probadas = features.filter((f) => f.state === "working").length;
  const sinProbar = features.filter((f) => f.state === "unverified").length;
  const memoryUsed = status.memory_total_mib - status.memory_available_mib;
  const memoryPercent =
    status.memory_total_mib > 0
      ? Math.round((memoryUsed / status.memory_total_mib) * 100)
      : 0;

  return (
    <div className="stack">
      {!summary.signalingUrl && (
        <Notice tone="warn">
          <strong>Falta configurar el servidor de senalizacion.</strong> Sin el, los
          dispositivos no pueden encontrar este PC desde fuera de la red local. Se
          configura en <em>Configuracion</em>, y el servidor se despliega siguiendo{" "}
          <span className="mono">docs/deployment/despliegue.md</span>.
        </Notice>
      )}

      <div className="grid grid--stats">
        <Stat
          label="Dispositivos autorizados"
          value={summary.activeDevices}
          note={
            summary.revokedDevices > 0
              ? `${summary.revokedDevices} revocado(s)`
              : "Ninguno revocado"
          }
        />
        <Stat
          label="Sesiones activas"
          value={0}
          note="El host aun no mantiene sesiones abiertas"
        />
        <Stat
          label="Memoria en uso"
          value={`${memoryPercent} %`}
          note={`${formatMemory(memoryUsed)} de ${formatMemory(status.memory_total_mib)}`}
        />
        <Stat
          label="Tiempo encendido"
          value={formatDuration(status.uptime_secs)}
          note={`${status.cpu_cores} nucleos logicos`}
        />
      </div>

      <Card
        title="Este equipo"
        hint="Datos leidos del sistema en este momento."
        actions={<Pill tone="ok">Host activo</Pill>}
      >
        <table className="table">
          <tbody>
            <tr>
              <td style={{ color: "var(--text-secondary)", width: 220 }}>Nombre del equipo</td>
              <td className="mono">{status.hostname}</td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Sistema operativo</td>
              <td>{status.os_description}</td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Identidad del host</td>
              <td className="mono">{summary.deviceId}</td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Huella de la clave publica</td>
              <td>
                <span className="mono" style={{ color: "var(--accent-bright)" }}>
                  {summary.fingerprint}
                </span>
                <div style={{ color: "var(--text-muted)", fontSize: 11.5, marginTop: 2 }}>
                  Compruebe que coincide con la que muestra el dispositivo al emparejar.
                </div>
              </td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Version</td>
              <td>
                NEXUS REMOTE {summary.appVersion}
                <span style={{ color: "var(--text-muted)" }}>
                  {" "}
                  · protocolo v{summary.protocolVersion}
                </span>
              </td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>Servidor de senalizacion</td>
              <td className="mono">{summary.signalingUrl ?? "sin configurar"}</td>
            </tr>
          </tbody>
        </table>
      </Card>

      {sinProbar > 0 && (
        <Notice tone="warn">
          <strong>{sinProbar} funciones estan escritas pero sin comprobar.</strong> El
          codigo de captura y control de Windows compila para su plataforma, pero
          todavia no se ha ejecutado en un PC con Windows. Hasta entonces no se debe
          dar por bueno.
        </Notice>
      )}

      <Card
        title="Estado de las funciones"
        hint={
          `${probadas} de ${features.length} probadas ejecutandolas` +
          (sinProbar > 0
            ? `, ${sinProbar} escritas para Windows pero sin ejecutar alli todavia.`
            : ".")
        }
      >
        <div className="feature-list">
          {features.map((feature) => (
            <FeatureRow key={feature.key} feature={feature} />
          ))}
        </div>
      </Card>
    </div>
  );
}
