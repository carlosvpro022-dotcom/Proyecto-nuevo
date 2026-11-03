//! Maquina de estados de una sesion remota.
//!
//! Host y controlador comparten esta maquina para que ambos extremos coincidan
//! en que es una transicion legitima. Centralizar aqui las reglas evita el
//! clasico fallo de "el cliente cree que esta conectado y el host ya cerro".
//!
//! El estado `Closed` es terminal: una sesion cerrada no se reabre, se crea una
//! nueva. Esto hace imposible resucitar una sesion cuyo dispositivo acaba de
//! ser revocado.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::ids::SessionId;
use crate::time::Timestamp;

/// Motivo por el que una sesion termino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// La persona cerro la sesion desde el cliente o el host.
    UserRequested,
    /// El otro extremo cerro limpiamente.
    PeerClosed,
    /// La autenticacion mutua no se completo.
    AuthenticationFailed,
    /// Se perdio el transporte y se agotaron los reintentos.
    NetworkLost,
    /// El dispositivo fue revocado durante la sesion.
    DeviceRevoked,
    /// El host decidio cortar (por ejemplo, otra sesion con prioridad).
    HostPolicy,
    /// Error interno no recuperable.
    InternalError,
}

impl CloseReason {
    /// Nombre estable para el registro de sesiones.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::UserRequested => "user_requested",
            Self::PeerClosed => "peer_closed",
            Self::AuthenticationFailed => "authentication_failed",
            Self::NetworkLost => "network_lost",
            Self::DeviceRevoked => "device_revoked",
            Self::HostPolicy => "host_policy",
            Self::InternalError => "internal_error",
        }
    }

    /// Indica si merece aparecer como incidencia en la pantalla de seguridad.
    #[must_use]
    pub const fn is_security_relevant(self) -> bool {
        matches!(
            self,
            Self::AuthenticationFailed | Self::DeviceRevoked | Self::HostPolicy
        )
    }
}

/// Estado observable de la sesion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "reason")]
pub enum SessionState {
    /// Creada, sin empezar.
    Idle,
    /// Intercambiando pruebas de identidad con el otro extremo.
    Authenticating,
    /// Identidad verificada; aun sin medios.
    Authorized,
    /// Intercambiando descripciones y candidatos ICE.
    Negotiating,
    /// Video y control en marcha.
    Connected,
    /// El transporte cayo y se esta reintentando.
    Reconnecting,
    /// Terminada. Estado terminal.
    Closed(CloseReason),
}

impl SessionState {
    /// Nombre estable del estado.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Authenticating => "authenticating",
            Self::Authorized => "authorized",
            Self::Negotiating => "negotiating",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Closed(_) => "closed",
        }
    }

    /// Indica si el estado es terminal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Closed(_))
    }

    /// Indica si el canal de control puede transportar ordenes ahora mismo.
    #[must_use]
    pub const fn accepts_control(self) -> bool {
        matches!(self, Self::Connected)
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(reason) => write!(f, "closed({})", reason.name()),
            other => write!(f, "{}", other.name()),
        }
    }
}

/// Hecho que hace avanzar la sesion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event", content = "reason")]
pub enum SessionEvent {
    /// Empieza el handshake de identidad.
    AuthenticationStarted,
    /// El otro extremo demostro su identidad.
    AuthenticationSucceeded,
    /// El handshake fallo.
    AuthenticationFailed,
    /// Empieza el intercambio de oferta, respuesta y candidatos.
    NegotiationStarted,
    /// Los medios quedaron establecidos.
    MediaConnected,
    /// Se perdio el transporte.
    TransportLost,
    /// Se agotaron los reintentos de reconexion.
    ReconnectExhausted,
    /// Cierre solicitado localmente.
    CloseRequested,
    /// El otro extremo cerro.
    PeerClosed,
    /// El dispositivo perdio la autorizacion.
    DeviceRevoked,
    /// Fallo interno irrecuperable.
    InternalFailure,
}

