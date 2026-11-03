//! Almacen local del host, sobre `SQLite`.
//!
//! Guarda lo que el host necesita recordar entre arranques: los dispositivos
//! autorizados con su clave publica, el historial de sesiones y los ajustes.
//!
//! ## Que no se guarda aqui
//!
//! - **La clave privada del host.** Vive en un archivo aparte con permisos
//!   restringidos (ver [`crate::identity`]), no en la base de datos.
//! - **Los codigos de emparejamiento en claro.** Solo se guarda su derivacion
//!   Argon2id, y unicamente mientras el emparejamiento esta abierto.
//!
//! El esquema se versiona con `PRAGMA user_version`, de modo que una version
//! futura pueda migrar sin perder los dispositivos ya emparejados.

use nexus_core::{DeviceId, DeviceInfo, DevicePlatform, DeviceRole, PermissionSet, Timestamp};
use nexus_security::identity::DevicePublicKey;
use nexus_security::registry::{DeviceRecord, DeviceRegistry};
use nexus_security::{SecurityError, SecurityResult};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{HostError, HostResult};

/// Version del esquema que entiende este binario.
pub const SCHEMA_VERSION: i64 = 1;

/// Almacen local del host.
#[derive(Debug)]
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Abre (o crea) la base de datos en la ruta indicada y aplica migraciones.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si no se puede abrir o migrar.
    pub fn open(path: &std::path::Path) -> HostResult<Self> {
        let connection = Connection::open(path)?;
        let store = Self { connection };
        store.configure()?;
        store.migrate()?;
        Ok(store)
    }

    /// Abre una base de datos en memoria. Para pruebas.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si no se puede crear.
    pub fn open_in_memory() -> HostResult<Self> {
        let connection = Connection::open_in_memory()?;
        let store = Self { connection };
        store.configure()?;
        store.migrate()?;
        Ok(store)
    }

    fn configure(&self) -> HostResult<()> {
        // `foreign_keys` no esta activo por omision en SQLite y hace falta para
        // que las restricciones del esquema se respeten de verdad.
        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;
        Ok(())
    }

    fn migrate(&self) -> HostResult<()> {
        let current: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if current >= SCHEMA_VERSION {
            return Ok(());
        }

        if current < 1 {
            self.connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS devices (
                     id                 TEXT PRIMARY KEY NOT NULL,
                     display_name       TEXT NOT NULL,
                     platform           TEXT NOT NULL,
                     role               TEXT NOT NULL,
                     os_version         TEXT NOT NULL,
                     app_version        TEXT NOT NULL,
                     public_key_hex     TEXT NOT NULL UNIQUE,
                     permissions_json   TEXT NOT NULL,
                     paired_at_ms       INTEGER NOT NULL,
                     last_seen_ms       INTEGER,
                     revoked_at_ms      INTEGER,
                     unattended_access  INTEGER NOT NULL DEFAULT 0
                 );

                 CREATE TABLE IF NOT EXISTS sessions (
                     id            TEXT PRIMARY KEY NOT NULL,
                     device_id     TEXT NOT NULL,
                     started_at_ms INTEGER NOT NULL,
                     ended_at_ms   INTEGER,
                     close_reason  TEXT,
                     remote_hint   TEXT,
                     FOREIGN KEY (device_id) REFERENCES devices (id) ON DELETE CASCADE
                 );
                 CREATE INDEX IF NOT EXISTS sessions_by_device
                     ON sessions (device_id, started_at_ms DESC);

                 CREATE TABLE IF NOT EXISTS settings (
                     key   TEXT PRIMARY KEY NOT NULL,
                     value TEXT NOT NULL
                 );",
            )?;
        }

        self.connection
            .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        Ok(())
    }

    /// Version de esquema almacenada.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la consulta falla.
    pub fn schema_version(&self) -> HostResult<i64> {
        Ok(self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    /// Guarda o actualiza un dispositivo autorizado.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la escritura falla, por ejemplo si otra
    /// entrada ya usa la misma clave publica.
    pub fn save_device(&self, id: DeviceId, record: &DeviceRecord) -> HostResult<()> {
        let permissions = serde_json::to_string(&record.permissions)
            .map_err(|e| HostError::Database(e.to_string()))?;

        self.connection.execute(
            "INSERT INTO devices (
                 id, display_name, platform, role, os_version, app_version,
                 public_key_hex, permissions_json, paired_at_ms, last_seen_ms,
                 revoked_at_ms, unattended_access
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (id) DO UPDATE SET
                 display_name      = excluded.display_name,
                 platform          = excluded.platform,
                 role              = excluded.role,
                 os_version        = excluded.os_version,
                 app_version       = excluded.app_version,
                 public_key_hex    = excluded.public_key_hex,
                 permissions_json  = excluded.permissions_json,
                 last_seen_ms      = excluded.last_seen_ms,
                 revoked_at_ms     = excluded.revoked_at_ms,
                 unattended_access = excluded.unattended_access",
            params![
                id.to_string(),
                record.info.sanitized_name(),
                platform_to_text(record.info.platform),
                role_to_text(record.info.role),
                record.info.os_version,
                record.info.app_version,
                record.public_key.to_hex(),
                permissions,
                to_sqlite_ms(record.paired_at),
                record.last_seen.map(to_sqlite_ms),
                record.revoked_at.map(to_sqlite_ms),
                i64::from(record.unattended_access),
            ],
        )?;
        Ok(())
    }

    /// Lee un dispositivo.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la lectura falla o la fila esta corrupta.
    pub fn load_device(&self, id: DeviceId) -> HostResult<Option<DeviceRecord>> {
        let record = self
            .connection
            .query_row(
                "SELECT id, display_name, platform, role, os_version, app_version,
                        public_key_hex, permissions_json, paired_at_ms, last_seen_ms,
                        revoked_at_ms, unattended_access
                 FROM devices WHERE id = ?1",
                params![id.to_string()],
                row_to_record,
            )
            .optional()?;

        record.transpose()
    }

    /// Lista todos los dispositivos, los revocados incluidos.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la lectura falla.
    pub fn list_devices(&self) -> HostResult<Vec<(DeviceId, DeviceRecord)>> {
        let mut statement = self.connection.prepare(
            "SELECT id, display_name, platform, role, os_version, app_version,
                    public_key_hex, permissions_json, paired_at_ms, last_seen_ms,
                    revoked_at_ms, unattended_access
             FROM devices ORDER BY paired_at_ms DESC, id ASC",
        )?;

        let rows = statement.query_map([], row_to_record)?;
        let mut out = Vec::new();
        for row in rows {
            let record = row??;
            out.push((record.info.id, record));
        }
        Ok(out)
    }

    /// Marca un dispositivo como revocado. Devuelve `true` si cambio algo.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la escritura falla.
    pub fn revoke_device(&self, id: DeviceId, now: Timestamp) -> HostResult<bool> {
        let changed = self.connection.execute(
            "UPDATE devices SET revoked_at_ms = ?2 WHERE id = ?1 AND revoked_at_ms IS NULL",
            params![id.to_string(), to_sqlite_ms(now)],
        )?;
        Ok(changed > 0)
    }

    /// Anota la ultima conexion correcta de un dispositivo.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la escritura falla.
    pub fn touch_device(&self, id: DeviceId, now: Timestamp) -> HostResult<()> {
        self.connection.execute(
            "UPDATE devices SET last_seen_ms = ?2 WHERE id = ?1",
            params![id.to_string(), to_sqlite_ms(now)],
        )?;
        Ok(())
    }

    /// Borra definitivamente un dispositivo y su historial.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la escritura falla.
    pub fn delete_device(&self, id: DeviceId) -> HostResult<bool> {
        let removed = self
            .connection
            .execute("DELETE FROM devices WHERE id = ?1", params![id.to_string()])?;
        Ok(removed > 0)
    }

    /// Guarda un ajuste.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la escritura falla.
    pub fn set_setting(&self, key: &str, value: &str) -> HostResult<()> {
        self.connection.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Lee un ajuste.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la lectura falla.
    pub fn get_setting(&self, key: &str) -> HostResult<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Anota el inicio de una sesion en el historial.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la escritura falla, por ejemplo si el
    /// dispositivo no existe.
    pub fn log_session_start(
        &self,
        session_id: nexus_core::SessionId,
        device_id: DeviceId,
        now: Timestamp,
        remote_hint: Option<&str>,
    ) -> HostResult<()> {
        self.connection.execute(
            "INSERT INTO sessions (id, device_id, started_at_ms, remote_hint)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session_id.to_string(),
                device_id.to_string(),
                to_sqlite_ms(now),
                remote_hint
            ],
        )?;
        Ok(())
    }

    /// Cierra una sesion del historial.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la escritura falla.
    pub fn log_session_end(
        &self,
        session_id: nexus_core::SessionId,
        now: Timestamp,
        reason: nexus_core::CloseReason,
    ) -> HostResult<()> {
        self.connection.execute(
            "UPDATE sessions SET ended_at_ms = ?2, close_reason = ?3 WHERE id = ?1",
            params![session_id.to_string(), to_sqlite_ms(now), reason.name()],
        )?;
        Ok(())
    }

    /// Numero de sesiones registradas para un dispositivo.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la lectura falla.
    pub fn session_count(&self, device_id: DeviceId) -> HostResult<i64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM sessions WHERE device_id = ?1",
            params![device_id.to_string()],
            |row| row.get(0),
        )?)
    }
}

