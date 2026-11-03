//! Punto de entrada del host de NEXUS REMOTE.
//!
//! En Windows se compila sin consola para que al abrir la aplicacion no aparezca
//! una ventana negra detras del panel. En compilaciones de depuracion si se deja
//! la consola, que es donde se leen los registros durante el desarrollo.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    // Los registros van a la consola en desarrollo. El nivel se controla con la
    // variable NEXUS_LOG (por ejemplo `NEXUS_LOG=debug`).
    let filter = std::env::var("NEXUS_LOG").unwrap_or_else(|_| "info".to_owned());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    nexus_windows_host::run();
}
