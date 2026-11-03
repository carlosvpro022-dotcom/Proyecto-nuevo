//! Pares conectados y sesiones en curso.
//!
//! El centro de reparto del servidor. Guarda quien esta conectado y autenticado,
//! que hosts se han declarado disponibles y que sesiones hay abiertas, y mueve
//! los mensajes de un extremo al otro.
//!
//! Tres decisiones que conviene entender:
//!
//! - **Es sincrono.** Ninguna funcion es `async`: todas terminan en un
//!   `try_send` que no espera. Asi puede vivir detras de un cerrojo corto sin
//!   riesgo de bloquear el runtime, y se puede probar entero sin montar
//!   sockets.
//! - **Un cliente lento se desconecta.** Si su cola de salida se llena, se le
//!   retira en lugar de acumular mensajes: la memoria del servidor no puede
//!   depender de lo rapido que lea un movil con mala cobertura.
//! - **Una sesion nueva del mismo dispositivo desplaza a la anterior.** Es lo
//!   que espera cualquiera que reconecte tras un corte de red, y evita que una
//!   conexion zombi retenga el host.

use std::collections::HashMap;

use nexus_core::device::DeviceRole;
use nexus_core::protocol::signaling::ServerToClient;
use nexus_core::{DeviceId, DeviceInfo, SessionId, Timestamp};
use tokio::sync::mpsc;

/// Mensajes que caben en la cola de salida de un cliente.
///
/// Un cliente al dia tiene la cola casi siempre vacia: los mensajes de
/// senalizacion son pocos y pequenos. Sesenta y cuatro absorben una rafaga de
/// candidatos ICE sin dar margen a que un cliente que no lee acumule memoria.
pub const PEER_QUEUE_DEPTH: usize = 64;

/// Sesiones simultaneas que admite el servidor.
pub const DEFAULT_MAX_SESSIONS: usize = 256;

/// Fallos de enrutado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubError {
    /// El host no esta conectado, o no se ha declarado disponible.
    HostUnavailable,
    /// El host ya tiene una sesion en curso.
    HostBusy,
    /// Quien lo pide ya esta en una sesion.
    AlreadyInSession,
    /// La sesion no existe.
    UnknownSession,
    /// Quien lo pide no es parte de esa sesion.
    NotAMember,
    /// El otro extremo se desconecto.
    PeerGone,
    /// El otro extremo no lee lo bastante rapido y se le ha retirado.
    PeerOverloaded,
    /// Se alcanzo el tope de sesiones simultaneas.
    TooManySessions,
    /// La operacion corresponde a un host y la pide un controlador, o al revés.
    WrongRole,
}

impl HubError {
    /// Codigo estable que se envia al cliente.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::HostUnavailable => "host_unavailable",
            Self::HostBusy => "host_busy",
            Self::AlreadyInSession => "already_in_session",
            Self::UnknownSession => "unknown_session",
            Self::NotAMember => "not_a_member",
            Self::PeerGone => "peer_gone",
            Self::PeerOverloaded => "peer_overloaded",
            Self::TooManySessions => "too_many_sessions",
            Self::WrongRole => "wrong_role",
        }
    }

    /// Mensaje legible para el cliente.
    ///
    /// No distingue "el host no existe" de "el host esta apagado": ambos son
    /// `HostUnavailable`, para no confirmar a nadie que un identificador existe.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::HostUnavailable => "el equipo no esta disponible",
            Self::HostBusy => "el equipo ya tiene una sesion en curso",
            Self::AlreadyInSession => "ya hay una sesion en curso en este dispositivo",
            Self::UnknownSession => "la sesion no existe",
            Self::NotAMember => "la sesion no es de este dispositivo",
            Self::PeerGone => "el otro extremo se desconecto",
            Self::PeerOverloaded => "el otro extremo no responde",
            Self::TooManySessions => "el servidor esta al limite de sesiones",
            Self::WrongRole => "la operacion no corresponde a este dispositivo",
        }
    }
}

