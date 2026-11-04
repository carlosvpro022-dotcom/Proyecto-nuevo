/** Configuracion: servidor propio y datos del host. */

import { useState } from "react";

import type {
  HostSummary,
  NetworkExposure,
  SharedFolder,
  SignalingProbe,
} from "../api";
import { Card, Notice } from "../components/ui";

export function Settings({
  summary,
  onSaveSignalingUrl,
  onCheckSignaling,
  onSaveNetwork,
  onSaveSharedFolders,
  probe,
  exposure,
  folders,
  checking,
  busy,
  error,
}: {
  summary: HostSummary;
  onSaveSignalingUrl: (url: string) => void;
  onCheckSignaling: (enrollmentToken: string) => void;
  onSaveNetwork: (udpPort: number, publicEndpoint: string | null) => void;
  onSaveSharedFolders: (folders: SharedFolder[]) => void;
  probe: SignalingProbe | null;
  exposure: NetworkExposure | null;
  folders: SharedFolder[];
  checking: boolean;
  busy: boolean;
  error: string | null;
}) {
  const [url, setUrl] = useState(summary.signalingUrl ?? "");
  const [token, setToken] = useState("");
  const [port, setPort] = useState(String(exposure?.udp_port ?? 0));
  const [endpoint, setEndpoint] = useState(exposure?.public_endpoint ?? "");
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
        title="Si el servidor esta en este mismo PC"
        hint="Solo hace falta si aloja el servidor aqui y abre un puerto en su router. Dejelo en blanco si usa un servidor aparte."
      >
        <div className="grid grid--two">
          <div className="field">
            <label className="field__label" htmlFor="udp-port">
              Puerto UDP de la sesion
            </label>
            <input
              id="udp-port"
              className="input"
              type="number"
              min={0}
              max={65535}
              value={port}
              onChange={(event) => setPort(event.target.value)}
            />
            <span className="field__hint">
              El mismo que abrio en el router. Un puerto al azar (0) no se puede
              reenviar porque no se sabe cual sera.
            </span>
          </div>

          <div className="field">
            <label className="field__label" htmlFor="public-endpoint">
              Direccion publica de su casa
            </label>
            <input
              id="public-endpoint"
              className="input"
              type="text"
              value={endpoint}
              placeholder="micasa.duckdns.org"
              spellCheck={false}
              onChange={(event) => setEndpoint(event.target.value)}
            />
            <span className="field__hint">
              Un nombre de DNS dinamico, mejor que una IP: la que da su
              operador cambia y el nombre no. Se resuelve en cada sesion.
            </span>
          </div>
        </div>

        <div className="row" style={{ marginTop: "var(--space-4)" }}>
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={() =>
              onSaveNetwork(
                Number.parseInt(port, 10) || 0,
                endpoint.trim().length > 0 ? endpoint.trim() : null,
              )
            }
          >
            Guardar
          </button>
          <span className="field__hint">
            La guia completa esta en docs/deployment/en-el-mismo-pc.md
          </span>
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

      <SharedFoldersCard
        folders={folders}
        busy={busy}
        onSave={onSaveSharedFolders}
      />

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

/**
 * Las carpetas que este equipo comparte.
 *
 * Es la pantalla que decide a que puede llegar un dispositivo remoto. Por eso
 * dice lo que de verdad hace falta saber: que sin ninguna carpeta no se
 * comparte nada, y que marcar la escritura permite dejar y sobrescribir
 * archivos dentro.
 */
function SharedFoldersCard({
  folders,
  busy,
  onSave,
}: {
  folders: SharedFolder[];
  busy: boolean;
  onSave: (folders: SharedFolder[]) => void;
}) {
  const [draft, setDraft] = useState<SharedFolder[]>(folders);
  const [label, setLabel] = useState("");
  const [path, setPath] = useState("");
  const [writable, setWritable] = useState(false);

  const canAdd = label.trim().length > 0 && path.trim().length > 0;
  const dirty = JSON.stringify(draft) !== JSON.stringify(folders);

  function add() {
    if (!canAdd) return;
    setDraft([
      ...draft,
      { label: label.trim(), path: path.trim(), writable },
    ]);
    setLabel("");
    setPath("");
    setWritable(false);
  }

  return (
    <Card
      title="Carpetas compartidas"
      hint="A lo unico que un dispositivo remoto puede llegar. Fuera de esta lista, nada."
    >
      {draft.length === 0 ? (
        <Notice>
          Ahora mismo no se comparte ninguna carpeta, asi que ningun dispositivo
          puede ver archivos de este PC, ni aunque tenga el permiso concedido.
          Son dos cosas separadas a proposito: el permiso lo tiene el
          dispositivo, la carpeta la elige usted aqui.
        </Notice>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>Nombre</th>
              <th>Carpeta en este PC</th>
              <th>Pueden escribir</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {draft.map((folder, index) => (
              <tr key={`${folder.path}-${index}`}>
                <td>{folder.label}</td>
                <td className="mono" style={{ wordBreak: "break-all" }}>
                  {folder.path}
                </td>
                <td className="mono">{folder.writable ? "si" : "no"}</td>
                <td>
                  <button
                    type="button"
                    className="btn btn--ghost"
                    onClick={() =>
                      setDraft(draft.filter((_, i) => i !== index))
                    }
                  >
                    Quitar
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <div className="grid grid--two" style={{ marginTop: "var(--space-4)" }}>
        <div className="field">
          <label className="field__label" htmlFor="shared-label">
            Nombre que vera quien se conecte
          </label>
          <input
            id="shared-label"
            className="input"
            type="text"
            value={label}
            placeholder="Intercambio"
            onChange={(event) => setLabel(event.target.value)}
          />
        </div>

        <div className="field">
          <label className="field__label" htmlFor="shared-path">
            Carpeta en este PC
          </label>
          <input
            id="shared-path"
            className="input"
            type="text"
            value={path}
            placeholder="C:\\Users\\usuario\\Intercambio"
            spellCheck={false}
            onChange={(event) => setPath(event.target.value)}
          />
          <span className="field__hint">
            Ruta completa. Tiene que existir, y no puede ser la raiz de un
            disco: compartir <span className="mono">C:\</span> entero anula el
            sentido de tener una lista.
          </span>
        </div>
      </div>

      <label className="row" style={{ marginTop: "var(--space-3)", gap: "var(--space-2)" }}>
        <input
          type="checkbox"
          checked={writable}
          onChange={(event) => setWritable(event.target.checked)}
        />
        <span className="field__hint">
          Permitir escribir dentro. Sin marcar, solo se puede mirar y
          descargar; marcado, tambien dejar archivos.
        </span>
      </label>

      <div className="row" style={{ marginTop: "var(--space-4)" }}>
        <button
          type="button"
          className="btn"
          disabled={!canAdd || busy}
          onClick={add}
        >
          Anadir a la lista
        </button>
        <button
          type="button"
          className="btn btn--primary"
          disabled={!dirty || busy}
          onClick={() => onSave(draft)}
        >
          Guardar
        </button>
        {dirty && (
          <button
            type="button"
            className="btn btn--ghost"
            onClick={() => setDraft(folders)}
          >
            Descartar cambios
          </button>
        )}
      </div>

      <div style={{ marginTop: "var(--space-4)" }}>
        <Notice tone="warn">
          Un archivo que se suba no sobrescribe uno que ya exista con el mismo
          nombre: se rechaza y se dice por que. Sobrescribir pedira una
          confirmacion en este PC, y eso todavia no esta hecho.
        </Notice>
      </div>
    </Card>
  );
}
