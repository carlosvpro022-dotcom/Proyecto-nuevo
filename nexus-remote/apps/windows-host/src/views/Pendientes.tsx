/**
 * Las pantallas de Archivos e Historial.
 *
 * Las dos siguen la misma regla que el resto del panel: dicen lo que de verdad
 * hay. Donde falta algo, dicen que falta y de que depende, en lugar de ensenar
 * una maqueta con datos de ejemplo que se pueda confundir con algo que ya
 * funciona.
 *
 * La de Archivos ya tiene funcionalidad detras: el motor del host lista,
 * descarga y sube, verificando el contenido. Lo que no hay todavia es el
 * cliente desde el que usarlo, y eso lo dice.
 */

import {
  formatMoment,
  type FeatureStatus,
  type SessionRow,
  type SharedFolder,
} from "../api";
import { Card, Notice } from "../components/ui";

function detailFor(features: FeatureStatus[], key: string, fallback: string): string {
  return features.find((f) => f.key === key)?.detail ?? fallback;
}

export function Files({
  features,
  folders,
}: {
  features: FeatureStatus[];
  folders: SharedFolder[];
}) {
  return (
    <div className="stack">
      <Card
        title="Carpetas que se comparten ahora mismo"
        hint="Es a lo unico que puede llegar un dispositivo remoto. Se eligen en Configuracion."
      >
        {folders.length === 0 ? (
          <Notice tone="warn">
            No se comparte ninguna carpeta, asi que ningun dispositivo puede
            ver archivos de este PC. Para compartir alguna, vaya a{" "}
            <em>Configuracion</em>.
          </Notice>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Nombre</th>
                <th>Carpeta en este PC</th>
                <th>Pueden escribir</th>
              </tr>
            </thead>
            <tbody>
              {folders.map((folder) => (
                <tr key={folder.path}>
                  <td>{folder.label}</td>
                  <td className="mono" style={{ wordBreak: "break-all" }}>
                    {folder.path}
                  </td>
                  <td className="mono">{folder.writable ? "si" : "no"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      <Card
        title="Que hace el host cuando llega un archivo"
        hint="Esto ya funciona y esta probado moviendo archivos de verdad por el canal cifrado."
      >
        <Notice tone="accent">
          {detailFor(
            features,
            "files",
            "Listar, descargar y subir funcionan.",
          )}
        </Notice>
        <table className="table" style={{ marginTop: "var(--space-4)" }}>
          <tbody>
            <tr>
              <td style={{ color: "var(--text-secondary)", width: 280 }}>
                Fuera de las carpetas compartidas
              </td>
              <td>
                Se rechaza. Una ruta prohibida y una que no existe dan el mismo
                motivo, para que nadie pueda averiguar que hay en el disco
                preguntando rutas.
              </td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>
                Un archivo que llega
              </td>
              <td>
                Se escribe aparte y solo aparece con su nombre definitivo
                cuando su contenido coincide con su resumen{" "}
                <span className="mono">SHA-256</span>. Si la sesion se corta,
                no queda nada.
              </td>
            </tr>
            <tr>
              <td style={{ color: "var(--text-secondary)" }}>
                Un archivo que ya existia
              </td>
              <td>
                No se sobrescribe: se rechaza y se dice por que.
              </td>
            </tr>
          </tbody>
        </table>
      </Card>

      <Card title="Lo que todavia no hay">
        <Notice tone="warn">
          Falta el cliente de iPhone y iPad desde el que explorar estas
          carpetas, que es la etapa 4 y necesita un Mac con Xcode. El motor del
          lado del host ya esta, pero sin cliente no hay nada que lo use.
        </Notice>
        <ul style={{ marginTop: "var(--space-4)", color: "var(--text-secondary)" }}>
          <li>Sobrescribir un archivo, pidiendo confirmacion en este PC.</li>
          <li>Reanudar una transferencia cortada en lugar de empezar de nuevo.</li>
          <li>Transferir carpetas enteras, no archivo por archivo.</li>
        </ul>
      </Card>
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