/// El almacen sirve directamente como lista de autorizados del autenticador.
///
/// Asi el flujo de autenticacion probado en `nexus-security` funciona contra
/// `SQLite` sin ninguna capa intermedia que pudiera divergir.
impl DeviceRegistry for Store {
    fn get(&self, device_id: DeviceId) -> Option<DeviceRecord> {
        // El rasgo no admite error: un fallo de lectura se registra y se trata
        // como "no esta", que es el resultado seguro (deniega el acceso).
        match self.load_device(device_id) {
            Ok(record) => record,
            Err(error) => {
                tracing::error!(%error, %device_id, "fallo al leer el dispositivo; se deniega");
                None
            }
        }
    }

    fn upsert(&mut self, device_id: DeviceId, record: DeviceRecord) -> SecurityResult<()> {
        self.save_device(device_id, &record)
            .map_err(|e| SecurityError::HashFailure(e.to_string()))
    }

    fn revoke(&mut self, device_id: DeviceId, now: Timestamp) -> SecurityResult<bool> {
        Store::revoke_device(self, device_id, now)
            .map_err(|e| SecurityError::HashFailure(e.to_string()))
    }

    fn touch(&mut self, device_id: DeviceId, now: Timestamp) -> SecurityResult<()> {
        self.touch_device(device_id, now)
            .map_err(|e| SecurityError::HashFailure(e.to_string()))
    }

