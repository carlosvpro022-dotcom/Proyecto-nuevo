//! # Host de NEXUS REMOTE para Windows
//!
//! Comparte la pantalla de este PC y acepta control desde los dispositivos
//! Apple previamente autorizados.
//!
//! ## Que funciona hoy
//!
//! - Identidad criptografica propia, persistente entre arranques
//!   ([`identity`]).
//! - Emparejamiento por codigo temporal y QR, con limite de intentos y
//!   caducidad ([`host`]).
//! - Lista blanca de dispositivos en `SQLite`, con permisos por dispositivo,
//!   revocacion e historial de sesiones ([`store`]).
//! - Autenticacion mutua de un dispositivo ya emparejado contra esa lista.
//! - Informacion real del equipo ([`system`]).
//! - Panel de escritorio sobre Tauri que muestra todo lo anterior
//!   ([`commands`]).
//!
//! ## Que falta y en que etapa
//!
//! El comando [`commands::feature_matrix`] devuelve la lista completa y la
//! interfaz la usa para no mostrar botones que no hacen nada. En resumen:
//! captura de pantalla y control de entrada en la etapa 2, conexion con el
//! servidor propio y WebRTC en la etapa 3, audio, archivos, portapapeles y
//! administracion del equipo en la etapa 5.

#![forbid(unsafe_code)]

pub mod admission;
pub mod commands;
pub mod confirmation;
pub mod error;
pub mod files;
pub mod host;
pub mod identity;
pub mod runtime;
pub mod session;
pub mod signaling;
pub mod store;
pub mod supervisor;
pub mod system;

pub use error::{HostError, HostResult, UiError};
pub use host::HostService;
pub use store::Store;

/// Nombre del directorio de datos dentro de la carpeta de la aplicacion.
pub const DATA_DIR_NAME: &str = "NexusRemote";

/// Arranca la aplicacion de escritorio.
///
/// # Panics
///
/// Si el directorio de datos no se puede preparar o la base de datos local no
/// se puede abrir. Son fallos de los que el host no puede recuperarse: sin
/// identidad ni lista de dispositivos no hay nada que ofrecer, y arrancar una
/// ventana que no puede hacer nada seria peor que no arrancar.
pub fn run() {
    let data_dir = data_directory();
    tracing::info!(path = %data_dir.display(), "directorio de datos del host");

    let service = HostService::open(&data_dir).unwrap_or_else(|error| {
        panic!(
            "no se pudo inicializar el host en {}: {error}.\n\
             Compruebe que la carpeta existe y es escribible.",
            data_dir.display()
        )
    });

    tracing::info!(
        device_id = %service.device_id(),
        fingerprint = %service.public_key().short_fingerprint(),
        "host listo"
    );

    let state = commands::AppState::new(service);

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::host_summary,
            commands::system_status,
            commands::list_devices,
            commands::open_pairing,
            commands::current_pairing,
            commands::cancel_pairing,
            commands::revoke_device,
            commands::delete_device,
            commands::set_device_profile,
            commands::set_signaling_url,
            commands::list_monitors,
            commands::engine_status,
            commands::check_signaling,
            commands::start_host,
            commands::stop_host,
            commands::runtime_status,
            commands::session_history,
            commands::set_network_exposure,
            commands::network_exposure,
            commands::set_shared_folders,
            commands::shared_folders,
            commands::pending_confirmation,
            commands::approve_confirmation,
            commands::refuse_confirmation,
            commands::feature_matrix,
            commands::permission_catalog,
        ])
        .run(tauri::generate_context!())
        .expect("fallo al arrancar la ventana de la aplicacion");
}

/// Directorio donde el host guarda su identidad y su base de datos.
///
/// En Windows es `%APPDATA%\NexusRemote`. En Linux, usado solo para
/// desarrollo, `$XDG_DATA_HOME/NexusRemote` o `~/.local/share/NexusRemote`.
#[must_use]
pub fn data_directory() -> std::path::PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(std::path::PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".local/share"))
            })
    };

    base.unwrap_or_else(std::env::temp_dir).join(DATA_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_directory_ends_with_the_app_folder() {
        let dir = data_directory();
        assert!(
            dir.ends_with(DATA_DIR_NAME),
            "ruta inesperada: {}",
            dir.display()
        );
        assert!(dir.is_absolute(), "deberia ser absoluta: {}", dir.display());
    }
}