impl SessionEvent {
    /// Nombre estable del evento.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::AuthenticationStarted => "authentication_started",
            Self::AuthenticationSucceeded => "authentication_succeeded",
            Self::AuthenticationFailed => "authentication_failed",
            Self::NegotiationStarted => "negotiation_started",
            Self::MediaConnected => "media_connected",
            Self::TransportLost => "transport_lost",
            Self::ReconnectExhausted => "reconnect_exhausted",
            Self::CloseRequested => "close_requested",
            Self::PeerClosed => "peer_closed",
            Self::DeviceRevoked => "device_revoked",
            Self::InternalFailure => "internal_failure",
        }
    }

    /// Eventos de cierre que son validos desde cualquier estado no terminal.
    const fn as_unconditional_close(self) -> Option<CloseReason> {
        match self {
            Self::CloseRequested => Some(CloseReason::UserRequested),
            Self::PeerClosed => Some(CloseReason::PeerClosed),
            Self::DeviceRevoked => Some(CloseReason::DeviceRevoked),
            Self::InternalFailure => Some(CloseReason::InternalError),
            _ => None,
        }
    }
}

/// Politica de reintentos de reconexion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    /// Numero maximo de intentos antes de cerrar.
    pub max_attempts: u32,
    /// Espera del primer intento.
    pub base_delay: Duration,
    /// Tope de la espera.
    pub max_delay: Duration,
}

impl ReconnectPolicy {
    /// Espera correspondiente a un intento, con retroceso exponencial acotado.
    ///
    /// El intento 1 espera `base_delay`, el 2 el doble, y asi hasta `max_delay`.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        let shift = (attempt - 1).min(16);
        let millis = self.base_delay.as_millis().saturating_mul(1u128 << shift);
        Duration::from_millis(millis.min(self.max_delay.as_millis()) as u64)
    }
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 6,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(15),
        }
    }
}

/// Sesion remota con su historial de estados.
#[derive(Debug, Clone)]
pub struct Session {
    id: SessionId,
    state: SessionState,
    policy: ReconnectPolicy,
    reconnect_attempts: u32,
    created_at: Timestamp,
    last_change_at: Timestamp,
    connected_at: Option<Timestamp>,
}

impl Session {
    /// Crea una sesion en `Idle` con la politica de reconexion por omision.
    #[must_use]
    pub fn new(id: SessionId, now: Timestamp) -> Self {
        Self::with_policy(id, now, ReconnectPolicy::default())
    }

    /// Crea una sesion con una politica de reconexion concreta.
    #[must_use]
    pub const fn with_policy(id: SessionId, now: Timestamp, policy: ReconnectPolicy) -> Self {
        Self {
            id,
            state: SessionState::Idle,
            policy,
            reconnect_attempts: 0,
            created_at: now,
            last_change_at: now,
            connected_at: None,
        }
    }

    /// Identificador de la sesion.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// Estado actual.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Intentos de reconexion consumidos.
    #[must_use]
    pub const fn reconnect_attempts(&self) -> u32 {
        self.reconnect_attempts
    }

    /// Instante de creacion.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Instante del ultimo cambio de estado.
    #[must_use]
    pub const fn last_change_at(&self) -> Timestamp {
        self.last_change_at
    }

    /// Duracion de la sesion desde que los medios se establecieron por primera vez.
    #[must_use]
    pub fn duration_since_connected(&self, now: Timestamp) -> Option<Duration> {
        self.connected_at.map(|t| now.abs_diff(t))
    }

    /// Espera que corresponde al proximo intento de reconexion.
    #[must_use]
    pub fn next_reconnect_delay(&self) -> Duration {
        self.policy.delay_for_attempt(self.reconnect_attempts)
    }

    /// Indica si quedan reintentos disponibles.
    #[must_use]
    pub const fn can_retry(&self) -> bool {
        self.reconnect_attempts < self.policy.max_attempts
    }