    fn list(&self) -> Vec<(DeviceId, DeviceRecord)> {
        self.list_devices().unwrap_or_else(|error| {
            tracing::error!(%error, "fallo al listar dispositivos");
            Vec::new()
        })
    }
}

/// Convierte una marca temporal a lo que `SQLite` puede guardar.
///
/// `SQLite` almacena enteros como `i64`, y las marcas del protocolo son `u64` en
/// milisegundos. La conversion es segura en la practica (un `i64` de
/// milisegundos cubre hasta el ano 292 millones) pero se hace en un solo sitio y
/// de forma explicita, en lugar de con un `as` repartido por las consultas.
fn to_sqlite_ms(timestamp: Timestamp) -> i64 {
    // `i64::MAX` como tope: preferible saturar que envolver a negativo, que
    // ordenaria el historial al reves.
    i64::try_from(timestamp.0).unwrap_or(i64::MAX)
}

/// Convierte lo leido de `SQLite` a una marca temporal.
///
/// Un valor negativo solo puede venir de una fila corrupta o escrita a mano; se
/// interpreta como la epoca en lugar de envolver a un numero enorme.
fn from_sqlite_ms(value: i64) -> Timestamp {
    u64::try_from(value).map_or(Timestamp::EPOCH, Timestamp)
}

type RowResult = rusqlite::Result<HostResult<DeviceRecord>>;

