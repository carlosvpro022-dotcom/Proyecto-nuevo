//! Estado del enlace de senalizacion.
//!
//! El WebSocket con el servidor propio se cae: cambia la red del movil, el
//! portatil se suspende, el operador corta una conexion inactiva. El enlace
//! tiene que recuperarse solo, pero sin reintentar en bucle cerrado contra un
//! servidor que esta caido.
//!
//! Esta maquina de estados decide cuando reintentar y con cuanta espera,
//! reutilizando la politica de retroceso exponencial de
//! [`ReconnectPolicy`](nexus_core::ReconnectPolicy). Es logica pura: no abre
//! sockets, solo dice que hacer. Abrirlos es trabajo de la etapa 3.

use std::time::Duration;

use nexus_core::ReconnectPolicy;

use crate::error::{TransportError, TransportResult};

/// Estado del enlace con el servidor de senalizacion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// Sin conexion y sin intentarlo.
    Idle,
    /// Abriendo el WebSocket.
    Connecting,
    /// Conectado pero sin autenticar.
    Connected,
    /// Autenticado: listo para senalizar.
    Ready,
    /// Esperando antes del siguiente intento.
    Backoff,
    /// Cerrado definitivamente.
    Closed,
}

impl LinkState {
    /// Nombre estable del estado.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Ready => "ready",
            Self::Backoff => "backoff",
            Self::Closed => "closed",
        }
    }

    /// Indica si se pueden enviar mensajes de senalizacion.
    #[must_use]
    pub const fn can_signal(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Hecho que hace avanzar el enlace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEvent {
    /// Se pide abrir el enlace.
    ConnectRequested,
    /// El WebSocket quedo abierto.
    SocketOpened,
    /// La autenticacion con el servidor salio bien.
    Authenticated,
    /// El enlace se perdio.
    SocketClosed,
    /// Toca reintentar tras la espera.
    RetryDue,
    /// Cierre definitivo pedido localmente.
    CloseRequested,
}

impl LinkEvent {
    /// Nombre estable del evento.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ConnectRequested => "connect_requested",
            Self::SocketOpened => "socket_opened",
            Self::Authenticated => "authenticated",
            Self::SocketClosed => "socket_closed",
            Self::RetryDue => "retry_due",
            Self::CloseRequested => "close_requested",
        }
    }
}

/// Enlace de senalizacion con su politica de reconexion.
#[derive(Debug, Clone)]
pub struct SignalingLink {
    state: LinkState,
    policy: ReconnectPolicy,
    attempts: u32,
}

impl SignalingLink {
    /// Crea el enlace con la politica por omision.
    #[must_use]
    pub fn new() -> Self {
        Self::with_policy(ReconnectPolicy::default())
    }

    /// Crea el enlace con una politica concreta.
    #[must_use]
    pub const fn with_policy(policy: ReconnectPolicy) -> Self {
        Self {
            state: LinkState::Idle,
            policy,
            attempts: 0,
        }
    }

    /// Estado actual.
    #[must_use]
    pub const fn state(&self) -> LinkState {
        self.state
    }

    /// Intentos de reconexion consumidos.
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Espera que corresponde al proximo intento.
    #[must_use]
    pub fn current_backoff(&self) -> Duration {
        self.policy.delay_for_attempt(self.attempts)
    }

    /// Indica si quedan reintentos.
    #[must_use]
    pub const fn can_retry(&self) -> bool {
        self.attempts < self.policy.max_attempts
    }

    /// Aplica un evento al enlace.
    ///
    /// Al perderse el socket, el enlace pasa a `Backoff` si quedan reintentos y
    /// a `Closed` si se agotaron, informando en ese caso con
    /// [`TransportError::ReconnectExhausted`] para que la interfaz pueda
    /// explicar por que dejo de intentarlo.
    ///
    /// # Errors
    ///
    /// - [`TransportError::InvalidLinkState`] si el evento no es legitimo en el
    ///   estado actual.
    /// - [`TransportError::ReconnectExhausted`] cuando se agotan los intentos.
    pub fn apply(&mut self, event: LinkEvent) -> TransportResult<LinkState> {
        use LinkEvent as E;
        use LinkState as S;

        if event == E::CloseRequested {
            self.state = S::Closed;
            return Ok(self.state);
        }

        if self.state == S::Closed {
            return Err(TransportError::InvalidLinkState {
                state: self.state.name(),
            });
        }

        self.state = match (self.state, event) {
            (S::Idle | S::Backoff, E::ConnectRequested | E::RetryDue) => S::Connecting,
            (S::Connecting, E::SocketOpened) => S::Connected,
            (S::Connected, E::Authenticated) => {
                // Una sesion establecida devuelve el presupuesto completo de
                // reintentos: la siguiente caida empieza con la espera minima.
                self.attempts = 0;
                S::Ready
            }
            (S::Connecting | S::Connected | S::Ready, E::SocketClosed) => {
                self.attempts = self.attempts.saturating_add(1);
                if self.can_retry() {
                    S::Backoff
                } else {
                    self.state = S::Closed;
                    return Err(TransportError::ReconnectExhausted {
                        attempts: self.attempts,
                    });
                }
            }
            (state, _) => {
                return Err(TransportError::InvalidLinkState {
                    state: state.name(),
                });
            }
        };

        Ok(self.state)
    }
}

impl Default for SignalingLink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_attempts: u32) -> ReconnectPolicy {
        ReconnectPolicy {
            max_attempts,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }

    fn drive(link: &mut SignalingLink, events: &[LinkEvent]) {
        for event in events {
            link.apply(*event)
                .unwrap_or_else(|e| panic!("{} deberia valer: {e}", event.name()));
        }
    }

