//! Un host real con una pantalla sintetica, para poder ejecutar el cliente web.
//!
//! No es una demostracion ni un producto: es **el otro extremo** que hace falta
//! para que el cliente web se pueda ejecutar y comprobar en esta maquina.
//! Arranca el servidor de senalizacion de verdad y el bucle de verdad del host,
//! con su autenticacion, su atadura de sesion y su envio de pantalla.
//!
//! Lo unico que no es real son los pixeles: la captura solo funciona en
//! Windows, asi que los fotogramas vienen de [`SyntheticCapturer`], que es
//! color plano y lo dice en su nombre.
//!
//! El dispositivo que va a controlar se declara emparejado por variables de
//! entorno. Eso ocupa el lugar del emparejamiento de verdad, que ya existe y se
//! prueba aparte; aqui se da por hecho para no repetirlo en cada arranque.
//!
//! ```text
//! NEXUS_WEB_CONTROLADOR_ID=<uuid> \
//! NEXUS_WEB_CONTROLADOR_CLAVE=<hex de 64> \
//!   cargo run -p nexus-windows-host --example host_de_pruebas
//! ```
//!
//! Escribe una linea JSON en la salida con la URL y el identificador del host,
//! y se queda atendiendo hasta que se le mate.

use std::sync::Arc;

use nexus_capture::capturer::ScreenCapturer;
use nexus_capture::error::CaptureError;
use nexus_core::device::{DeviceInfo, DevicePlatform, DeviceRole};
use nexus_core::monitor::{MonitorInfo, MonitorLayout};
use nexus_core::{DeviceId, PermissionSet, Timestamp};
use nexus_security::DeviceIdentity;
use nexus_security::registry::DeviceRecord;
use nexus_signaling::{AppState, Config};
use nexus_windows_host::runtime::{HostRuntime, InMemoryLookup, PlatformInjectors, RuntimeConfig};
use nexus_windows_host::screen::CapturerSource;

/// Testigo de alta del servidor que se arranca aqui.
const TESTIGO: &str = "testigo-de-alta-del-host-de-pruebas-del-cliente-web";

/// La pantalla que el host declara y captura.
fn pantalla() -> MonitorLayout {
    MonitorLayout(vec![MonitorInfo {
        index: 0,
        name: "Pantalla de pruebas".to_owned(),
        width: 1_280,
        height: 720,
        origin_x: 0,
        origin_y: 0,
        scale_percent: 100,
        is_primary: true,
        refresh_hz: Some(60),
    }])
}

/// Fuente del capturador sintetico. **No es captura.**
struct PantallaSintetica;

impl CapturerSource for PantallaSintetica {
    fn create(&self) -> Result<Box<dyn ScreenCapturer>, CaptureError> {
        Ok(Box::new(nexus_capture::SyntheticCapturer::new(pantalla())))
    }
}

/// Lee una variable de entorno o explica que falta y termina.
fn pedir(nombre: &str) -> String {
    match std::env::var(nombre) {
        Ok(valor) if !valor.trim().is_empty() => valor,
        _ => {
            eprintln!("falta la variable {nombre}; vea el comentario de este ejemplo");
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,nexus=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    // Quien va a controlar. Su clave publica llega de fuera porque la genera el
    // navegador: aqui solo se declara como emparejado.
    let controlador_id: DeviceId = pedir("NEXUS_WEB_CONTROLADOR_ID")
        .parse()
        .unwrap_or_else(|e| {
            eprintln!("NEXUS_WEB_CONTROLADOR_ID no es un identificador valido: {e}");
            std::process::exit(2);
        });
    let clave = nexus_security::DevicePublicKey::from_hex(&pedir("NEXUS_WEB_CONTROLADOR_CLAVE"))
        .unwrap_or_else(|e| {
            eprintln!("NEXUS_WEB_CONTROLADOR_CLAVE no es una clave Ed25519 valida: {e}");
            std::process::exit(2);
        });

    // El servidor de senalizacion de verdad, en un puerto libre.
    let config = Config::from_lookup(|name| match name {
        "NEXUS_PUBLIC_URL" => Some("ws://127.0.0.1/v1/signaling".to_owned()),
        "NEXUS_TURN_SHARED_SECRET" => Some("secreto-de-turn-del-host-de-pruebas".to_owned()),
        "NEXUS_TURN_URLS" => Some("turn:127.0.0.1:3478".to_owned()),
        "NEXUS_STUN_URLS" => Some("stun:127.0.0.1:3478".to_owned()),
        "NEXUS_HOST_ENROLLMENT_TOKEN" => Some(TESTIGO.to_owned()),
        _ => None,
    })
    .expect("la configuracion del servidor de pruebas es valida");

    let state = Arc::new(AppState::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("un puerto libre en la maquina local");
    let address = listener.local_addr().expect("direccion de escucha");
    let url = format!("ws://{address}/v1/signaling");

    let servido = Arc::clone(&state);
    tokio::spawn(async move {
        let _ = nexus_signaling::serve(listener, servido, std::future::pending()).await;
    });

    // El host: identidad propia, registro con el controlador ya emparejado y
    // acceso desatendido, que es lo que permite entrar sin que nadie apruebe
    // nada en el PC.
    let host_id = DeviceId::new();
    let identity = DeviceIdentity::generate(host_id);
    let mut registro = InMemoryLookup::new();
    registro.insert(DeviceRecord::new(
        DeviceInfo {
            id: controlador_id,
            display_name: "Cliente web".to_owned(),
            platform: DevicePlatform::Unknown,
            role: DeviceRole::Controller,
            os_version: "navegador".to_owned(),
            app_version: "nexus-web".to_owned(),
        },
        clave,
        PermissionSet::standard(),
        true,
        Timestamp::now(),
    ));

    let (eventos, mut recibe) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(evento) = recibe.recv().await {
            eprintln!("[host] {evento:?}");
        }
    });

    let runtime = HostRuntime::new(
        RuntimeConfig {
            url: url.clone(),
            device: DeviceInfo {
                id: host_id,
                display_name: "PC de pruebas".to_owned(),
                platform: DevicePlatform::Linux,
                role: DeviceRole::Host,
                os_version: std::env::consts::OS.to_owned(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            enrollment_token: Some(TESTIGO.to_owned()),
            monitors: pantalla(),
            udp_port: 0,
            // Sin reescritura de direccion: el navegador esta en esta misma
            // maquina y los candidatos reales le sirven.
            public_endpoint: None,
            shared_roots: Vec::new(),
        },
        identity,
        Arc::new(registro),
        Arc::new(PlatformInjectors),
        eventos,
    )
    .with_capturers(Arc::new(PantallaSintetica));

    // Una linea, para que quien lo arranque sepa a donde conectarse.
    println!(
        "{}",
        serde_json::json!({ "url": url, "host_id": host_id.to_string() })
    );

    runtime.run(std::future::pending()).await;
}
