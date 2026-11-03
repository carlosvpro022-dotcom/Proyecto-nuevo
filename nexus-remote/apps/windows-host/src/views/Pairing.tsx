/**
 * Conexion remota: emparejar un dispositivo nuevo.
 *
 * Es la pantalla que convierte un iPhone o un iPad desconocido en un
 * dispositivo autorizado. El codigo se muestra aqui y caduca solo.
 */

import { useEffect, useState } from "react";
import QRCode from "qrcode";

import type { HostSummary, PairingView, PermissionProfile } from "../api";
import { Card, Notice, Pill } from "../components/ui";

const PROFILES: { value: PermissionProfile; label: string; detail: string }[] = [
  {
    value: "view_only",
    label: "Solo ver",
    detail: "Ve la pantalla y la informacion del equipo. No puede controlar nada.",
  },
  {
    value: "standard",
    label: "Control completo",
    detail:
      "Raton, teclado, audio, portapapeles y lectura de archivos. Sin administracion del equipo.",
  },
  {
    value: "full",
    label: "Control y administracion",
    detail:
      "Todo lo anterior mas escritura de archivos y bloquear, reiniciar o apagar el equipo.",
  },
];

export function Pairing({
  summary,
  pairing,
  onOpen,
  onCancel,
  busy,
}: {
  summary: HostSummary;
  pairing: PairingView | null;
  onOpen: (profile: PermissionProfile, unattended: boolean) => void;
  onCancel: () => void;
  busy: boolean;
}) {
  const [profile, setProfile] = useState<PermissionProfile>("standard");
  const [unattended, setUnattended] = useState(false);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [remaining, setRemaining] = useState(0);

  // El QR se genera desde el contenido que entrega el backend. Si falla, se
  // muestra el codigo escrito: nunca se deja la pantalla sin forma de emparejar.
  useEffect(() => {
    if (!pairing) {
      setQrDataUrl(null);
      return;
    }
    let cancelled = false;
    QRCode.toDataURL(pairing.qrPayload, {
      errorCorrectionLevel: "M",
      margin: 1,
      width: 368,
      color: { dark: "#0a0b0d", light: "#ffffff" },
    })
      .then((url) => {
        if (!cancelled) setQrDataUrl(url);
      })
      .catch(() => {
        if (!cancelled) setQrDataUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [pairing]);

  // Cuenta atras hasta la caducidad, recalculada desde el reloj para que no se
  // desvie si la pestana se queda en segundo plano.
  useEffect(() => {
    if (!pairing) {
      setRemaining(0);
      return;
    }
    const tick = () => {
      setRemaining(Math.max(0, Math.ceil((pairing.expiresAtMs - Date.now()) / 1000)));
    };
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [pairing]);

  const expired = pairing !== null && remaining === 0;

  return (
    <div className="stack">
      {!summary.signalingUrl && (
        <Notice tone="warn">
          El codigo se puede generar, pero el dispositivo no podra conectarse hasta
          que configure la URL del servidor de senalizacion en{" "}
          <em>Configuracion</em>.
        </Notice>
      )}

      {pairing && !expired ? (
        <Card
          title="Codigo de emparejamiento activo"
          hint="Introduzcalo en la aplicacion del iPhone o iPad, o escanee el codigo QR."
          actions={
            <>
              <Pill tone={remaining < 60 ? "warn" : "accent"}>
                Caduca en{" "}
                <span className="countdown">
                  {Math.floor(remaining / 60)}:{String(remaining % 60).padStart(2, "0")}
                </span>
              </Pill>
              <button
                type="button"
                className="btn"
                onClick={onCancel}
                disabled={busy}
              >
                Cancelar
              </button>
            </>
          }
        >
          <div className="row" style={{ gap: "var(--space-6)", alignItems: "flex-start" }}>
            <div className="field" style={{ gap: "var(--space-4)" }}>
              <div className="code-display" aria-label="Codigo de emparejamiento">
                {pairing.code}
              </div>
              <div className="field__hint">
                Intentos restantes: {pairing.remainingAttempts}. Tras agotarlos el
                codigo deja de valer y hay que generar otro.
              </div>
              <div className="field__hint">
                Compruebe que el dispositivo muestra esta huella:{" "}
                <span className="mono" style={{ color: "var(--accent-bright)" }}>
                  {summary.fingerprint}
                </span>
              </div>
            </div>

            {qrDataUrl ? (
              <div className="field" style={{ alignItems: "center" }}>
                <div className="qr">
                  <img src={qrDataUrl} alt="Codigo QR con la invitacion de emparejamiento" />
                </div>
                <div className="field__hint">Escanear desde la camara del dispositivo</div>
              </div>
            ) : (
              <Notice>
                No se pudo generar el codigo QR. Introduzca el codigo a mano.
              </Notice>
            )}
          </div>
        </Card>
      ) : (
        <Card
          title="Emparejar un dispositivo"
          hint="Elija que podra hacer el dispositivo. Siempre se puede cambiar despues."
        >
          {expired && (
            <Notice tone="warn">
              El codigo anterior caduco sin usarse. Genere otro.
            </Notice>
          )}

          <div className="grid grid--two" style={{ marginTop: expired ? "var(--space-4)" : 0 }}>
            {PROFILES.map((option) => (
              <label
                key={option.value}
                className="checkbox"
                style={{
                  padding: "var(--space-4)",
                  borderRadius: "var(--radius)",
                  border: `1px solid ${
                    profile === option.value ? "var(--accent)" : "var(--border-subtle)"
                  }`,
                  background:
                    profile === option.value ? "var(--accent-wash)" : "var(--surface-2)",
                }}
              >
                <input
                  type="radio"
                  name="profile"
                  value={option.value}
                  checked={profile === option.value}
                  onChange={() => setProfile(option.value)}
                />
                <span>
                  <span style={{ fontWeight: 560, display: "block" }}>{option.label}</span>
                  <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>
                    {option.detail}
                  </span>
                </span>
              </label>
            ))}
          </div>

          <div style={{ marginTop: "var(--space-5)" }}>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={unattended}
                onChange={(event) => setUnattended(event.target.checked)}
              />
              <span>
                <span style={{ fontWeight: 560, display: "block" }}>
                  Permitir acceso desatendido
                </span>
                <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>
                  El dispositivo podra conectarse sin que nadie acepte la sesion en este
                  PC. Util para acceder de viaje; concedalo solo a un dispositivo que
                  lleve siempre encima y tenga bloqueo de pantalla.
                </span>
              </span>
            </label>
          </div>

          {unattended && (
            <div style={{ marginTop: "var(--space-4)" }}>
              <Notice tone="warn">
                Con acceso desatendido, quien tenga ese dispositivo desbloqueado entra en
                este PC sin pedir permiso. Revoquelo desde <em>Mis dispositivos</em> si lo
                pierde.
              </Notice>
            </div>
          )}

          <div className="row" style={{ marginTop: "var(--space-5)" }}>
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => onOpen(profile, unattended)}
              disabled={busy}
            >
              Generar codigo
            </button>
            <span className="field__hint">
              El codigo es valido 5 minutos y de un solo uso.
            </span>
          </div>
        </Card>
      )}
    </div>
  );
}