    #[test]
    fn the_happy_path_reaches_ready() {
        let mut link = SignalingLink::new();
        assert_eq!(link.state(), LinkState::Idle);
        assert!(!link.state().can_signal());

        drive(
            &mut link,
            &[
                LinkEvent::ConnectRequested,
                LinkEvent::SocketOpened,
                LinkEvent::Authenticated,
            ],
        );
        assert_eq!(link.state(), LinkState::Ready);
        assert!(link.state().can_signal());
    }

    #[test]
    fn signalling_before_authentication_is_not_allowed() {
        let mut link = SignalingLink::new();
        drive(
            &mut link,
            &[LinkEvent::ConnectRequested, LinkEvent::SocketOpened],
        );
        assert_eq!(link.state(), LinkState::Connected);
        assert!(!link.state().can_signal(), "aun no esta autenticado");
    }

    #[test]
    fn authentication_without_a_socket_is_rejected() {
        let mut link = SignalingLink::new();
        let err = link.apply(LinkEvent::Authenticated).unwrap_err();
        assert_eq!(err, TransportError::InvalidLinkState { state: "idle" });
    }

    #[test]
    fn losing_the_socket_moves_to_backoff() {
        let mut link = SignalingLink::with_policy(policy(5));
        drive(
            &mut link,
            &[
                LinkEvent::ConnectRequested,
                LinkEvent::SocketOpened,
                LinkEvent::Authenticated,
                LinkEvent::SocketClosed,
            ],
        );
        assert_eq!(link.state(), LinkState::Backoff);
        assert_eq!(link.attempts(), 1);
        assert_eq!(link.current_backoff(), Duration::from_millis(500));
    }

    #[test]
    fn the_backoff_grows_with_each_failure() {
        let mut link = SignalingLink::with_policy(policy(10));
        drive(&mut link, &[LinkEvent::ConnectRequested]);

        let mut delays = Vec::new();
        for _ in 0..4 {
            link.apply(LinkEvent::SocketClosed).expect("cae");
            delays.push(link.current_backoff());
            link.apply(LinkEvent::RetryDue).expect("reintenta");
        }
        assert_eq!(
            delays,
            vec![
                Duration::from_millis(500),
                Duration::from_millis(1_000),
                Duration::from_millis(2_000),
                Duration::from_millis(4_000),
            ]
        );
    }

    #[test]
    fn the_backoff_is_capped() {
        let mut link = SignalingLink::with_policy(policy(20));
        drive(&mut link, &[LinkEvent::ConnectRequested]);
        for _ in 0..12 {
            link.apply(LinkEvent::SocketClosed).expect("cae");
            link.apply(LinkEvent::RetryDue).expect("reintenta");
        }
        assert_eq!(link.current_backoff(), Duration::from_secs(8));
    }

    #[test]
    fn a_successful_session_resets_the_budget() {
        let mut link = SignalingLink::with_policy(policy(5));
        drive(&mut link, &[LinkEvent::ConnectRequested]);
        link.apply(LinkEvent::SocketClosed).expect("cae");
        link.apply(LinkEvent::SocketClosed).ok();
        assert!(link.attempts() > 0);

        drive(
            &mut link,
            &[
                LinkEvent::RetryDue,
                LinkEvent::SocketOpened,
                LinkEvent::Authenticated,
            ],
        );
        assert_eq!(
            link.attempts(),
            0,
            "una sesion correcta devuelve el presupuesto"
        );
    }

    #[test]
    fn exhausting_the_retries_closes_the_link_and_says_why() {
        let mut link = SignalingLink::with_policy(policy(2));
        drive(&mut link, &[LinkEvent::ConnectRequested]);

        link.apply(LinkEvent::SocketClosed).expect("primer fallo");
        link.apply(LinkEvent::RetryDue).expect("reintenta");

        let err = link.apply(LinkEvent::SocketClosed).unwrap_err();
        assert_eq!(err, TransportError::ReconnectExhausted { attempts: 2 });
        assert_eq!(link.state(), LinkState::Closed);
    }

    #[test]
    fn a_closed_link_is_terminal() {
        let mut link = SignalingLink::new();
        link.apply(LinkEvent::CloseRequested).expect("cierra");
        assert_eq!(link.state(), LinkState::Closed);

        for event in [
            LinkEvent::ConnectRequested,
            LinkEvent::SocketOpened,
            LinkEvent::Authenticated,
        ] {
            assert!(
                link.apply(event).is_err(),
                "{} no debe reabrir",
                event.name()
            );
        }
    }

    #[test]
    fn closing_is_accepted_from_any_state() {
        for prefix in [
            vec![],
            vec![LinkEvent::ConnectRequested],
            vec![LinkEvent::ConnectRequested, LinkEvent::SocketOpened],
            vec![
                LinkEvent::ConnectRequested,
                LinkEvent::SocketOpened,
                LinkEvent::Authenticated,
            ],
        ] {
            let mut link = SignalingLink::new();
            drive(&mut link, &prefix);
            link.apply(LinkEvent::CloseRequested)
                .expect("cerrar siempre vale");
            assert_eq!(link.state(), LinkState::Closed);
        }
    }

    #[test]
    fn a_rejected_event_leaves_the_state_untouched() {
        let mut link = SignalingLink::new();
        drive(&mut link, &[LinkEvent::ConnectRequested]);
        assert!(link.apply(LinkEvent::Authenticated).is_err());
        assert_eq!(link.state(), LinkState::Connecting);
    }
}
