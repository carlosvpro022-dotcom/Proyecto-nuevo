/**
 * Aplicacion del panel del host.
 *
 * Mantiene el estado que viene del backend y lo reparte entre las pantallas.
 * Si el backend no esta disponible (por ejemplo al abrir el panel en un
 * navegador sin Tauri) se muestra un aviso explicito en lugar de datos de
 * ejemplo.
 */

import { useCallback, useEffect, useState } from "react";

import {
  api,
  isHostAvailable,
  toHostError,
  type DeviceSummary,
  type FeatureStatus,
  type HostSummary,
  type EngineStatus,
  type MonitorInfo,
  type PairingView,
  type PermissionEntry,
  type PermissionProfile,
  type SystemStatus,
} from "./api";
import { Shell, type NavEntry } from "./components/Shell";
import { Card, Notice, Pill, Spinner } from "./components/ui";
import { Dashboard } from "./views/Dashboard";
import { Devices } from "./views/Devices";
import { Pairing } from "./views/Pairing";
import { Files, History } from "./views/Pendientes";
import { RemoteDesktop } from "./views/RemoteDesktop";
import { Security } from "./views/Security";
import { Settings } from "./views/Settings";

type ViewId =
  | "dashboard"
  | "devices"
  | "pairing"
  | "desktop"
  | "files"
  | "history"
  | "security"
  | "settings";

const TITLES: Record<ViewId, { title: string; subtitle: string }> = {
  dashboard: {
    title: "Panel",
    subtitle: "Estado de este host y de las funciones disponibles",
  },
  devices: {
    title: "Mis dispositivos",
    subtitle: "Solo los dispositivos de esta lista pueden conectarse",
  },
  pairing: {
    title: "Conexion remota",
    subtitle: "Autorizar un iPhone o iPad nuevo",
  },
  desktop: {
    title: "Escritorio remoto",
    subtitle: "Transmision de la pantalla y control del equipo",
  },
  files: {
    title: "Transferencia de archivos",
    subtitle: "Explorador remoto de las carpetas compartidas",
  },
  history: {
    title: "Historial de sesiones",
    subtitle: "Quien se conecto, cuando y durante cuanto tiempo",
  },
  security: {
    title: "Seguridad",
    subtitle: "Que protege el sistema y que queda pendiente",
  },
  settings: {
    title: "Configuracion",
    subtitle: "Servidor propio, calidad e identidad del host",
  },
};

/** Estado de carga de los datos del backend. */
interface HostData {
  summary: HostSummary;
  status: SystemStatus;
  devices: DeviceSummary[];
  features: FeatureStatus[];
  catalog: PermissionEntry[];
  pairing: PairingView | null;
  engine: EngineStatus | null;
  /** Pantallas detectadas. Vacio si la plataforma no las puede enumerar. */
  monitors: MonitorInfo[];
  /** Por que no se pudieron enumerar, si es el caso. */
  monitorsError: string | null;
}

