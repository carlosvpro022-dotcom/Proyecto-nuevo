//! Informacion real del equipo host.
//!
//! Se lee del sistema con `sysinfo`, que funciona tanto en Windows como en
//! Linux. Los valores son reales: no hay datos de ejemplo. El panel del host
//! muestra exactamente lo que devuelve este modulo.
//!
//! El unico campo que todavia no se puede averiguar es si la sesion de Windows
//! esta bloqueada, que requiere la API de sesiones de Windows y corresponde a la
//! etapa 5. Se devuelve `false` y el campo esta documentado como pendiente.

use nexus_core::protocol::control::SystemStatus;
use sysinfo::System;

/// Lee el estado actual del equipo.
///
/// `session_locked` siempre es `false`: detectar el bloqueo de la sesion de
/// Windows es trabajo de la etapa 5 y no se finge aqui.
#[must_use]
pub fn read_status() -> SystemStatus {
    let mut system = System::new();
    system.refresh_memory();

    SystemStatus {
        hostname: System::host_name().unwrap_or_else(|| "desconocido".to_owned()),
        os_description: os_description(),
        cpu_cores: u16::try_from(num_cpus()).unwrap_or(u16::MAX),
        memory_total_mib: system.total_memory() / (1024 * 1024),
        memory_available_mib: system.available_memory() / (1024 * 1024),
        uptime_secs: System::uptime(),
        session_locked: false,
    }
}

fn os_description() -> String {
    let name = System::name().unwrap_or_else(|| std::env::consts::OS.to_owned());
    match System::os_version() {
        Some(version) => format!("{name} {version}"),
        None => name,
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_reports_real_values() {
        let status = read_status();

        assert!(
            !status.hostname.is_empty(),
            "el nombre del equipo no deberia venir vacio"
        );
        assert!(!status.os_description.is_empty());
        assert!(
            status.cpu_cores >= 1,
            "al menos un nucleo: {}",
            status.cpu_cores
        );
        assert!(
            status.memory_total_mib > 0,
            "memoria total: {}",
            status.memory_total_mib
        );
        assert!(
            status.memory_available_mib <= status.memory_total_mib,
            "disponible {} no puede superar el total {}",
            status.memory_available_mib,
            status.memory_total_mib
        );
        assert!(
            status.uptime_secs > 0,
            "tiempo encendido: {}",
            status.uptime_secs
        );
    }

    #[test]
    fn repeated_reads_agree_on_the_static_parts() {
        let first = read_status();
        let second = read_status();
        assert_eq!(first.hostname, second.hostname);
        assert_eq!(first.cpu_cores, second.cpu_cores);
        assert_eq!(first.memory_total_mib, second.memory_total_mib);
    }

    #[test]
    fn the_status_serializes_for_the_protocol() {
        let status = read_status();
        let raw = serde_json::to_string(&status).expect("serializa");
        let back: SystemStatus = serde_json::from_str(&raw).expect("deserializa");
        assert_eq!(back, status);
    }

    #[test]
    fn session_lock_detection_is_still_pending() {
        // Esta prueba documenta una limitacion conocida: cuando se implemente
        // la deteccion (etapa 5) habra que cambiarla.
        assert!(
            !read_status().session_locked,
            "mientras no se implemente, el campo debe ser false y no un valor inventado"
        );
    }
}
