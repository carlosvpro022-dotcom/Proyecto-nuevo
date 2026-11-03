/** Seguridad: que protege el sistema y que falta por proteger. */

import { permissionLabel, type DeviceSummary, type PermissionEntry } from "../api";
import { Card, Notice, Pill } from "../components/ui";

/**
 * Medidas implementadas, con el modulo donde vive cada una.
 *
 * La lista refleja el codigo real: cada entrada corresponde a un modulo con
 * pruebas. No es una lista de intenciones.
 */
const IMPLEMENTED = [
  {
    name: "Identidad propia por dispositivo",
    detail: "Par de claves Ed25519 generado en cada dispositivo. La clave privada no sale de el.",
    module: "nexus-security::identity",
  },
  {
    name: "Autenticacion mutua",
    detail:
      "Reto de un solo uso firmado por ambas partes, con el papel del firmante dentro de la firma para evitar reflexion.",
    module: "nexus-security::challenge",
  },
  {
    name: "Verificacion estricta de firmas",
    detail: "Se rechazan claves de orden pequeno y firmas maleables, ya al registrar el dispositivo.",
    module: "nexus-security::identity",
  },
  {
    name: "Emparejamiento con codigo temporal",
    detail: "Codigo de 40 bits derivado con Argon2id, 5 minutos de validez y 5 intentos.",
    module: "nexus-security::pairing",
  },
  {
    name: "Proteccion contra reenvio",
    detail: "Ventana de reloj de 30 segundos mas registro de nonces ya usados, con memoria acotada.",
    module: "nexus-security::replay",
  },
  {
    name: "Limite de intentos con bloqueo creciente",
    detail: "Cada tanda de fallos duplica la espera. Esperar a que expire no reinicia la escalada.",
    module: "nexus-security::ratelimit",
  },
  {
    name: "Lista blanca y revocacion",
    detail: "Un dispositivo que no esta en la lista, o esta revocado, no abre sesion aunque su firma sea valida.",
    module: "nexus-security::registry",
  },
  {
    name: "Permisos por operacion",
    detail: "Cada orden del canal de control declara el permiso que necesita, comprobado en un solo punto.",
    module: "nexus-core::protocol::control",
  },
  {
    name: "Confirmacion de operaciones destructivas",
    detail: "Reiniciar y apagar exigen confirmacion en el host aunque el permiso este concedido.",
    module: "nexus-core::permissions",
  },
  {
    name: "Restriccion de acceso al disco",
    detail:
      "Solo las carpetas declaradas. Se rechaza el recorrido de directorios, los enlaces que escapan y los nombres reservados de Windows.",
    module: "nexus-file-transfer::access",
  },
  {
    name: "Credenciales TURN temporales",
    detail: "Derivadas por HMAC-SHA1 con caducidad. Filtrar una no da acceso permanente al relay.",
    module: "nexus-signaling::turn",
  },
  {
    name: "Historial de conexiones",
    detail: "Cada sesion queda registrada con su dispositivo, su duracion y su motivo de cierre.",
    module: "host::store",
  },
];

/** Lo que falta, dicho sin rodeos. */
const PENDING = [
  {
    name: "Cifrado de la clave privada del host",
    detail: "Falta protegerla con DPAPI en Windows. Hoy esta en un archivo sin cifrar.",
    stage: 6,
  },
  {
    name: "Cifrado de medios extremo a extremo en marcha",
    detail: "DTLS-SRTP esta elegido y documentado, pero no hay sesion WebRTC todavia.",
    stage: 3,
  },
  {
    name: "TLS en la senalizacion",
    detail: "El servidor lo delega en un proxy inverso; falta desplegarlo y probarlo de extremo a extremo.",
    stage: 3,
  },
  {
    name: "Revision de seguridad completa",
    detail: "Pruebas de penetracion sobre el sistema en marcha.",
    stage: 6,
  },
];