export function App() {
  const [view, setView] = useState<ViewId>("dashboard");
  const [data, setData] = useState<HostData | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    try {
      const [summary, status, devices, features, catalog, pairing, engine] =
        await Promise.all([
          api.hostSummary(),
          api.systemStatus(),
          api.listDevices(),
          api.featureMatrix(),
          api.permissionCatalog(),
          api.currentPairing(),
          api.engineStatus(),
        ]);

      // La enumeracion de monitores falla a proposito en las plataformas sin
      // backend. No es motivo para dejar el panel sin datos: se guarda el
      // motivo y se muestra en su pantalla.
      let monitors: MonitorInfo[] = [];
      let monitorsError: string | null = null;
      try {
        monitors = await api.listMonitors();
      } catch (error) {
        monitorsError = toHostError(error).message;
      }

      setData({
        summary,
        status,
        devices,
        features,
        catalog,
        pairing,
        engine,
        monitors,
        monitorsError,
      });
      setLoadError(null);
    } catch (error) {
      setLoadError(toHostError(error).message);
    }
  }, []);

  useEffect(() => {
    if (!isHostAvailable()) {
      setLoadError("El panel no esta conectado al backend del host.");
      return;
    }
    void reload();
    // El estado del equipo cambia por si solo (memoria, tiempo encendido), asi
    // que se refresca periodicamente. 5 segundos es suficiente y no molesta.
    const timer = window.setInterval(() => void reload(), 5_000);
    return () => window.clearInterval(timer);
  }, [reload]);

  /** Ejecuta una accion, muestra su error si falla y recarga el estado. */
  const run = useCallback(
    async (action: () => Promise<unknown>) => {
      setBusy(true);
      setActionError(null);
      try {
        await action();
        await reload();
      } catch (error) {
        setActionError(toHostError(error).message);
      } finally {
        setBusy(false);
      }
    },
    [reload],
  );

  if (!isHostAvailable() || (loadError && !data)) {
    return <Disconnected message={loadError ?? "Sin conexion con el host."} />;
  }

  if (!data) {
    return (
      <div className="shell" style={{ gridTemplateColumns: "1fr", placeItems: "center" }}>
        <Spinner label="Leyendo el estado del host…" />
      </div>
    );
  }

  const nav: NavEntry[] = [
    { id: "dashboard", label: "Panel", icon: "dashboard", ready: true },
    {
      id: "devices",
      label: "Mis dispositivos",
      icon: "devices",
      ready: true,
      ...(data.devices.length > 0 ? { badge: String(data.devices.length) } : {}),
    },
    { id: "pairing", label: "Conexion remota", icon: "connect", ready: true },
    {
      id: "desktop",
      label: "Escritorio remoto",
      icon: "desktop",
      ready: false,
      ...(data.monitors.length > 0 ? { badge: String(data.monitors.length) } : { badge: "E2" }),
    },
    { id: "files", label: "Archivos", icon: "files", ready: false, badge: "E5" },
    { id: "history", label: "Historial", icon: "history", ready: false, badge: "E3" },
    { id: "security", label: "Seguridad", icon: "security", ready: true },
    { id: "settings", label: "Configuracion", icon: "settings", ready: true },
  ];

  return (
    <Shell
      nav={nav}
      active={view}
      onNavigate={(id) => {
        setView(id as ViewId);
        setActionError(null);
      }}
      title={TITLES[view].title}
      subtitle={TITLES[view].subtitle}
      headerActions={
        <>
          {busy && <span className="spinner" aria-label="Procesando" />}
          <Pill tone={data.summary.signalingUrl ? "ok" : "warn"}>
            {data.summary.signalingUrl ? "Servidor configurado" : "Servidor sin configurar"}
          </Pill>
        </>
      }
      footer={
        <>
          <div style={{ color: "var(--text-secondary)", fontWeight: 560 }}>
            {data.status.hostname}
          </div>
          <div className="mono" style={{ marginTop: 2 }}>
            {data.summary.fingerprint}
          </div>
          <div style={{ marginTop: 4 }}>
            v{data.summary.appVersion} · protocolo v{data.summary.protocolVersion}
          </div>
        </>
      }
    >
      {actionError && (
        <div style={{ marginBottom: "var(--space-5)", maxWidth: 1100 }}>
          <Notice tone="danger">{actionError}</Notice>
        </div>
      )}

      {view === "dashboard" && (
        <Dashboard summary={data.summary} status={data.status} features={data.features} />
      )}

      {view === "devices" && (
        <Devices
          devices={data.devices}
          busy={busy}
          onRevoke={(id) => void run(() => api.revokeDevice(id))}
          onDelete={(id) => void run(() => api.deleteDevice(id))}
          onChangeProfile={(id, profile) => void run(() => api.setDeviceProfile(id, profile))}
          onGoToPairing={() => setView("pairing")}
        />
      )}

      {view === "pairing" && (
        <Pairing
          summary={data.summary}
          pairing={data.pairing}
          busy={busy}
          onOpen={(profile: PermissionProfile, unattended: boolean) =>
            void run(() => api.openPairing(profile, unattended))
          }
          onCancel={() => void run(() => api.cancelPairing())}
        />
      )}

      {view === "desktop" && (
        <RemoteDesktop
          features={data.features}
          engine={data.engine}
          monitors={data.monitors}
          monitorsError={data.monitorsError}
        />
      )}
      {view === "files" && <Files features={data.features} />}
      {view === "history" && <History features={data.features} />}

      {view === "security" && <Security devices={data.devices} catalog={data.catalog} />}

      {view === "settings" && (
        <Settings
          summary={data.summary}
          busy={busy}
          error={actionError}
          onSaveSignalingUrl={(url) => void run(() => api.setSignalingUrl(url))}
        />
      )}
    </Shell>
  );
}

/** Pantalla cuando no hay backend detras. No muestra datos inventados. */
function Disconnected({ message }: { message: string }) {
  return (
    <div className="shell" style={{ gridTemplateColumns: "1fr", placeItems: "center" }}>
      <div style={{ maxWidth: 560, padding: "var(--space-6)" }}>
        <Card title="Sin conexion con el host">
          <Notice tone="danger">{message}</Notice>
          <p style={{ color: "var(--text-secondary)", fontSize: 13, marginTop: "var(--space-4)" }}>
            Este panel es la interfaz de la aplicacion de escritorio: necesita el
            backend de Rust detras. Para ejecutarlo:
          </p>
          <pre
            className="mono"
            style={{
              marginTop: "var(--space-3)",
              padding: "var(--space-3)",
              background: "var(--surface-0)",
              borderRadius: "var(--radius-sm)",
              border: "1px solid var(--border-subtle)",
              userSelect: "text",
            }}
          >
            cd nexus-remote/apps/windows-host{"\n"}npm install{"\n"}npm run tauri dev
          </pre>
          <p style={{ color: "var(--text-muted)", fontSize: 12, marginTop: "var(--space-3)" }}>
            No se muestran datos de ejemplo a proposito: un panel con datos falsos seria
            indistinguible de uno roto.
          </p>
        </Card>
      </div>
    </div>
  );
}