fn row_to_record(row: &rusqlite::Row<'_>) -> RowResult {
    let id_text: String = row.get(0)?;
    let public_key_hex: String = row.get(6)?;
    let permissions_json: String = row.get(7)?;
    let paired_at_ms: i64 = row.get(8)?;
    let last_seen_ms: Option<i64> = row.get(9)?;
    let revoked_at_ms: Option<i64> = row.get(10)?;
    let unattended: i64 = row.get(11)?;

    let Ok(id) = id_text.parse::<DeviceId>() else {
        return Ok(Err(HostError::Database(format!(
            "identificador corrupto: {id_text}"
        ))));
    };
    let public_key = match DevicePublicKey::from_hex(&public_key_hex) {
        Ok(key) => key,
        Err(error) => {
            return Ok(Err(HostError::Database(format!(
                "clave publica corrupta: {error}"
            ))));
        }
    };
    let permissions: PermissionSet = match serde_json::from_str(&permissions_json) {
        Ok(set) => set,
        Err(error) => {
            return Ok(Err(HostError::Database(format!(
                "permisos corruptos: {error}"
            ))));
        }
    };

    let platform_text: String = row.get(2)?;
    let role_text: String = row.get(3)?;

    Ok(Ok(DeviceRecord {
        info: DeviceInfo {
            id,
            display_name: row.get(1)?,
            platform: platform_from_text(&platform_text),
            role: role_from_text(&role_text),
            os_version: row.get(4)?,
            app_version: row.get(5)?,
        },
        public_key,
        permissions,
        paired_at: from_sqlite_ms(paired_at_ms),
        last_seen: last_seen_ms.map(from_sqlite_ms),
        revoked_at: revoked_at_ms.map(from_sqlite_ms),
        unattended_access: unattended != 0,
    }))
}

const fn platform_to_text(platform: DevicePlatform) -> &'static str {
    match platform {
        DevicePlatform::Windows => "windows",
        DevicePlatform::Ios => "ios",
        DevicePlatform::IpadOs => "ipados",
        DevicePlatform::Linux => "linux",
        DevicePlatform::Unknown => "unknown",
    }
}

fn platform_from_text(text: &str) -> DevicePlatform {
    match text {
        "windows" => DevicePlatform::Windows,
        "ios" => DevicePlatform::Ios,
        "ipados" => DevicePlatform::IpadOs,
        "linux" => DevicePlatform::Linux,
        _ => DevicePlatform::Unknown,
    }
}

const fn role_to_text(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Host => "host",
        DeviceRole::Controller => "controller",
    }
}