    /// Aplica un evento y devuelve el estado resultante.
    ///
    /// # Errors
    ///
    /// Devuelve [`CoreError::InvalidTransition`] si el evento no es legitimo en
    /// el estado actual, incluida cualquier transicion desde `Closed`.
    // La tabla de transiciones se lee como una tabla: fusionar brazos que
    // coinciden en destino pero no en significado la haria menos auditable.
    #[allow(clippy::match_same_arms)]
    pub fn apply(&mut self, event: SessionEvent, now: Timestamp) -> CoreResult<SessionState> {
        use SessionEvent as E;
        use SessionState as S;

        if self.state.is_terminal() {
            return Err(CoreError::InvalidTransition {
                from: self.state.name(),
                event: event.name(),
            });
        }

        if let Some(reason) = event.as_unconditional_close() {
            return Ok(self.transition_to(S::Closed(reason), now));
        }

        let next = match (self.state, event) {
            (S::Idle, E::AuthenticationStarted) => S::Authenticating,
            (S::Authenticating, E::AuthenticationSucceeded) => S::Authorized,
            (S::Authenticating, E::AuthenticationFailed) => {
                S::Closed(CloseReason::AuthenticationFailed)
            }
            (S::Authorized, E::NegotiationStarted) => S::Negotiating,
            (S::Negotiating, E::MediaConnected) => S::Connected,
            (S::Negotiating | S::Connected, E::TransportLost) => S::Reconnecting,
            (S::Reconnecting, E::MediaConnected) => S::Connected,
            (S::Reconnecting, E::TransportLost) => S::Reconnecting,
            (S::Reconnecting, E::ReconnectExhausted) => S::Closed(CloseReason::NetworkLost),
            (from, event) => {
                return Err(CoreError::InvalidTransition {
                    from: from.name(),
                    event: event.name(),
                });
            }
        };

        // El contador de reintentos sube al entrar en reconexion y se reinicia
        // cuando los medios vuelven: una caida posterior empieza de cero.
        match (self.state, next) {
            (_, S::Reconnecting) => {
                self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
            }
            (S::Reconnecting, S::Connected) => self.reconnect_attempts = 0,
            _ => {}
        }

        if next == S::Connected && self.connected_at.is_none() {
            self.connected_at = Some(now);
        }

        Ok(self.transition_to(next, now))
    }

    fn transition_to(&mut self, next: SessionState, now: Timestamp) -> SessionState {
        self.state = next;
        self.last_change_at = now;
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new(SessionId::new(), Timestamp(1_000))
    }

    fn drive(session: &mut Session, events: &[SessionEvent]) {
        for (step, event) in events.iter().enumerate() {
            session
                .apply(*event, Timestamp(2_000 + step as u64))
                .unwrap_or_else(|e| panic!("paso {step} ({}) deberia valer: {e}", event.name()));
        }
    }

    #[test]
    fn happy_path_reaches_connected() {
        let mut s = session();
        assert_eq!(s.state(), SessionState::Idle);
        drive(
            &mut s,
            &[
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationSucceeded,
                SessionEvent::NegotiationStarted,
                SessionEvent::MediaConnected,
            ],
        );
        assert_eq!(s.state(), SessionState::Connected);
        assert!(s.state().accepts_control());
    }

    #[test]
    fn control_is_refused_outside_connected() {
        let mut s = session();
        assert!(!s.state().accepts_control());
        drive(&mut s, &[SessionEvent::AuthenticationStarted]);
        assert!(!s.state().accepts_control());
    }

    #[test]
    fn negotiation_cannot_start_before_authentication() {
        let mut s = session();
        let err = s
            .apply(SessionEvent::NegotiationStarted, Timestamp(2_000))
            .unwrap_err();
        assert_eq!(
            err,
            CoreError::InvalidTransition {
                from: "idle",
                event: "negotiation_started"
            }
        );
        assert_eq!(
            s.state(),
            SessionState::Idle,
            "un evento rechazado no cambia el estado"
        );
    }

    #[test]
    fn media_cannot_connect_without_negotiation() {
        let mut s = session();
        drive(
            &mut s,
            &[
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationSucceeded,
            ],
        );
        assert!(
            s.apply(SessionEvent::MediaConnected, Timestamp(3_000))
                .is_err()
        );
    }

    #[test]
    fn failed_authentication_closes_the_session() {
        let mut s = session();
        drive(
            &mut s,
            &[
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationFailed,
            ],
        );
        assert_eq!(
            s.state(),
            SessionState::Closed(CloseReason::AuthenticationFailed)
        );
        assert!(s.state().is_terminal());
    }

    #[test]
    fn closed_sessions_are_terminal() {
        let mut s = session();
        drive(&mut s, &[SessionEvent::CloseRequested]);
        for event in [
            SessionEvent::AuthenticationStarted,
            SessionEvent::MediaConnected,
            SessionEvent::CloseRequested,
        ] {
            assert!(
                s.apply(event, Timestamp(9_000)).is_err(),
                "{} no debe reabrir una sesion cerrada",
                event.name()
            );
        }
    }

