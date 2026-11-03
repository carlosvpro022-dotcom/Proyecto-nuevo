/**
 * Capa de acceso al backend del host.
 *
 * Todos los tipos de aqui son el espejo de las estructuras de
 * `src-tauri/src/commands.rs`. Si una cambia y la otra no, `tsc` no lo detecta,
 * asi que la regla es cambiar ambas en el mismo commit; las pruebas de
 * integracion de `tests/` comprueban los nombres de los comandos.
 *
 * Cuando el panel se abre en un navegador normal (`npm run dev` sin Tauri) no
 * hay backend. En ese caso **no se inventan datos**: `isHostAvailable` devuelve
 * `false` y la interfaz muestra que no esta conectada al host. Un panel con
 * datos de ejemplo seria indistinguible de uno roto.
 */

import { invoke } from "@tauri-apps/api/core";

export interface HostSummary {
  deviceId: string;
  fingerprint: string;
  fingerprintFull: string;
  hostname: string;
  osDescription: string;
  appVersion: string;
  protocolVersion: number;
  signalingUrl: string | null;
  activeDevices: number;
  revokedDevices: number;
}

export interface SystemStatus {
  hostname: string;
  os_description: string;
  cpu_cores: number;
  memory_total_mib: number;
  memory_available_mib: number;
  uptime_secs: number;
  session_locked: boolean;
}

export interface DeviceSummary {
  id: string;
  name: string;
  platform: string;
  fingerprint: string;
  pairedAtMs: number;
  lastSeenMs: number | null;
  revoked: boolean;
  unattendedAccess: boolean;
  permissions: string[];
}

export interface PairingView {
  pairingId: string;
  code: string;
  qrPayload: string;
  expiresAtMs: number;
  ttlSecs: number;
  remainingAttempts: number;
}

/**
 * Hasta donde esta verificada una funcion. Corresponde a los niveles del
 * ADR-0007: `working` se ha ejecutado y probado, `unverified` esta escrito y
 * compila para Windows pero nunca se ha ejecutado alli, y `pending` todavia no
 * existe.
 */
export type FeatureState = "working" | "unverified" | "pending";

export interface FeatureStatus {
  key: string;
  label: string;
  state: FeatureState;
  stage: number;
  detail: string;
}

/** Resultado de comprobar una capacidad en este equipo concreto. */
export interface EngineCheck {
  working: boolean;
  detail: string;
}

/** Que puede hacer de verdad este equipo ahora mismo. */
export interface EngineStatus {
  capture: EngineCheck;
  input: EngineCheck;
}

/** Una pantalla detectada en el host. */
export interface MonitorInfo {
  index: number;
  name: string;
  width: number;
  height: number;
  origin_x: number;
  origin_y: number;
  scale_percent: number;
  is_primary: boolean;
  refresh_hz: number | null;
}

export interface PermissionEntry {
  name: string;
  sensitive: boolean;
  needs_confirmation: boolean;
}

export type PermissionProfile = "view_only" | "standard" | "full";

/** Sesion en curso, tal como la informa el bucle del host. */
export interface ActiveSession {
  sessionId: string;
  controller: string;
  permissions: string[];
  startedAtMs: number;
}

/** Que esta haciendo el bucle que atiende sesiones. */
export interface RuntimeStatus {
  running: boolean;
  available: boolean;
  hasRelay: boolean;
  detail: string;
  activeSession: ActiveSession | null;
  sessionsServed: number;
  sessionsRefused: number;
}

/** Una sesion del historial. */
export interface SessionRow {
  session_id: string;
  device_id: string;
  device_name: string | null;
  started_at_ms: number;
  ended_at_ms: number | null;
  close_reason: string | null;
}

/** Resultado de intentar conectarse al servidor de senalizacion. */
export interface SignalingProbe {
  connected: boolean;
  iceServers: number;
  hasRelay: boolean;
  supportsRemoteNetworks: boolean;
  detail: string;
}

/** Error del backend ya traducido a algo mostrable. */
export interface HostError {
  code: string;
  message: string;
}

