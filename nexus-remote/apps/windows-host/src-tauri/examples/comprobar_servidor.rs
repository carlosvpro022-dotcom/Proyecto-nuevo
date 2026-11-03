//! Comprueba un servidor de senalizacion desde la linea de ordenes.
//!
//! Hace exactamente lo mismo que el boton "Probar conexion" del panel: se
//! conecta de verdad, se autentica con una identidad recien generada y se
//! declara disponible como host. Sirve para comprobar un despliegue desde la
//! propia maquina del servidor, sin tener que abrir el panel en el PC.
//!
//! ```bash
//! cargo run -p nexus-windows-host --example comprobar_servidor -- \
//!   wss://nexus.midominio.com/v1/signaling "$NEXUS_HOST_ENROLLMENT_TOKEN"
//! ```
//!
//! La identidad que usa es efimera y se descarta al terminar: no toca la del
//! host ni deja nada registrado que haya que limpiar despues, salvo la propia
//! alta en el directorio del servidor.

use nexus_core::device::{DeviceInfo, DevicePlatform, DeviceRole};
use nexus_core::{Capability, CapabilitySet, DeviceId};
use nexus_security::DeviceIdentity;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(url) = args.next() else {
        eprintln!("uso: comprobar_servidor <url> [testigo-de-alta]");
        eprintln!();
        eprintln!("  url      direccion del canal, por ejemplo");
        eprintln!("           wss://nexus.midominio.com/v1/signaling");
        eprintln!("  testigo  valor de NEXUS_HOST_ENROLLMENT_TOKEN; hace falta");
        eprintln!("           la primera vez que un host se da de alta");
        return std::process::ExitCode::FAILURE;
    };
    let token = args.next();

    let device_id = DeviceId::new();
    let identity = DeviceIdentity::generate(device_id);
    let info = DeviceInfo {
        id: device_id,
        display_name: "Comprobacion desde la linea de ordenes".to_owned(),
        platform: DevicePlatform::Windows,
        role: DeviceRole::Host,
        os_version: "n/d".to_owned(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let mut capabilities = CapabilitySet::empty();
    capabilities.insert(Capability::ScreenStream);

    println!("Comprobando {url}");
    println!(
        "  identidad efimera {}",
        identity.public_key().short_fingerprint()
    );

    let probe = nexus_windows_host::signaling::probe(
        &url,
        &info,
        &capabilities,
        &identity,
        token.as_deref(),
        1,
    )
    .await;

    println!();
    println!("  conectado ................ {}", si_o_no(probe.connected));
    println!("  servidores de red ........ {}", probe.ice_servers);
    println!("  relay TURN ............... {}", si_o_no(probe.has_relay));
    println!(
        "  alcanzable desde otra red  {}",
        si_o_no(probe.supports_remote_networks)
    );
    println!();
    println!("  {}", probe.detail);

    if probe.connected && probe.has_relay {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

const fn si_o_no(valor: bool) -> &'static str {
    if valor { "si" } else { "NO" }
}