/// Identifica una conexion concreta de un dispositivo.
///
/// Hace falta porque un mismo dispositivo puede tener dos conexiones durante un
/// instante: la que se cae y la que toma el relevo. Sin esto, la limpieza de la
/// conexion antigua retiraria del registro a la nueva, que seguiria con su
/// socket abierto pero invisible para el servidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionToken(u64);

/// Resultado de registrar una conexion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attachment {
    /// Testigo con el que esta conexion debe retirarse.
    pub token: ConnectionToken,
    /// Si desplazo a una conexion anterior del mismo dispositivo.
    pub replaced: bool,
}

/// Momento del ciclo de vida de una sesion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// El controlador la pidio y el host todavia no contesto.
    Requested,
    /// El host acepto; se esta negociando WebRTC.
    Negotiating,
}

/// Sesion registrada en el servidor.
///
/// El servidor sabe **quienes** hablan y **cuando**, nunca **que** se dice: el
/// contenido va cifrado extremo a extremo por el canal de datos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    /// Identificador de la sesion.
    pub session_id: SessionId,
    /// Host de la sesion.
    pub host: DeviceId,
    /// Controlador de la sesion.
    pub controller: DeviceId,
    /// Momento del ciclo de vida.
    pub phase: SessionPhase,
    /// Instante en que se pidio.
    pub opened_at: Timestamp,
}

impl SessionEntry {
    /// Indica si el dispositivo forma parte de la sesion.
    #[must_use]
    pub fn includes(&self, device_id: DeviceId) -> bool {
        self.host == device_id || self.controller == device_id
    }

    /// El otro extremo de la sesion, visto desde `device_id`.
    #[must_use]
    pub fn counterpart(&self, device_id: DeviceId) -> Option<DeviceId> {
        if self.host == device_id {
            Some(self.controller)
        } else if self.controller == device_id {
            Some(self.host)
        } else {
            None
        }
    }
}

/// Resumen publicable de un host disponible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableHost {
    /// Identificador del host.
    pub device_id: DeviceId,
    /// Nombre declarado.
    pub display_name: String,
    /// Monitores que anuncio.
    pub monitor_count: u8,
    /// Si tiene una sesion en curso.
    pub busy: bool,
}

#[derive(Debug)]
struct Peer {
    info: DeviceInfo,
    outbound: mpsc::Sender<ServerToClient>,
    /// Conexion concreta a la que corresponde esta entrada.
    token: ConnectionToken,
    /// Monitores anunciados, una vez el host se declara disponible.
    announced_monitors: Option<u8>,
    session: Option<SessionId>,
}

/// Registro de pares conectados y sesiones.
#[derive(Debug)]
pub struct Hub {
    peers: HashMap<DeviceId, Peer>,
    sessions: HashMap<SessionId, SessionEntry>,
    max_sessions: usize,
    next_token: u64,
}