    #[test]
    fn revocation_closes_from_any_live_state() {
        for prefix in [
            vec![],
            vec![SessionEvent::AuthenticationStarted],
            vec![
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationSucceeded,
                SessionEvent::NegotiationStarted,
                SessionEvent::MediaConnected,
            ],
        ] {
            let mut s = session();
            drive(&mut s, &prefix);
            s.apply(SessionEvent::DeviceRevoked, Timestamp(5_000))
                .expect("revocar siempre vale");
            assert_eq!(s.state(), SessionState::Closed(CloseReason::DeviceRevoked));
        }
    }

    #[test]
    fn transport_loss_moves_to_reconnecting_and_counts_attempts() {
        let mut s = session();
        drive(
            &mut s,
            &[
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationSucceeded,
                SessionEvent::NegotiationStarted,
                SessionEvent::MediaConnected,
                SessionEvent::TransportLost,
            ],
        );
        assert_eq!(s.state(), SessionState::Reconnecting);
        assert_eq!(s.reconnect_attempts(), 1);

        drive(
            &mut s,
            &[SessionEvent::TransportLost, SessionEvent::TransportLost],
        );
        assert_eq!(s.reconnect_attempts(), 3);
    }

    #[test]
    fn successful_reconnect_resets_the_counter() {
        let mut s = session();
        drive(
            &mut s,
            &[
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationSucceeded,
                SessionEvent::NegotiationStarted,
                SessionEvent::MediaConnected,
                SessionEvent::TransportLost,
                SessionEvent::TransportLost,
                SessionEvent::MediaConnected,
            ],
        );
        assert_eq!(s.state(), SessionState::Connected);
        assert_eq!(s.reconnect_attempts(), 0);
    }

    #[test]
    fn exhausted_reconnects_close_with_network_lost() {
        let mut s = session();
        drive(
            &mut s,
            &[
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationSucceeded,
                SessionEvent::NegotiationStarted,
                SessionEvent::MediaConnected,
                SessionEvent::TransportLost,
                SessionEvent::ReconnectExhausted,
            ],
        );
        assert_eq!(s.state(), SessionState::Closed(CloseReason::NetworkLost));
    }

    #[test]
    fn retry_budget_is_enforced() {
        let policy = ReconnectPolicy {
            max_attempts: 2,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
        };
        let mut s = Session::with_policy(SessionId::new(), Timestamp(0), policy);
        drive(
            &mut s,
            &[
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationSucceeded,
                SessionEvent::NegotiationStarted,
                SessionEvent::MediaConnected,
                SessionEvent::TransportLost,
            ],
        );
        assert!(s.can_retry());
        drive(&mut s, &[SessionEvent::TransportLost]);
        assert!(!s.can_retry(), "con 2 intentos consumidos ya no quedan");
    }

    // Toda la progresion se expresa en milisegundos para que el doblado
    // (500, 1000, 2000, 4000) se vea de un golpe.
    #[test]
    fn backoff_grows_and_is_capped() {
        let policy = ReconnectPolicy {
            max_attempts: 10,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(4),
        };
        assert_eq!(policy.delay_for_attempt(0), Duration::ZERO);
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(500));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(1_000));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(2_000));
        assert_eq!(policy.delay_for_attempt(4), Duration::from_millis(4_000));
        assert_eq!(
            policy.delay_for_attempt(50),
            Duration::from_secs(4),
            "debe quedar acotado"
        );
    }

    #[test]
    fn connected_duration_is_measured_from_first_connection() {
        let mut s = session();
        assert!(s.duration_since_connected(Timestamp(5_000)).is_none());
        drive(
            &mut s,
            &[
                SessionEvent::AuthenticationStarted,
                SessionEvent::AuthenticationSucceeded,
                SessionEvent::NegotiationStarted,
            ],
        );
        s.apply(SessionEvent::MediaConnected, Timestamp(10_000))
            .expect("conecta");
        assert_eq!(
            s.duration_since_connected(Timestamp(13_500)),
            Some(Duration::from_millis(3_500))
        );
    }

    #[test]
    fn state_display_includes_close_reason() {
        assert_eq!(SessionState::Connected.to_string(), "connected");
        assert_eq!(
            SessionState::Closed(CloseReason::DeviceRevoked).to_string(),
            "closed(device_revoked)"
        );
    }

    #[test]
    fn security_relevant_reasons_are_flagged() {
        assert!(CloseReason::AuthenticationFailed.is_security_relevant());
        assert!(CloseReason::DeviceRevoked.is_security_relevant());
        assert!(!CloseReason::UserRequested.is_security_relevant());
    }
}
