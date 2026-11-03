//! Utilidades compartidas por las pruebas de integracion de red.
//!
//! Lo que monta aqui es el servidor **de verdad**: el mismo
//! `nexus_signaling::serve` que usa el binario, escuchando en un puerto libre
//! de la maquina local. No hay dobles ni simulaciones del servidor.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nexus_core::device::{DeviceInfo, DevicePlatform, DeviceRole};
use nexus_core::{Capability, CapabilitySet, DeviceId};
use nexus_security::DeviceIdentity;
use nexus_signaling::{AppState, Config};
use nexus_transport::client::{Accepted, ConnectRequest, SignalingClient};

/// Testigo de alta del servidor de pruebas.
pub const TOKEN: &str = "testigo-de-alta-para-las-pruebas-de-integracion";

/// Tope de espera para cualquier paso de senalizacion.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Un servidor de senalizacion en marcha, escuchando en la maquina local.
pub struct TestServer {
    /// URL del canal de senalizacion.
    pub url: String,
    /// Estado del servidor, para poder mirar su registro desde la prueba.
    pub state: Arc<AppState>,
    /// Direccion en la que escucha.
    pub address: SocketAddr,
}

impl TestServer {
    /// Arranca un servidor en un puerto libre.
    ///
    /// Los servidores STUN y TURN que anuncia apuntan a la propia maquina en un
    /// puerto cerrado. Es deliberado: la configuracion tiene que ser valida
    /// (sin ella el cliente se niega a conectar, y con razon), pero en una
    /// prueba no hay ningun Coturn en marcha. Lo que se comprueba aqui es la
    /// senalizacion y la conexion directa; que el relay funcione de verdad hace
    /// falta desplegarlo.
    pub async fn start() -> Self {
        let config = Config::from_lookup(|name| match name {
            "NEXUS_PUBLIC_URL" => Some("wss://pruebas.invalido/v1/signaling".to_owned()),
            "NEXUS_TURN_SHARED_SECRET" => Some("secreto-de-turn-para-las-pruebas".to_owned()),
            "NEXUS_TURN_URLS" => Some("turn:127.0.0.1:3478".to_owned()),
            "NEXUS_STUN_URLS" => Some("stun:127.0.0.1:3478".to_owned()),
            "NEXUS_HOST_ENROLLMENT_TOKEN" => Some(TOKEN.to_owned()),
            _ => None,
        })
        .expect("configuracion de prueba valida");

        let state = Arc::new(AppState::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("un puerto libre en la maquina local");
        let address = listener.local_addr().expect("direccion de escucha");

        let served = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = nexus_signaling::serve(listener, served, std::future::pending()).await;
        });

        Self {
            url: format!("ws://{address}/v1/signaling"),
            state,
            address,
        }
    }

    /// Lineas del registro de conexiones del servidor.
    pub fn audit_lines(&self) -> Vec<String> {
        self.state
            .shared()
            .audit
            .entries()
            .map(ToString::to_string)
            .collect()
    }
}

/// Un dispositivo de prueba: su identidad y los datos que declara.
pub struct Device {
    /// Identidad criptografica.
    pub identity: DeviceIdentity,
    /// Datos declarados.
    pub info: DeviceInfo,
    /// Capacidades que anuncia.
    pub capabilities: CapabilitySet,
}

impl Device {
    /// Crea un host de prueba.
    pub fn host(name: &str) -> Self {
        Self::new(name, DeviceRole::Host, DevicePlatform::Windows)
    }

    /// Crea un controlador de prueba.
    pub fn controller(name: &str) -> Self {
        Self::new(name, DeviceRole::Controller, DevicePlatform::IpadOs)
    }

    fn new(name: &str, role: DeviceRole, platform: DevicePlatform) -> Self {
        let id = DeviceId::new();
        let mut capabilities = CapabilitySet::empty();
        capabilities.insert(Capability::ScreenStream);
        capabilities.insert(Capability::RemoteInput);
        capabilities.insert(Capability::MultiMonitor);

        Self {
            identity: DeviceIdentity::generate(id),
            info: DeviceInfo {
                id,
                display_name: name.to_owned(),
                platform,
                role,
                os_version: "1.0".to_owned(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities,
        }
    }

    /// Identificador del dispositivo.
    pub fn id(&self) -> DeviceId {
        self.info.id
    }

    /// Copia de la identidad, para tareas que no pueden tomarla prestada.
    pub fn duplicate_identity(&self) -> DeviceIdentity {
        DeviceIdentity::from_secret_bytes(self.info.id, self.identity.export_secret().as_slice())
            .expect("la misma clave")
    }

    /// Se conecta al servidor y se autentica.
    pub async fn connect(
        &self,
        server: &TestServer,
        token: Option<&str>,
    ) -> Result<(SignalingClient, Accepted), nexus_transport::TransportError> {
        SignalingClient::connect(
            ConnectRequest {
                url: &server.url,
                device: &self.info,
                capabilities: &self.capabilities,
                enrollment_token: token,
            },
            &self.identity,
        )
        .await
    }

    /// Se conecta y falla la prueba si no lo consigue.
    pub async fn connect_ok(
        &self,
        server: &TestServer,
        token: Option<&str>,
    ) -> (SignalingClient, Accepted) {
        self.connect(server, token)
            .await
            .unwrap_or_else(|e| panic!("{} deberia autenticarse: {e}", self.info.display_name))
    }
}

/// Inyector de entrada cuyo registro vive fuera del manejador.
///
/// El `SessionHandler` del host se queda con el inyector que se le pasa, asi
/// que para poder comprobar que llego hace falta uno que comparta su registro.
#[derive(Clone, Default)]
pub struct InyectorCompartido(std::sync::Arc<std::sync::Mutex<Vec<nexus_input::ResolvedInput>>>);

impl InyectorCompartido {
    /// Ordenes que ha recibido, en orden.
    pub fn recibidas(&self) -> Vec<nexus_input::ResolvedInput> {
        self.0.lock().expect("sin envenenar").clone()
    }
}

impl nexus_input::InputInjector for InyectorCompartido {
    fn name(&self) -> &'static str {
        "compartido"
    }

    fn inject(&mut self, input: &nexus_input::ResolvedInput) -> nexus_input::InputResult<()> {
        self.0.lock().expect("sin envenenar").push(input.clone());
        Ok(())
    }

    fn release_all(&mut self) -> nexus_input::InputResult<()> {
        Ok(())
    }
}