/** Indica si el panel corre dentro de Tauri y por tanto tiene backend. */
export function isHostAvailable(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Convierte cualquier fallo en un error con forma conocida. */
export function toHostError(error: unknown): HostError {
  if (typeof error === "object" && error !== null && "code" in error && "message" in error) {
    const candidate = error as { code: unknown; message: unknown };
    if (typeof candidate.code === "string" && typeof candidate.message === "string") {
      return { code: candidate.code, message: candidate.message };
    }
  }
  if (error instanceof Error) {
    return { code: "unexpected", message: error.message };
  }
  return { code: "unexpected", message: String(error) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isHostAvailable()) {
    throw { code: "no_host", message: "El panel no esta conectado al backend del host." };
  }
  return invoke<T>(command, args);
}

/*
 * Los comandos de Rust devuelven los campos en snake_case (serde no los
 * renombra) y aqui se usan en camelCase, que es lo idiomatico en TypeScript.
 * La traduccion se hace en un solo sitio, al cruzar la frontera.
 */

interface RawHostSummary {
  device_id: string;
  fingerprint: string;
  fingerprint_full: string;
  hostname: string;
  os_description: string;
  app_version: string;
  protocol_version: number;
  signaling_url: string | null;
  active_devices: number;
  revoked_devices: number;
}

interface RawDeviceSummary {
  id: string;
  name: string;
  platform: string;
  fingerprint: string;
  paired_at_ms: number;
  last_seen_ms: number | null;
  revoked: boolean;
  unattended_access: boolean;
  permissions: string[];
}

interface RawActiveSession {
  session_id: string;
  controller: string;
  permissions: string[];
  started_at_ms: number;
}

interface RawRuntimeStatus {
  running: boolean;
  available: boolean;
  has_relay: boolean;
  detail: string;
  active_session: RawActiveSession | null;
  sessions_served: number;
  sessions_refused: number;
}

interface RawSignalingProbe {
  connected: boolean;
  ice_servers: number;
  has_relay: boolean;
  supports_remote_networks: boolean;
  detail: string;
}

interface RawPairingView {
  pairing_id: string;
  code: string;
  qr_payload: string;
  expires_at_ms: number;
  ttl_secs: number;
  remaining_attempts: number;
}

function mapSummary(raw: RawHostSummary): HostSummary {
  return {
    deviceId: raw.device_id,
    fingerprint: raw.fingerprint,
    fingerprintFull: raw.fingerprint_full,
    hostname: raw.hostname,
    osDescription: raw.os_description,
    appVersion: raw.app_version,
    protocolVersion: raw.protocol_version,
    signalingUrl: raw.signaling_url,
    activeDevices: raw.active_devices,
    revokedDevices: raw.revoked_devices,
  };
}

function mapDevice(raw: RawDeviceSummary): DeviceSummary {
  return {
    id: raw.id,
    name: raw.name,
    platform: raw.platform,
    fingerprint: raw.fingerprint,
    pairedAtMs: raw.paired_at_ms,
    lastSeenMs: raw.last_seen_ms,
    revoked: raw.revoked,
    unattendedAccess: raw.unattended_access,
    permissions: raw.permissions,
  };
}

function mapRuntime(raw: RawRuntimeStatus): RuntimeStatus {
  return {
    running: raw.running,
    available: raw.available,
    hasRelay: raw.has_relay,
    detail: raw.detail,
    activeSession: raw.active_session
      ? {
          sessionId: raw.active_session.session_id,
          controller: raw.active_session.controller,
          permissions: raw.active_session.permissions,
          startedAtMs: raw.active_session.started_at_ms,
        }
      : null,
    sessionsServed: raw.sessions_served,
    sessionsRefused: raw.sessions_refused,
  };
}

function mapProbe(raw: RawSignalingProbe): SignalingProbe {
  return {
    connected: raw.connected,
    iceServers: raw.ice_servers,
    hasRelay: raw.has_relay,
    supportsRemoteNetworks: raw.supports_remote_networks,
    detail: raw.detail,
  };
}

function mapPairing(raw: RawPairingView): PairingView {
  return {
    pairingId: raw.pairing_id,
    code: raw.code,
    qrPayload: raw.qr_payload,
    expiresAtMs: raw.expires_at_ms,
    ttlSecs: raw.ttl_secs,
    remainingAttempts: raw.remaining_attempts,
  };
}

export const api = {
  async hostSummary(): Promise<HostSummary> {
    return mapSummary(await call<RawHostSummary>("host_summary"));
  },

  async systemStatus(): Promise<SystemStatus> {
    return call<SystemStatus>("system_status");
  },

  async listDevices(): Promise<DeviceSummary[]> {
    const raw = await call<RawDeviceSummary[]>("list_devices");
    return raw.map(mapDevice);
  },

  async openPairing(
    profile: PermissionProfile,
    unattendedAccess: boolean,
  ): Promise<PairingView> {
    return mapPairing(
      await call<RawPairingView>("open_pairing", { profile, unattendedAccess }),
    );
  },

  async currentPairing(): Promise<PairingView | null> {
    const raw = await call<RawPairingView | null>("current_pairing");
    return raw ? mapPairing(raw) : null;
  },

  async cancelPairing(): Promise<boolean> {
    return call<boolean>("cancel_pairing");
  },

  async revokeDevice(deviceId: string): Promise<void> {
    return call<void>("revoke_device", { deviceId });
  },

  async deleteDevice(deviceId: string): Promise<void> {
    return call<void>("delete_device", { deviceId });
  },

  async setDeviceProfile(
    deviceId: string,
    profile: PermissionProfile,
  ): Promise<DeviceSummary> {
    return mapDevice(
      await call<RawDeviceSummary>("set_device_profile", { deviceId, profile }),
    );
  },

  async setSignalingUrl(url: string): Promise<void> {
    return call<void>("set_signaling_url", { url });
  },

  /**
   * Intenta conectarse al servidor configurado y cuenta lo que pasa.
   *
   * El testigo de alta solo hace falta la primera vez que este host se conecta
   * a un servidor. No se guarda en ningun sitio.
   */
  async checkSignaling(enrollmentToken?: string): Promise<SignalingProbe> {
    return mapProbe(
      await call<RawSignalingProbe>("check_signaling", {
        enrollmentToken: enrollmentToken ?? null,
      }),
    );
  },

  /**
   * Arranca el bucle que atiende sesiones.
   *
   * A partir de aqui el equipo esta localizable y un dispositivo emparejado
   * puede abrir sesion.
   */
  async startHost(enrollmentToken?: string): Promise<RuntimeStatus> {
    return mapRuntime(
      await call<RawRuntimeStatus>("start_host", {
        enrollmentToken: enrollmentToken ?? null,
      }),
    );
  },

  async stopHost(): Promise<RuntimeStatus> {
    return mapRuntime(await call<RawRuntimeStatus>("stop_host"));
  },

  async runtimeStatus(): Promise<RuntimeStatus> {
    return mapRuntime(await call<RawRuntimeStatus>("runtime_status"));
  },

  async sessionHistory(): Promise<SessionRow[]> {
    return call<SessionRow[]>("session_history");
  },

  async listMonitors(): Promise<MonitorInfo[]> {
    return call<MonitorInfo[]>("list_monitors");
  },

  async engineStatus(): Promise<EngineStatus> {
    return call<EngineStatus>("engine_status");
  },

  async featureMatrix(): Promise<FeatureStatus[]> {
    return call<FeatureStatus[]>("feature_matrix");
  },

  async permissionCatalog(): Promise<PermissionEntry[]> {
    return call<PermissionEntry[]>("permission_catalog");
  },
};

/* ------------------------------------------------------------- utilidades */

/** Formatea una marca en milisegundos como fecha local legible. */
export function formatMoment(ms: number | null): string {
  if (ms === null || ms === 0) return "—";
  return new Date(ms).toLocaleString("es-ES", {
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Formatea una duracion en segundos como texto corto. */
export function formatDuration(totalSeconds: number): string {
  if (totalSeconds < 60) return `${totalSeconds} s`;
  const days = Math.floor(totalSeconds / 86_400);
  const hours = Math.floor((totalSeconds % 86_400) / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  if (days > 0) return `${days} d ${hours} h`;
  if (hours > 0) return `${hours} h ${minutes} min`;
  return `${minutes} min`;
}

/** Formatea mebibytes como gibibytes con un decimal. */
export function formatMemory(mib: number): string {
  if (mib < 1024) return `${mib} MiB`;
  return `${(mib / 1024).toFixed(1)} GiB`;
}

/** Traduce el nombre interno de un permiso a texto para la persona. */
export function permissionLabel(name: string): string {
  const labels: Record<string, string> = {
    view_screen: "Ver la pantalla",
    control_pointer: "Mover el raton",
    control_keyboard: "Usar el teclado",
    receive_audio: "Recibir audio",
    clipboard_read: "Leer el portapapeles",
    clipboard_write: "Escribir en el portapapeles",
    file_read: "Leer archivos",
    file_write: "Escribir archivos",
    system_info: "Ver informacion del sistema",
    lock_workstation: "Bloquear Windows",
    reboot: "Reiniciar el equipo",
    shutdown: "Apagar el equipo",
    unattended_access: "Acceso desatendido",
  };
  return labels[name] ?? name;
}


/** Texto corto para el estado de verificacion de una funcion. */
export function featureStateLabel(state: FeatureState): string {
  switch (state) {
    case "working":
      return "Probado";
    case "unverified":
      return "Sin probar";
    case "pending":
      return "Pendiente";
  }
}

/** Formatea una resolucion con su escala, si no es del 100 %. */
export function formatResolution(monitor: MonitorInfo): string {
  const base = `${monitor.width} × ${monitor.height}`;
  return monitor.scale_percent === 100 ? base : `${base} · ${monitor.scale_percent} %`;
}