export function Security({
  devices,
  catalog,
}: {
  devices: DeviceSummary[];
  catalog: PermissionEntry[];
}) {
  const unattended = devices.filter((d) => d.unattendedAccess && !d.revoked);
  const sensitive = devices.filter(
    (d) => !d.revoked && d.permissions.some((p) => isSensitive(p, catalog)),
  );

  return (
    <div className="stack">
      {unattended.length > 0 && (
        <Notice tone="warn">
          <strong>
            {unattended.length} dispositivo(s) con acceso desatendido:
          </strong>{" "}
          {unattended.map((d) => d.name).join(", ")}. Pueden entrar sin que nadie
          acepte la sesion en este PC.
        </Notice>
      )}

      <div className="grid grid--stats">
        <div className="stat">
          <span className="stat__label">Dispositivos activos</span>
          <span className="stat__value">{devices.filter((d) => !d.revoked).length}</span>
          <span className="stat__note">{devices.filter((d) => d.revoked).length} revocado(s)</span>
        </div>
        <div className="stat">
          <span className="stat__label">Con permisos sensibles</span>
          <span className="stat__value">{sensitive.length}</span>
          <span className="stat__note">Escritura, teclado o administracion</span>
        </div>
        <div className="stat">
          <span className="stat__label">Acceso desatendido</span>
          <span className="stat__value">{unattended.length}</span>
          <span className="stat__note">Entran sin confirmacion</span>
        </div>
        <div className="stat">
          <span className="stat__label">Intentos fallidos</span>
          <span className="stat__value">—</span>
          <span className="stat__note">Se contara con la etapa 3</span>
        </div>
      </div>

      <Card
        title="Medidas implementadas"
        hint="Cada entrada corresponde a un modulo del codigo con sus propias pruebas."
      >
        <div className="feature-list">
          {IMPLEMENTED.map((item) => (
            <div className="feature" key={item.name}>
              <div className="feature__body">
                <div className="feature__name">{item.name}</div>
                <div className="feature__detail">{item.detail}</div>
                <div className="mono" style={{ color: "var(--text-muted)", fontSize: 11 }}>
                  {item.module}
                </div>
              </div>
              <Pill tone="ok">Activa</Pill>
            </div>
          ))}
        </div>
      </Card>

      <Card
        title="Pendiente"
        hint="Lo que todavia no protege el sistema. Conviene conocerlo antes de usarlo en produccion."
      >
        <div className="feature-list">
          {PENDING.map((item) => (
            <div className="feature" key={item.name}>
              <div className="feature__body">
                <div className="feature__name">{item.name}</div>
                <div className="feature__detail">{item.detail}</div>
              </div>
              <Pill tone="warn">Etapa {item.stage}</Pill>
            </div>
          ))}
        </div>
      </Card>

      <Card
        title="Catalogo de permisos"
        hint="Los permisos que se pueden conceder a un dispositivo."
      >
        <table className="table">
          <thead>
            <tr>
              <th>Permiso</th>
              <th>Riesgo</th>
              <th>Confirmacion en el host</th>
            </tr>
          </thead>
          <tbody>
            {catalog.map((entry) => (
              <tr key={entry.name}>
                <td>
                  <div>{permissionLabel(entry.name)}</div>
                  <div className="mono" style={{ color: "var(--text-muted)", fontSize: 11 }}>
                    {entry.name}
                  </div>
                </td>
                <td>
                  {entry.sensitive ? (
                    <Pill tone="warn">Sensible</Pill>
                  ) : (
                    <Pill tone="idle">Normal</Pill>
                  )}
                </td>
                <td style={{ color: "var(--text-secondary)" }}>
                  {entry.needs_confirmation ? "Siempre" : "No"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
    </div>
  );
}

function isSensitive(permission: string, catalog: PermissionEntry[]): boolean {
  return catalog.find((entry) => entry.name === permission)?.sensitive ?? false;
}
