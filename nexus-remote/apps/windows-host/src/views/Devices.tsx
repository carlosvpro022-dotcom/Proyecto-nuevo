/** Mis dispositivos: lista blanca, permisos y revocacion. */

import { useState } from "react";

import {
  formatMoment,
  permissionLabel,
  type DeviceSummary,
  type PermissionProfile,
} from "../api";
import { Card, Empty, Notice, Pill } from "../components/ui";

export function Devices({
  devices,
  onRevoke,
  onDelete,
  onChangeProfile,
  onGoToPairing,
  busy,
}: {
  devices: DeviceSummary[];
  onRevoke: (id: string) => void;
  onDelete: (id: string) => void;
  onChangeProfile: (id: string, profile: PermissionProfile) => void;
  onGoToPairing: () => void;
  busy: boolean;
}) {
  const [confirming, setConfirming] = useState<string | null>(null);

  if (devices.length === 0) {
    return (
      <div className="stack">
        <Card title="Mis dispositivos">
          <Empty>
            Todavia no hay ningun dispositivo autorizado. Solo los dispositivos que
            empareje aqui podran conectarse a este PC.
          </Empty>
          <div className="row" style={{ justifyContent: "center" }}>
            <button type="button" className="btn btn--primary" onClick={onGoToPairing}>
              Emparejar un dispositivo
            </button>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <div className="stack">
      <Card
        title="Dispositivos autorizados"
        hint="Solo estos dispositivos pueden conectarse. Revocar surte efecto de inmediato."
        actions={
          <button type="button" className="btn btn--primary" onClick={onGoToPairing}>
            Emparejar otro
          </button>
        }
      >
        <table className="table">
          <thead>
            <tr>
              <th>Dispositivo</th>
              <th>Huella</th>
              <th>Emparejado</th>
              <th>Ultima conexion</th>
              <th>Permisos</th>
              <th>Estado</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {devices.map((device) => (
              <tr key={device.id}>
                <td>
                  <div style={{ fontWeight: 560 }}>{device.name}</div>
                  <div style={{ color: "var(--text-muted)", fontSize: 11.5 }}>
                    {device.platform}
                    {device.unattendedAccess && " · acceso desatendido"}
                  </div>
                </td>
                <td className="mono">{device.fingerprint}</td>
                <td style={{ color: "var(--text-secondary)" }}>
                  {formatMoment(device.pairedAtMs)}
                </td>
                <td style={{ color: "var(--text-secondary)" }}>
                  {formatMoment(device.lastSeenMs)}
                </td>
                <td>
                  <select
                    className="select"
                    value={profileOf(device)}
                    disabled={device.revoked || busy}
                    onChange={(event) =>
                      onChangeProfile(device.id, event.target.value as PermissionProfile)
                    }
                    aria-label={`Permisos de ${device.name}`}
                  >
                    <option value="view_only">Solo ver</option>
                    <option value="standard">Control completo</option>
                    <option value="full">Control y administracion</option>
                  </select>
                  <div
                    style={{ color: "var(--text-muted)", fontSize: 11, marginTop: 4 }}
                    title={device.permissions.map(permissionLabel).join(", ")}
                  >
                    {device.permissions.length} permiso(s)
                  </div>
                </td>
                <td>
                  {device.revoked ? (
                    <Pill tone="danger">Revocado</Pill>
                  ) : (
                    <Pill tone="ok">Activo</Pill>
                  )}
                </td>
                <td>
                  {confirming === device.id ? (
                    <div className="row">
                      <button
                        type="button"
                        className="btn btn--danger"
                        disabled={busy}
                        onClick={() => {
                          onDelete(device.id);
                          setConfirming(null);
                        }}
                      >
                        Confirmar borrado
                      </button>
                      <button
                        type="button"
                        className="btn btn--ghost"
                        onClick={() => setConfirming(null)}
                      >
                        Cancelar
                      </button>
                    </div>
                  ) : (
                    <div className="row">
                      {!device.revoked && (
                        <button
                          type="button"
                          className="btn btn--danger"
                          disabled={busy}
                          onClick={() => onRevoke(device.id)}
                        >
                          Revocar
                        </button>
                      )}
                      <button
                        type="button"
                        className="btn btn--ghost"
                        onClick={() => setConfirming(device.id)}
                      >
                        Borrar
                      </button>
                    </div>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>

      <Notice>
        <strong>Revocar</strong> deja el dispositivo en la lista para que quede
        constancia, pero le impide conectarse. <strong>Borrar</strong> elimina tambien
        su historial de sesiones y no se puede deshacer: para volver a usarlo habria
        que emparejarlo de nuevo.
      </Notice>
    </div>
  );
}

/** Deduce el perfil a partir de los permisos concedidos. */
function profileOf(device: DeviceSummary): PermissionProfile {
  if (device.permissions.includes("shutdown")) return "full";
  if (device.permissions.includes("control_pointer")) return "standard";
  return "view_only";
}
