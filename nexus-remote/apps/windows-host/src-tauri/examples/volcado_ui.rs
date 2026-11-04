//! Vuelca a JSON lo que devolverian los comandos del host.
//!
//! Sirve para comprobar que el panel se dibuja correctamente con datos de
//! verdad, sin necesidad de abrir la ventana de Tauri. Se usa en el guion
//! `scripts/verificar-panel.mjs`, que carga el panel compilado en un navegador
//! e inyecta este volcado.
//!
//! Llama a las mismas funciones que los comandos (`build_host_summary`,
//! `to_summary`, `feature_matrix`, `permission_catalog`), asi que lo que se
//! verifica es el panel real con datos reales, no una maqueta.
//!
//! Uso:
//!
//! ```text
//! cargo run -p nexus-windows-host --example volcado_ui > panel.json
//! ```

use nexus_core::{DeviceId, DeviceInfo, DevicePlatform, DeviceRole, PermissionSet, Timestamp};
use nexus_security::DeviceIdentity;
use nexus_windows_host::HostService;
use nexus_windows_host::commands::{
    build_host_summary, feature_matrix, permission_catalog, to_summary,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Host efimero: base de datos en memoria, identidad recien generada.
    let mut host = HostService::open_ephemeral(DeviceIdentity::generate(DeviceId::new()))?;
    host.set_signaling_url("wss://nexus.midominio.com/v1/signaling")?;

    let now = Timestamp::now();

    // Se emparejan dos dispositivos reales, recorriendo el flujo completo.
    for (name, platform, permissions, unattended) in [
        (
            "iPad Pro de Carlos",
            DevicePlatform::IpadOs,
            PermissionSet::full(),
            true,
        ),
        (
            "iPhone de Carlos",
            DevicePlatform::Ios,
            PermissionSet::standard(),
            false,
        ),
    ] {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let (_, code) = host.open_pairing(permissions, unattended, now)?;
        host.redeem_pairing(
            code.as_str(),
            DeviceInfo {
                id: identity.device_id(),
                display_name: name.to_owned(),
                platform,
                role: DeviceRole::Controller,
                os_version: "18.1".to_owned(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            &identity.public_key().to_hex(),
            now,
        )?;
    }

    // Y queda un emparejamiento abierto, para ver tambien esa pantalla.
    let (session, code) = host.open_pairing(PermissionSet::standard(), false, now)?;
    let invite = host.current_invite(now)?;

    // Dos carpetas compartidas de verdad, para que la pantalla de archivos se
    // dibuje con datos y no con el caso vacio. Se crean en la carpeta temporal
    // del sistema porque `set_shared_roots` comprueba que existan: el volcado
    // no puede inventarse rutas que el host rechazaria.
    let base = std::env::temp_dir().join("nexus-volcado-ui");
    let lectura = base.join("Documentos");
    let escritura = base.join("Intercambio");
    for dir in [&lectura, &escritura] {
        std::fs::create_dir_all(dir)?;
    }
    host.set_shared_roots(&[
        nexus_file_transfer::SharedRoot::read_only("Documentos", &lectura),
        nexus_file_transfer::SharedRoot::writable("Intercambio", &escritura),
    ])?;

    let devices: Vec<_> = host.list_devices()?.into_iter().map(to_summary).collect();
    let summary = build_host_summary(&mut host)?;

    let payload = serde_json::json!({
        "host_summary": summary,
        "system_status": nexus_windows_host::system::read_status(),
        "list_devices": devices,
        "feature_matrix": feature_matrix(),
        "engine_status": host.engine_status(),
        // En Linux la enumeracion falla a proposito; el volcado refleja ese
        // mismo caso, que es el que vera quien desarrolle aqui.
        "list_monitors": host.list_monitors().map(|layout| layout.0).unwrap_or_default(),
        "permission_catalog": permission_catalog(),
        // El bucle no esta en marcha en el volcado: lo que se comprueba es que
        // el panel pinta bien ese caso, que es el primero que vera cualquiera.
        "runtime_status": nexus_windows_host::supervisor::RuntimeStatus::stopped(),
        "session_history": host.store().list_sessions(100)?,
        "network_exposure": {
            "udp_port": host.network_exposure()?.0,
            "public_endpoint": host.network_exposure()?.1,
        },
        "shared_folders": host
            .shared_roots()?
            .into_iter()
            .map(|root| {
                serde_json::json!({
                    "label": root.label,
                    "path": root.path.to_string_lossy(),
                    "writable": root.writable,
                })
            })
            .collect::<Vec<_>>(),
        "current_pairing": {
            "pairing_id": session.id.to_string(),
            "code": code.grouped(),
            "qr_payload": invite.to_json()?,
            "expires_at_ms": session.expires_at.0,
            "ttl_secs": nexus_windows_host::host::pairing_ttl().as_secs(),
            "remaining_attempts": session.remaining_attempts(),
        },
    });

    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}