fn role_from_text(text: &str) -> DeviceRole {
    if text == "host" {
        DeviceRole::Host
    } else {
        DeviceRole::Controller
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_security::DeviceIdentity;

    const NOW: Timestamp = Timestamp(1_730_000_000_000);

    fn record(identity: &DeviceIdentity, name: &str) -> DeviceRecord {
        DeviceRecord::new(
            DeviceInfo {
                id: identity.device_id(),
                display_name: name.to_owned(),
                platform: DevicePlatform::IpadOs,
                role: DeviceRole::Controller,
                os_version: "18.1".to_owned(),
                app_version: "0.1.0".to_owned(),
            },
            identity.public_key(),
            PermissionSet::standard(),
            false,
            NOW,
        )
    }

    fn store() -> Store {
        Store::open_in_memory().expect("base en memoria")
    }

    #[test]
    fn a_new_store_is_migrated_to_the_current_schema() {
        let store = store();
        assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);
        assert!(store.list_devices().expect("lista").is_empty());
    }

    #[test]
    fn migration_is_idempotent() {
        let temp = tempfile::tempdir().expect("temporal");
        let path = temp.path().join("nexus.db");

        let first = Store::open(&path).expect("primera apertura");
        let identity = DeviceIdentity::generate(DeviceId::new());
        first
            .save_device(identity.device_id(), &record(&identity, "iPad"))
            .expect("guarda");
        drop(first);

        // Reabrir no debe perder datos ni volver a crear las tablas.
        let second = Store::open(&path).expect("segunda apertura");
        assert_eq!(second.schema_version().expect("version"), SCHEMA_VERSION);
        assert_eq!(second.list_devices().expect("lista").len(), 1);
    }

    #[test]
    fn a_device_survives_a_save_load_cycle() {
        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        let original = record(&identity, "iPad Pro");
        store
            .save_device(identity.device_id(), &original)
            .expect("guarda");

        let loaded = store
            .load_device(identity.device_id())
            .expect("lee")
            .expect("existe");
        assert_eq!(loaded.public_key, original.public_key);
        assert_eq!(loaded.permissions, original.permissions);
        assert_eq!(loaded.paired_at, NOW);
        assert!(loaded.is_active());
    }

    #[test]
    fn an_unknown_device_reads_as_absent() {
        assert!(store().load_device(DeviceId::new()).expect("lee").is_none());
    }

    #[test]
    fn saving_twice_updates_instead_of_duplicating() {
        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        store
            .save_device(identity.device_id(), &record(&identity, "iPad"))
            .expect("guarda");

        let mut updated = record(&identity, "iPad de Carlos");
        updated.permissions = PermissionSet::full();
        store
            .save_device(identity.device_id(), &updated)
            .expect("actualiza");

        assert_eq!(store.list_devices().expect("lista").len(), 1);
        let loaded = store
            .load_device(identity.device_id())
            .expect("lee")
            .expect("existe");
        assert_eq!(loaded.info.display_name, "iPad de Carlos");
        assert_eq!(loaded.permissions, PermissionSet::full());
    }

    #[test]
    fn two_devices_cannot_share_a_public_key() {
        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        store
            .save_device(identity.device_id(), &record(&identity, "iPad"))
            .expect("guarda");

        // Otro identificador, la misma clave publica: el esquema lo impide.
        let clone_id = DeviceId::new();
        let mut cloned = record(&identity, "Impostor");
        cloned.info.id = clone_id;
        assert!(
            store.save_device(clone_id, &cloned).is_err(),
            "una clave publica no debe poder registrarse dos veces"
        );
    }

    #[test]
    fn revoking_is_persisted_and_idempotent() {
        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        store
            .save_device(identity.device_id(), &record(&identity, "iPad"))
            .expect("guarda");

        assert!(
            store
                .revoke_device(identity.device_id(), Timestamp(NOW.0 + 1))
                .expect("revoca")
        );
        assert!(
            !store
                .revoke_device(identity.device_id(), Timestamp(NOW.0 + 2))
                .expect("ya estaba")
        );

        let loaded = store
            .load_device(identity.device_id())
            .expect("lee")
            .expect("existe");
        assert!(!loaded.is_active());
        assert_eq!(loaded.revoked_at, Some(Timestamp(NOW.0 + 1)));
    }

    #[test]
    fn touching_records_the_last_connection() {
        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        store
            .save_device(identity.device_id(), &record(&identity, "iPad"))
            .expect("guarda");

        store
            .touch_device(identity.device_id(), Timestamp(NOW.0 + 500))
            .expect("anota");
        let loaded = store
            .load_device(identity.device_id())
            .expect("lee")
            .expect("existe");
        assert_eq!(loaded.last_seen, Some(Timestamp(NOW.0 + 500)));
    }

    #[test]
    fn deleting_a_device_removes_its_sessions() {
        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        store
            .save_device(identity.device_id(), &record(&identity, "iPad"))
            .expect("guarda");

        let session = nexus_core::SessionId::new();
        store
            .log_session_start(session, identity.device_id(), NOW, Some("movil"))
            .expect("anota sesion");
        assert_eq!(
            store.session_count(identity.device_id()).expect("cuenta"),
            1
        );

        assert!(store.delete_device(identity.device_id()).expect("borra"));
        assert_eq!(
            store.session_count(identity.device_id()).expect("cuenta"),
            0,
            "las sesiones deben irse con el dispositivo"
        );
    }

    #[test]
    fn a_session_for_an_unknown_device_is_refused() {
        // La clave ajena del esquema impide un historial huerfano.
        let store = store();
        let result =
            store.log_session_start(nexus_core::SessionId::new(), DeviceId::new(), NOW, None);
        assert!(
            result.is_err(),
            "no debe admitirse una sesion sin dispositivo"
        );
    }

    #[test]
    fn a_session_can_be_closed_with_its_reason() {
        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        store
            .save_device(identity.device_id(), &record(&identity, "iPad"))
            .expect("guarda");

        let session = nexus_core::SessionId::new();
        store
            .log_session_start(session, identity.device_id(), NOW, None)
            .expect("abre");
        store
            .log_session_end(
                session,
                Timestamp(NOW.0 + 1_000),
                nexus_core::CloseReason::UserRequested,
            )
            .expect("cierra");
        assert_eq!(
            store.session_count(identity.device_id()).expect("cuenta"),
            1
        );
    }

    #[test]
    fn timestamp_conversion_saturates_instead_of_wrapping() {
        let now_i64 = i64::try_from(NOW.0).expect("la marca de prueba cabe en i64");
        assert_eq!(to_sqlite_ms(Timestamp(0)), 0);
        assert_eq!(to_sqlite_ms(NOW), now_i64);
        assert_eq!(to_sqlite_ms(Timestamp(u64::MAX)), i64::MAX);

        assert_eq!(from_sqlite_ms(0), Timestamp::EPOCH);
        assert_eq!(
            from_sqlite_ms(-1),
            Timestamp::EPOCH,
            "una fila corrupta no debe envolver"
        );
        assert_eq!(from_sqlite_ms(now_i64), NOW);
    }

    #[test]
    fn settings_round_trip() {
        let store = store();
        assert!(store.get_setting("quality").expect("lee").is_none());
        store
            .set_setting("quality", "equilibrado")
            .expect("escribe");
        assert_eq!(
            store.get_setting("quality").expect("lee").as_deref(),
            Some("equilibrado")
        );

        store.set_setting("quality", "nitido").expect("sobrescribe");
        assert_eq!(
            store.get_setting("quality").expect("lee").as_deref(),
            Some("nitido")
        );
    }

    #[test]
    fn device_names_are_sanitized_before_being_stored() {
        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        let mut dirty = record(&identity, "  iPad\u{0}   de\tCarlos  ");
        dirty.info.display_name = "  iPad\u{0}   de\tCarlos  ".to_owned();
        store
            .save_device(identity.device_id(), &dirty)
            .expect("guarda");

        let loaded = store
            .load_device(identity.device_id())
            .expect("lee")
            .expect("existe");
        assert_eq!(loaded.info.display_name, "iPad de Carlos");
    }

    #[test]
    fn the_store_works_as_the_authenticator_registry() {
        use nexus_security::registry::{Authenticator, DeviceRegistry as _};

        let store = store();
        let identity = DeviceIdentity::generate(DeviceId::new());
        store
            .save_device(identity.device_id(), &record(&identity, "iPad"))
            .expect("guarda");

        // El mismo flujo de autenticacion probado en nexus-security, pero
        // respaldado por SQLite en lugar de por memoria.
        let mut auth = Authenticator::new(store);
        let challenge = auth.issue_challenge(NOW).expect("reto");

        let client_nonce = nexus_security::challenge::Nonce::generate();
        let transcript = nexus_security::challenge::AuthTranscript::new(
            challenge.nonce,
            client_nonce,
            identity.device_id(),
            nexus_security::challenge::AuthRole::Controller,
            NOW,
        );
        let request = nexus_security::registry::AuthRequest {
            device_id: identity.device_id(),
            server_nonce: challenge.nonce,
            client_nonce,
            role: nexus_security::challenge::AuthRole::Controller,
            timestamp: NOW,
            signature_hex: identity.sign_hex(&transcript.to_bytes()),
        };

        let outcome = auth.verify(&request, NOW).expect("autentica contra SQLite");
        assert_eq!(outcome.permissions, PermissionSet::standard());

        // Y la revocacion a traves del rasgo tambien persiste.
        assert!(
            auth.registry_mut()
                .revoke(identity.device_id(), NOW)
                .expect("revoca")
        );
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let transcript = nexus_security::challenge::AuthTranscript::new(
            challenge.nonce,
            client_nonce,
            identity.device_id(),
            nexus_security::challenge::AuthRole::Controller,
            NOW,
        );
        let request = nexus_security::registry::AuthRequest {
            signature_hex: identity.sign_hex(&transcript.to_bytes()),
            server_nonce: challenge.nonce,
            ..request
        };
        assert_eq!(
            auth.verify(&request, NOW).unwrap_err(),
            nexus_security::SecurityError::DeviceRevoked
        );
    }

    #[test]
    fn listing_is_ordered_by_pairing_time_descending() {
        let store = store();
        for (offset, name) in [(0, "primero"), (100, "segundo"), (50, "tercero")] {
            let identity = DeviceIdentity::generate(DeviceId::new());
            let mut r = record(&identity, name);
            r.paired_at = Timestamp(NOW.0 + offset);
            store.save_device(identity.device_id(), &r).expect("guarda");
        }

        let names: Vec<_> = store
            .list_devices()
            .expect("lista")
            .into_iter()
            .map(|(_, r)| r.info.display_name)
            .collect();
        assert_eq!(names, vec!["segundo", "tercero", "primero"]);
    }
}
