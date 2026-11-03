/**
 * Pantallas que todavia no tienen funcionalidad detras.
 *
 * Cada una dice en que etapa llega, que falta exactamente y de que depende.
 * Es la alternativa honesta a una maqueta con datos de ejemplo: la persona ve
 * el sitio que ocupara la funcion, pero no puede confundirla con algo que ya
 * funciona.
 */

import { formatMoment, type FeatureStatus, type SessionRow } from "../api";
import { Card, Notice, Pending } from "../components/ui";

function detailFor(features: FeatureStatus[], key: string, fallback: string): string {
  return features.find((f) => f.key === key)?.detail ?? fallback;
}

function stageFor(features: FeatureStatus[], key: string, fallback: number): number {
  return features.find((f) => f.key === key)?.stage ?? fallback;
}

export function Files({ features }: { features: FeatureStatus[] }) {
  return (
    <div className="stack">
      <Pending
        title="Transferencia de archivos"
        stage={stageFor(features, "files", 5)}
        detail={detailFor(
          features,
          "files",
          "Falta mover bytes por el canal de datos.",
        )}
        dependsOn="El canal de datos ya existe; falta mover los bytes"
      />
    </div>
  );
}

export function History({ sessions }: { sessions: SessionRow[] }) {
  if (sessions.length === 0) {
    return (
      <Card
        title="Historial de sesiones"
        hint="Cada sesion atendida queda registrada con su dispositivo, su duracion y por que termino."
      >
        <Notice>
          Todavia no se ha atendido ninguna sesion. Empiece a atender en el{" "}
          <em>Panel</em> y conecte un dispositivo emparejado.
        </Notice>
      </Card>
    );
  }

  return (
    <Card
      title="Historial de sesiones"
      hint="De la mas reciente a la mas antigua. Las ultimas cien."
    >
      <table className="table">
        <thead>
          <tr>
            <th>Dispositivo</th>
            <th>Empezo</th>
            <th>Duracion</th>
            <th>Termino</th>
          </tr>
        </thead>
        <tbody>
          {sessions.map((sesion) => (
            <tr key={sesion.session_id}>
              <td>
                <div style={{ fontWeight: 560 }}>
                  {sesion.device_name ?? "Dispositivo eliminado"}
                </div>
                <div className="mono" style={{ color: "var(--text-muted)", fontSize: 11 }}>
                  {sesion.device_id.slice(0, 8)}
                </div>
              </td>
              <td style={{ color: "var(--text-secondary)" }}>
                {formatMoment(sesion.started_at_ms)}
              </td>
              <td className="mono">{duracion(sesion)}</td>
              <td style={{ color: "var(--text-secondary)" }}>
                {motivo(sesion.close_reason)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Card>
  );
}

/** Cuanto duro, o que sigue abierta. */
function duracion(sesion: SessionRow): string {
  if (sesion.ended_at_ms === null) return "en curso";
  const segundos = Math.max(0, Math.round((sesion.ended_at_ms - sesion.started_at_ms) / 1000));
  if (segundos < 60) return `${segundos} s`;
  const minutos = Math.floor(segundos / 60);
  if (minutos < 60) return `${minutos} min`;
  return `${Math.floor(minutos / 60)} h ${minutos % 60} min`;
}

/** Traduce el motivo de cierre que guarda la base de datos. */
function motivo(reason: string | null): string {
  switch (reason) {
    case null:
      return "sigue abierta";
    case "user_requested":
      return "la cerro una persona";
    case "peer_closed":
      return "se desconecto el dispositivo";
    case "authentication_failed":
      return "no se pudo comprobar la identidad";
    case "network_lost":
      return "se perdio la red";
    case "device_revoked":
      return "se revoco el dispositivo";
    case "host_policy":
      return "la corto este equipo";
    case "internal_error":
      return "error interno";
    default:
      return reason;
  }
}
