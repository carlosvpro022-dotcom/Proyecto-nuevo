//! Pruebas de integracion de NEXUS REMOTE.
//!
//! Este crate no contiene codigo de produccion: existe para alojar las pruebas
//! que cruzan varios crates a la vez, que no caben dentro de ninguno de ellos.
//!
//! Las pruebas viven en `tests/`. Lo que se comprueba aqui son recorridos
//! completos: emparejar un dispositivo y autenticarlo despues, que los
//! permisos declarados en el protocolo se respeten, que el almacen persista lo
//! que debe, y que la interfaz de Tauri y el panel hablen de los mismos
//! comandos.

/// Version del conjunto de pruebas, util en los informes.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