impl Hub {
    /// Crea un centro de reparto vacio.
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_sessions(DEFAULT_MAX_SESSIONS)
    }

    /// Crea el centro indicando el tope de sesiones simultaneas.
    #[must_use]
    pub fn with_max_sessions(max_sessions: usize) -> Self {
        Self {
            peers: HashMap::new(),
            sessions: HashMap::new(),
            max_sessions,
            next_token: 1,
        }
    }

    /// Conexiones autenticadas en este momento.
    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Sesiones registradas en este momento.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Indica si el dispositivo tiene una conexion autenticada.
    #[must_use]
    pub fn is_connected(&self, device_id: DeviceId) -> bool {
        self.peers.contains_key(&device_id)
    }

    /// Datos de una sesion.
    #[must_use]
    pub fn session(&self, session_id: SessionId) -> Option<&SessionEntry> {
        self.sessions.get(&session_id)
    }

    /// Hosts disponibles en este momento.
    #[must_use]
    pub fn available_hosts(&self) -> Vec<AvailableHost> {
        let mut hosts: Vec<_> = self
            .peers
            .iter()
            .filter_map(|(id, peer)| {
                peer.announced_monitors.map(|monitor_count| AvailableHost {
                    device_id: *id,
                    display_name: peer.info.sanitized_name(),
                    monitor_count,
                    busy: peer.session.is_some(),
                })
            })
            .collect();
        hosts.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        hosts
    }

    /// Registra una conexion autenticada.
    ///
    /// Si el dispositivo ya tenia una conexion, se le avisa y se cierra: una
    /// reconexion tras un corte de red no debe quedarse fuera porque la
    /// conexion anterior aun no se haya dado por muerta.
    ///
    /// Devuelve el testigo con el que esta conexion debera retirarse.
    pub fn attach(
        &mut self,
        info: DeviceInfo,
        outbound: mpsc::Sender<ServerToClient>,
    ) -> Attachment {
        let device_id = info.id;
        let replaced = self.peers.contains_key(&device_id);
        if replaced {
            // El aviso va antes de retirar la conexion: queda en su cola de
            // salida y lo recibe justo antes de que se cierre el socket.
            self.notify(
                device_id,
                ServerToClient::Error {
                    code: "connection_replaced".to_owned(),
                    message: "otra conexion de este mismo dispositivo tomo el relevo".to_owned(),
                },
            );
            self.detach(device_id);
        }

        let token = ConnectionToken(self.next_token);
        self.next_token = self.next_token.saturating_add(1);
        self.peers.insert(
            device_id,
            Peer {
                info,
                outbound,
                token,
                announced_monitors: None,
                session: None,
            },
        );
        Attachment { token, replaced }
    }

    /// Retira una conexion **solo si sigue siendo la vigente**.
    ///
    /// Es lo que debe usar una conexion al cerrarse. Si entre medias el
    /// dispositivo reconecto, la entrada del registro ya es de la conexion
    /// nueva y no hay que tocarla: retirarla dejaria al cliente nuevo con el
    /// socket abierto pero invisible para el servidor.
    ///
    /// Devuelve las sesiones que se cerraron, vacio si no habia nada que
    /// retirar.
    pub fn detach_if_current(
        &mut self,
        device_id: DeviceId,
        token: ConnectionToken,
    ) -> Vec<SessionId> {
        match self.peers.get(&device_id) {
            Some(peer) if peer.token == token => self.detach(device_id),
            _ => Vec::new(),
        }
    }

    /// Retira una conexion y cierra sus sesiones, avisando al otro extremo.
    ///
    /// Devuelve las sesiones que se cerraron.
    pub fn detach(&mut self, device_id: DeviceId) -> Vec<SessionId> {
        let Some(peer) = self.peers.remove(&device_id) else {
            return Vec::new();
        };

        let affected: Vec<SessionId> = peer.session.into_iter().collect();
        for session_id in &affected {
            if let Some(entry) = self.sessions.remove(session_id)
                && let Some(other) = entry.counterpart(device_id)
            {
                if let Some(other_peer) = self.peers.get_mut(&other) {
                    other_peer.session = None;
                }
                self.notify(
                    other,
                    ServerToClient::PeerGone {
                        session_id: *session_id,
                    },
                );
            }
        }
        affected
    }

    /// Declara un host disponible para recibir sesiones.
    ///
    /// # Errors
    ///
    /// - [`HubError::HostUnavailable`] si no hay conexion para ese dispositivo.
    /// - [`HubError::WrongRole`] si el dispositivo no se declaro como host.
    pub fn register_host(
        &mut self,
        device_id: DeviceId,
        monitor_count: u8,
    ) -> Result<(), HubError> {
        let peer = self
            .peers
            .get_mut(&device_id)
            .ok_or(HubError::HostUnavailable)?;
        if peer.info.role != DeviceRole::Host {
            return Err(HubError::WrongRole);
        }
        peer.announced_monitors = Some(monitor_count);
        Ok(())
    }

    /// Abre una sesion de un controlador contra un host.
    ///
    /// El servidor **no decide si el controlador tiene derecho**: solo comprueba
    /// que el host esta disponible y le pasa la peticion. La autorizacion la
    /// resuelve el host contra su propio registro de dispositivos emparejados.
    ///
    /// # Errors
    ///
    /// - [`HubError::WrongRole`] si quien lo pide no es un controlador.
    /// - [`HubError::AlreadyInSession`] si el controlador ya tiene sesion.
    /// - [`HubError::HostUnavailable`] si el host no esta conectado o no se
    ///   declaro disponible. Es el mismo error en los dos casos a proposito.
    /// - [`HubError::HostBusy`] si el host ya atiende otra sesion.
    /// - [`HubError::TooManySessions`] si el servidor esta al limite.
    pub fn open_session(
        &mut self,
        controller: DeviceId,
        host: DeviceId,
        now: Timestamp,
    ) -> Result<SessionEntry, HubError> {
        if self.sessions.len() >= self.max_sessions {
            return Err(HubError::TooManySessions);
        }

        let controller_peer = self
            .peers
            .get(&controller)
            .ok_or(HubError::HostUnavailable)?;
        if controller_peer.info.role != DeviceRole::Controller {
            return Err(HubError::WrongRole);
        }
        if controller_peer.session.is_some() {
            return Err(HubError::AlreadyInSession);
        }

        let host_peer = self.peers.get(&host).ok_or(HubError::HostUnavailable)?;
        if host_peer.info.role != DeviceRole::Host || host_peer.announced_monitors.is_none() {
            return Err(HubError::HostUnavailable);
        }
        if host_peer.session.is_some() {
            return Err(HubError::HostBusy);
        }

        let entry = SessionEntry {
            session_id: SessionId::new(),
            host,
            controller,
            phase: SessionPhase::Requested,
            opened_at: now,
        };
        self.sessions.insert(entry.session_id, entry.clone());
        for id in [host, controller] {
            if let Some(peer) = self.peers.get_mut(&id) {
                peer.session = Some(entry.session_id);
            }
        }
        Ok(entry)
    }

    /// Marca una sesion como aceptada por el host.
    ///
    /// # Errors
    ///
    /// - [`HubError::UnknownSession`] si no existe.
    /// - [`HubError::NotAMember`] si no la acepta su host.
    pub fn accept_session(
        &mut self,
        session_id: SessionId,
        host: DeviceId,
    ) -> Result<SessionEntry, HubError> {
        let entry = self
            .sessions
            .get_mut(&session_id)
            .ok_or(HubError::UnknownSession)?;
        if entry.host != host {
            return Err(HubError::NotAMember);
        }
        entry.phase = SessionPhase::Negotiating;
        Ok(entry.clone())
    }

    /// Cierra una sesion a peticion de uno de sus extremos.
    ///
    /// # Errors
    ///
    /// - [`HubError::UnknownSession`] si no existe.
    /// - [`HubError::NotAMember`] si quien la cierra no es parte de ella.
    pub fn close_session(
        &mut self,
        session_id: SessionId,
        by: DeviceId,
    ) -> Result<SessionEntry, HubError> {
        let entry = self
            .sessions
            .get(&session_id)
            .ok_or(HubError::UnknownSession)?
            .clone();
        if !entry.includes(by) {
            return Err(HubError::NotAMember);
        }

        self.sessions.remove(&session_id);
        for id in [entry.host, entry.controller] {
            if let Some(peer) = self.peers.get_mut(&id)
                && peer.session == Some(session_id)
            {
                peer.session = None;
            }
        }
        Ok(entry)
    }

    /// Reenvia un mensaje al otro extremo de una sesion.
    ///
    /// El mensaje se construye con una funcion que recibe el identificador del
    /// emisor, para que el servidor pueda rellenar el campo "quien lo envia"
    /// sin que el emisor pueda falsearlo.
    ///
    /// # Errors
    ///
    /// - [`HubError::UnknownSession`] o [`HubError::NotAMember`] si la sesion no
    ///   es de quien envia.
    /// - [`HubError::PeerGone`] o [`HubError::PeerOverloaded`] si el otro
    ///   extremo no puede recibirlo.
    pub fn relay<F>(
        &mut self,
        session_id: SessionId,
        from: DeviceId,
        build: F,
    ) -> Result<DeviceId, HubError>
    where
        F: FnOnce(DeviceId) -> ServerToClient,
    {
        let entry = self
            .sessions
            .get(&session_id)
            .ok_or(HubError::UnknownSession)?;
        let target = entry.counterpart(from).ok_or(HubError::NotAMember)?;
        self.send(target, build(from))?;
        Ok(target)
    }

    /// Envia un mensaje a un dispositivo concreto.
    ///
    /// # Errors
    ///
    /// - [`HubError::PeerGone`] si no esta conectado o cerro el canal.
    /// - [`HubError::PeerOverloaded`] si su cola esta llena; en ese caso se le
    ///   retira la conexion.
    pub fn send(&mut self, to: DeviceId, message: ServerToClient) -> Result<(), HubError> {
        let Some(peer) = self.peers.get(&to) else {
            return Err(HubError::PeerGone);
        };

        match peer.outbound.try_send(message) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // No se acumula: quien no lee, se va.
                self.detach(to);
                Err(HubError::PeerOverloaded)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.detach(to);
                Err(HubError::PeerGone)
            }
        }
    }

    /// Envia un mensaje sin que importe si llega.
    ///
    /// Para avisos de cortesia (`PeerGone`, conexion desplazada) donde no hay
    /// nada que hacer si el destinatario ya no esta.
    fn notify(&mut self, to: DeviceId, message: ServerToClient) {
        let _ = self.send(to, message);
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::device::DevicePlatform;

    const NOW: Timestamp = Timestamp(1_730_000_000_000);

    fn info(role: DeviceRole, name: &str) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId::new(),
            display_name: name.to_owned(),
            platform: match role {
                DeviceRole::Host => DevicePlatform::Windows,
                DeviceRole::Controller => DevicePlatform::IpadOs,
            },
            role,
            os_version: "1.0".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    struct Conn {
        info: DeviceInfo,
        inbox: mpsc::Receiver<ServerToClient>,
    }

    impl Conn {
        fn id(&self) -> DeviceId {
            self.info.id
        }

        fn next(&mut self) -> Option<ServerToClient> {
            self.inbox.try_recv().ok()
        }

        fn is_empty(&mut self) -> bool {
            self.inbox.try_recv().is_err()
        }
    }

    fn join(hub: &mut Hub, role: DeviceRole, name: &str) -> Conn {
        let (tx, inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
        let info = info(role, name);
        hub.attach(info.clone(), tx);
        Conn { info, inbox }
    }

    /// Un host conectado y ya declarado disponible.
    fn join_host(hub: &mut Hub, name: &str) -> Conn {
        let conn = join(hub, DeviceRole::Host, name);
        hub.register_host(conn.id(), 2).expect("se declara");
        conn
    }

    #[test]
    fn a_fresh_hub_is_empty() {
        let hub = Hub::new();
        assert_eq!(hub.peer_count(), 0);
        assert_eq!(hub.session_count(), 0);
        assert!(hub.available_hosts().is_empty());
    }

    #[test]
    fn a_host_appears_only_once_it_declares_itself() {
        let mut hub = Hub::new();
        let host = join(&mut hub, DeviceRole::Host, "PC");
        assert!(
            hub.available_hosts().is_empty(),
            "conectado no es lo mismo que disponible"
        );

        hub.register_host(host.id(), 3).expect("se declara");
        let hosts = hub.available_hosts();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].monitor_count, 3);
        assert!(!hosts[0].busy);
    }

    #[test]
    fn a_controller_cannot_declare_itself_a_host() {
        let mut hub = Hub::new();
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        assert_eq!(
            hub.register_host(controller.id(), 1),
            Err(HubError::WrongRole)
        );
    }

    #[test]
    fn declaring_an_unconnected_host_fails() {
        let mut hub = Hub::new();
        assert_eq!(
            hub.register_host(DeviceId::new(), 1),
            Err(HubError::HostUnavailable)
        );
    }

    #[test]
    fn the_happy_path_opens_and_closes_a_session() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");

        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("deberia abrir");
        assert_eq!(entry.phase, SessionPhase::Requested);
        assert_eq!(hub.session_count(), 1);

        let accepted = hub
            .accept_session(entry.session_id, host.id())
            .expect("el host acepta");
        assert_eq!(accepted.phase, SessionPhase::Negotiating);

        hub.close_session(entry.session_id, controller.id())
            .expect("cierra");
        assert_eq!(hub.session_count(), 0);
    }

    #[test]
    fn an_unavailable_host_and_an_unknown_host_look_the_same() {
        // Importa para no confirmar a un atacante que un identificador existe.
        let mut hub = Hub::new();
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let connected_but_silent = join(&mut hub, DeviceRole::Host, "PC");

        let unknown = hub.open_session(controller.id(), DeviceId::new(), NOW);
        let silent = hub.open_session(controller.id(), connected_but_silent.id(), NOW);
        assert_eq!(unknown, Err(HubError::HostUnavailable));
        assert_eq!(silent, Err(HubError::HostUnavailable));
    }

    #[test]
    fn a_busy_host_refuses_a_second_session() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let first = join(&mut hub, DeviceRole::Controller, "iPad");
        let second = join(&mut hub, DeviceRole::Controller, "iPhone");

        hub.open_session(first.id(), host.id(), NOW).expect("abre");
        assert_eq!(
            hub.open_session(second.id(), host.id(), NOW),
            Err(HubError::HostBusy)
        );
    }

    #[test]
    fn a_controller_cannot_open_two_sessions() {
        let mut hub = Hub::new();
        let first_host = join_host(&mut hub, "PC uno");
        let second_host = join_host(&mut hub, "PC dos");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");

        hub.open_session(controller.id(), first_host.id(), NOW)
            .expect("abre");
        assert_eq!(
            hub.open_session(controller.id(), second_host.id(), NOW),
            Err(HubError::AlreadyInSession)
        );
    }

    #[test]
    fn a_host_cannot_open_a_session_itself() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let other = join_host(&mut hub, "PC dos");
        assert_eq!(
            hub.open_session(host.id(), other.id(), NOW),
            Err(HubError::WrongRole)
        );
    }

    #[test]
    fn the_session_limit_is_enforced() {
        let mut hub = Hub::with_max_sessions(1);
        let first_host = join_host(&mut hub, "PC uno");
        let second_host = join_host(&mut hub, "PC dos");
        let first = join(&mut hub, DeviceRole::Controller, "iPad");
        let second = join(&mut hub, DeviceRole::Controller, "iPhone");

        hub.open_session(first.id(), first_host.id(), NOW)
            .expect("cabe");
        assert_eq!(
            hub.open_session(second.id(), second_host.id(), NOW),
            Err(HubError::TooManySessions)
        );
    }

    #[test]
    fn only_the_host_of_a_session_can_accept_it() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("abre");

        assert_eq!(
            hub.accept_session(entry.session_id, controller.id()),
            Err(HubError::NotAMember),
            "el controlador no puede aceptarse a si mismo"
        );
    }

    #[test]
    fn a_stranger_cannot_close_a_session() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let intruder = join(&mut hub, DeviceRole::Controller, "iPhone");
        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("abre");

        assert_eq!(
            hub.close_session(entry.session_id, intruder.id()),
            Err(HubError::NotAMember)
        );
        assert_eq!(hub.session_count(), 1, "la sesion sigue abierta");
    }

    #[test]
    fn relaying_reaches_the_other_end_and_only_el_otro() {
        let mut hub = Hub::new();
        let mut host = join_host(&mut hub, "PC");
        let mut controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("abre");
        let session_id = entry.session_id;

        let target = hub
            .relay(session_id, controller.id(), |_| ServerToClient::Offer {
                session_id,
                sdp: "v=0".to_owned(),
            })
            .expect("reenvia");
        assert_eq!(target, host.id());

        assert!(matches!(host.next(), Some(ServerToClient::Offer { .. })));
        assert!(
            controller.is_empty(),
            "el emisor no debe recibir su propio mensaje"
        );
    }

    #[test]
    fn the_server_fills_in_who_sent_a_relayed_message() {
        // El emisor no puede falsear quien envia: el servidor lo rellena.
        let mut hub = Hub::new();
        let mut host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("abre");
        let session_id = entry.session_id;

        hub.relay(session_id, controller.id(), |from| {
            ServerToClient::PeerAuth {
                session_id,
                device_id: from,
                fingerprint: "sha-256 AA".to_owned(),
                signature_hex: "ab".to_owned(),
            }
        })
        .expect("reenvia");

        match host.next() {
            Some(ServerToClient::PeerAuth { device_id, .. }) => {
                assert_eq!(device_id, controller.id());
            }
            other => panic!("se esperaba PeerAuth, llego {other:?}"),
        }
    }

    #[test]
    fn relaying_outside_your_own_session_is_refused() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let intruder = join(&mut hub, DeviceRole::Controller, "iPhone");
        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("abre");
        let session_id = entry.session_id;

        let result = hub.relay(session_id, intruder.id(), |_| ServerToClient::Offer {
            session_id,
            sdp: "v=0".to_owned(),
        });
        assert_eq!(result, Err(HubError::NotAMember));
    }

    #[test]
    fn relaying_to_an_unknown_session_is_refused() {
        let mut hub = Hub::new();
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let session_id = SessionId::new();
        let result = hub.relay(session_id, controller.id(), |_| ServerToClient::PeerGone {
            session_id,
        });
        assert_eq!(result, Err(HubError::UnknownSession));
    }

    #[test]
    fn disconnecting_tells_the_other_end() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let mut controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("abre");

        let closed = hub.detach(host.id());
        assert_eq!(closed, vec![entry.session_id]);
        assert_eq!(hub.session_count(), 0);
        assert!(matches!(
            controller.next(),
            Some(ServerToClient::PeerGone { .. })
        ));
    }

    #[test]
    fn after_a_disconnection_the_controller_can_open_a_new_session() {
        // Si al desconectarse el host no se liberase la sesion del controlador,
        // este se quedaria bloqueado hasta reiniciar la aplicacion.
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        hub.open_session(controller.id(), host.id(), NOW)
            .expect("abre");
        hub.detach(host.id());

        let host_again = join_host(&mut hub, "PC");
        hub.open_session(controller.id(), host_again.id(), NOW)
            .expect("deberia poder volver a conectar");
    }

    #[test]
    fn reconnecting_replaces_the_previous_connection() {
        let mut hub = Hub::new();
        let mut first = join(&mut hub, DeviceRole::Controller, "iPad");

        let (tx, _second_inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
        let replaced = hub.attach(first.info.clone(), tx).replaced;

        assert!(replaced);
        assert_eq!(hub.peer_count(), 1, "no deben quedar dos");
        match first.next() {
            Some(ServerToClient::Error { code, .. }) => {
                assert_eq!(code, "connection_replaced");
            }
            other => panic!("la conexion anterior deberia recibir el aviso, llego {other:?}"),
        }
    }

    #[test]
    fn cleaning_up_an_old_connection_does_not_remove_the_new_one() {
        // El fallo que esto evita: el movil cambia de red, reconecta, y cuando
        // la conexion antigua termina de morir se lleva por delante a la nueva.
        // El cliente se queda con el socket abierto y el servidor sin saber que
        // existe, de modo que nadie puede abrirle una sesion.
        let mut hub = Hub::new();
        let first = join(&mut hub, DeviceRole::Controller, "iPad");
        let first_token = hub.peers.get(&first.id()).expect("conectado").token;

        let (tx, _inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
        let second = hub.attach(first.info.clone(), tx);
        assert!(second.replaced);
        assert_ne!(second.token, first_token, "cada conexion su testigo");

        // Ahora termina de cerrarse la conexion antigua.
        let closed = hub.detach_if_current(first.id(), first_token);

        assert!(closed.is_empty(), "no tenia nada que cerrar");
        assert!(
            hub.is_connected(first.id()),
            "la conexion nueva debe seguir registrada"
        );
        assert_eq!(hub.peer_count(), 1);
    }

    #[test]
    fn a_connection_can_remove_itself_with_its_own_token() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let mut controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let token = hub.peers.get(&host.id()).expect("conectado").token;
        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("abre");

        let closed = hub.detach_if_current(host.id(), token);

        assert_eq!(closed, vec![entry.session_id]);
        assert!(!hub.is_connected(host.id()));
        assert!(matches!(
            controller.next(),
            Some(ServerToClient::PeerGone { .. })
        ));
    }

    #[test]
    fn detaching_with_a_stale_token_does_nothing() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let (tx, _inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
        let other = hub.attach(info(DeviceRole::Controller, "iPad"), tx);

        // El testigo de otro dispositivo no sirve para retirar este.
        assert!(hub.detach_if_current(host.id(), other.token).is_empty());
        assert!(hub.is_connected(host.id()));
    }

    #[test]
    fn replacing_a_connection_frees_its_session() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        hub.open_session(controller.id(), host.id(), NOW)
            .expect("abre");

        let (tx, _inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
        hub.attach(controller.info.clone(), tx);
        assert_eq!(hub.session_count(), 0, "la sesion anterior debe cerrarse");
    }

    #[test]
    fn a_peer_that_does_not_read_is_dropped() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        let entry = hub
            .open_session(controller.id(), host.id(), NOW)
            .expect("abre");
        let session_id = entry.session_id;

        // El host no lee nada: se le llena la cola.
        let mut last = Ok(host.id());
        for _ in 0..(PEER_QUEUE_DEPTH + 8) {
            last = hub.relay(session_id, controller.id(), |_| ServerToClient::Pong);
            if last.is_err() {
                break;
            }
        }

        assert_eq!(last, Err(HubError::PeerOverloaded));
        assert!(
            !hub.is_connected(host.id()),
            "un cliente que no lee se desconecta en lugar de acumular memoria"
        );
    }

    #[test]
    fn sending_to_a_closed_channel_reports_the_peer_as_gone() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let id = host.id();
        drop(host); // se cierra el receptor

        assert_eq!(hub.send(id, ServerToClient::Pong), Err(HubError::PeerGone));
        assert!(!hub.is_connected(id));
    }

    #[test]
    fn sending_to_an_unknown_device_reports_the_peer_as_gone() {
        let mut hub = Hub::new();
        assert_eq!(
            hub.send(DeviceId::new(), ServerToClient::Pong),
            Err(HubError::PeerGone)
        );
    }

    #[test]
    fn a_busy_host_is_listed_as_busy() {
        let mut hub = Hub::new();
        let host = join_host(&mut hub, "PC");
        let controller = join(&mut hub, DeviceRole::Controller, "iPad");
        hub.open_session(controller.id(), host.id(), NOW)
            .expect("abre");
        assert!(hub.available_hosts()[0].busy);
    }

    #[test]
    fn host_names_are_sanitised_before_being_listed() {
        // El nombre lo declara el dispositivo: no se publica tal cual.
        let mut hub = Hub::new();
        let (tx, _inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
        let mut raw = info(DeviceRole::Host, "PC\n\rcon saltos");
        raw.display_name = "PC\n\rcon saltos".to_owned();
        let id = raw.id;
        hub.attach(raw, tx);
        hub.register_host(id, 1).expect("se declara");

        let listed = &hub.available_hosts()[0].display_name;
        assert!(!listed.contains('\n'), "{listed:?}");
        assert!(!listed.contains('\r'), "{listed:?}");
    }

    #[test]
    fn detaching_an_unknown_device_is_harmless() {
        let mut hub = Hub::new();
        assert!(hub.detach(DeviceId::new()).is_empty());
    }

    #[test]
    fn error_codes_and_messages_are_distinct() {
        let all = [
            HubError::HostUnavailable,
            HubError::HostBusy,
            HubError::AlreadyInSession,
            HubError::UnknownSession,
            HubError::NotAMember,
            HubError::PeerGone,
            HubError::PeerOverloaded,
            HubError::TooManySessions,
            HubError::WrongRole,
        ];
        let codes: std::collections::BTreeSet<_> = all.iter().map(|e| e.code()).collect();
        assert_eq!(codes.len(), all.len(), "los codigos deben ser distintos");
        for error in all {
            assert!(!error.message().is_empty());
        }
    }
}
