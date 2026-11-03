//! El canal de senalizacion: `GET /v1/signaling`.
//!
//! # Lo que hace el servidor
//!
//! Autentica el dispositivo, lo pone en contacto con el otro extremo y reenvia
//! la negociacion WebRTC. Nada mas.
//!
//! # Lo que no hace, a proposito
//!
//! - **No concede permisos.** Los decide el host y llegan en
//!   `SessionAccepted`.
//! - **No lee la sesion.** La pantalla, el teclado y los archivos van por el
//!   canal de datos de WebRTC, cifrados extremo a extremo con DTLS.
//! - **No puede colocarse en medio sin que se note.** Reenvia el SDP, que lleva
//!   la huella del certificado DTLS, y podria sustituirla. Lo que lo impide es
//!   que los dos extremos firman despues las dos huellas y comprueban la del
//!   otro (`nexus_security::binding`): si el servidor sustituye una, la sesion
//!   se aborta. El servidor reenvia esas firmas sin poder forjarlas.
//!
//! # Estructura
//!
//! La maquina de estados del protocolo ([`Connection`]) es **sincrona** y no
//! conoce el socket: recibe texto y deja mensajes en una cola. Eso permite
//! probar el protocolo entero sin red, y deja el bucle asincrono reducido a
//! mover bytes.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use nexus_core::device::DeviceRole;
use nexus_core::protocol::Envelope;
use nexus_core::protocol::signaling::{AuthRejection, ClientToServer, ServerToClient};
use nexus_core::{CapabilitySet, DeviceId, DeviceInfo, PermissionSet, SessionId, Timestamp};
use nexus_security::registry::{AuthRequest, DeviceRegistry};
use nexus_security::{AuthRole, DevicePublicKey, Nonce, SecurityError};
use nexus_transport::codec::{decode_client_message, encode_server_message};
use tokio::sync::mpsc;

use crate::audit::{AuditEvent, AuthResult};
use crate::enrollment::{Admission, Refusal};
use crate::hub::{ConnectionToken, HubError, PEER_QUEUE_DEPTH};
use crate::state::AppState;

/// Tiempo sin recibir nada tras el cual se cierra la conexion.
///
/// El cliente manda `Ping` de cortesia mucho antes. Un valor generoso evita
/// cortar a un movil con mala cobertura; que exista evita acumular conexiones
/// muertas que retengan un host.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Longitud maxima del texto con que un host rechaza una sesion.
pub const MAX_REASON_LEN: usize = 160;

/// Que hacer con la conexion tras procesar un mensaje.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Seguir atendiendola.
    Continue,
    /// Cerrarla, con el motivo que se anota en el registro.
    Close(&'static str),
}

/// Punto del protocolo en que esta la conexion.
#[derive(Debug)]
enum Phase {
    /// Esperando el `Hello`.
    Greeting,
    /// Reto emitido, esperando la firma.
    Challenged {
        info: Box<DeviceInfo>,
        capabilities: CapabilitySet,
        server_nonce: Nonce,
        /// Si la identidad se dio de alta en este `Hello` y habria que
        /// olvidarla si la firma no llega a cuadrar.
        provisional: bool,
    },
    /// Autenticado.
    Ready {
        device_id: DeviceId,
        info: Box<DeviceInfo>,
        capabilities: CapabilitySet,
        /// Identifica esta conexion concreta dentro del centro de reparto.
        token: ConnectionToken,
    },
}

/// Una conexion de senalizacion y su punto del protocolo.
#[derive(Debug)]
pub struct Connection {
    state: Arc<AppState>,
    remote: String,
    outbound: mpsc::Sender<ServerToClient>,
    phase: Phase,
}

impl Connection {
    /// Crea la conexion y anota su apertura.
    #[must_use]
    pub fn new(
        state: Arc<AppState>,
        remote: String,
        outbound: mpsc::Sender<ServerToClient>,
        now: Timestamp,
    ) -> Self {
        state.shared().audit.record(
            now,
            AuditEvent::ConnectionOpened {
                remote: remote.clone(),
            },
        );
        Self {
            state,
            remote,
            outbound,
            phase: Phase::Greeting,
        }
    }

    /// Dispositivo autenticado en esta conexion, si lo hay.
    #[must_use]
    pub const fn device_id(&self) -> Option<DeviceId> {
        match &self.phase {
            Phase::Ready { device_id, .. } => Some(*device_id),
            _ => None,
        }
    }

    /// Testigo de esta conexion, una vez autenticada.
    const fn token(&self) -> Option<ConnectionToken> {
        match &self.phase {
            Phase::Ready { token, .. } => Some(*token),
            _ => None,
        }
    }

    /// Procesa un mensaje de texto recibido del cliente.
    pub fn handle_text(&mut self, raw: &str, now: Timestamp) -> Flow {
        let Ok(envelope) = decode_client_message(raw) else {
            // No se devuelve el detalle: el texto venia del cliente y no debe
            // reaparecer en su pantalla ni en el registro.
            return self.fail("bad_request", "el mensaje no cumple el protocolo");
        };

        let sent_at = envelope.ts;
        match envelope.payload {
            ClientToServer::Ping => self.reply(ServerToClient::Pong),
            ClientToServer::Hello {
                device,
                capabilities,
                public_key_hex,
                enrollment_token,
            } => self.on_hello(
                device,
                capabilities,
                &public_key_hex,
                enrollment_token.as_deref(),
                now,
            ),
            ClientToServer::AuthProof {
                device_id,
                client_nonce_hex,
                signature_hex,
            } => self.on_auth_proof(device_id, &client_nonce_hex, signature_hex, sent_at, now),
            other => self.on_session_message(other, now),
        }
    }

    /// Cierra la conexion: la retira del centro de reparto y lo anota.
    ///
    /// Se retira **solo si sigue siendo la conexion vigente** de ese
    /// dispositivo. Si entre medias reconecto —lo normal al cambiar de red—,
    /// el registro ya es de la conexion nueva y tocarla la dejaria con el
    /// socket abierto pero invisible para el servidor.
    pub fn close(&mut self, reason: &'static str, now: Timestamp) {
        let device_id = self.device_id();
        let token = self.token();
        let mut shared = self.state.shared();
        if let (Some(id), Some(token)) = (device_id, token) {
            for session_id in shared.hub.detach_if_current(id, token) {
                shared.audit.record(
                    now,
                    AuditEvent::SessionClosed {
                        session_id,
                        reason: "se desconecto un extremo",
                    },
                );
            }
        }
        shared
            .audit
            .record(now, AuditEvent::ConnectionClosed { device_id, reason });
    }

    // --- autenticacion ---

    fn on_hello(
        &mut self,
        device: DeviceInfo,
        capabilities: CapabilitySet,
        public_key_hex: &str,
        enrollment_token: Option<&str>,
        now: Timestamp,
    ) -> Flow {
        if !matches!(self.phase, Phase::Greeting) {
            return self.fail("unexpected_message", "la presentacion ya se hizo");
        }

        let Ok(key) = DevicePublicKey::from_hex(public_key_hex) else {
            return self.reject_auth(None, None, "clave publica no utilizable", None, now);
        };
        let fingerprint = key.short_fingerprint();

        let mut shared = self.state.shared();
        let admission = shared
            .auth
            .registry_mut()
            .admit(&device, &key, enrollment_token, now);

        let admission = match admission {
            Ok(admission) => admission,
            Err(refusal) => {
                drop(shared);
                return self.reject_auth(
                    Some(device.id),
                    Some(fingerprint),
                    refusal.reason(),
                    refusal_rejection(refusal),
                    now,
                );
            }
        };

        let challenge = match shared.auth.issue_challenge(now) {
            Ok(challenge) => challenge,
            Err(error) => {
                if admission == Admission::Enrolled {
                    shared.auth.registry_mut().forget(device.id);
                }
                drop(shared);
                return self.reject_auth(
                    Some(device.id),
                    Some(fingerprint),
                    "no se pudo emitir el reto",
                    rejection_for(&error),
                    now,
                );
            }
        };
        drop(shared);

        self.phase = Phase::Challenged {
            info: Box::new(device),
            capabilities,
            server_nonce: challenge.nonce,
            provisional: admission == Admission::Enrolled,
        };

        self.reply(ServerToClient::Challenge {
            nonce_hex: challenge.nonce.to_hex(),
            server_time: now,
        })
    }

    fn on_auth_proof(
        &mut self,
        device_id: DeviceId,
        client_nonce_hex: &str,
        signature_hex: String,
        sent_at: Timestamp,
        now: Timestamp,
    ) -> Flow {
        let Phase::Challenged {
            info,
            capabilities,
            server_nonce,
            provisional,
        } = std::mem::replace(&mut self.phase, Phase::Greeting)
        else {
            return self.fail("unexpected_message", "todavia no hay reto pendiente");
        };

        let fingerprint = self
            .state
            .shared()
            .auth
            .registry()
            .get(info.id)
            .map(|record| record.public_key.short_fingerprint());

        let forget_if_provisional = |state: &AppState| {
            if provisional {
                state.shared().auth.registry_mut().forget(info.id);
            }
        };

        if device_id != info.id {
            forget_if_provisional(&self.state);
            return self.reject_auth(
                Some(info.id),
                fingerprint,
                "la firma dice ser de otro dispositivo",
                None,
                now,
            );
        }

        let Ok(client_nonce) = Nonce::from_hex(client_nonce_hex) else {
            forget_if_provisional(&self.state);
            return self.reject_auth(
                Some(info.id),
                fingerprint,
                "nonce del cliente no utilizable",
                None,
                now,
            );
        };

        let request = AuthRequest {
            device_id,
            server_nonce,
            client_nonce,
            role: auth_role(info.role),
            timestamp: sent_at,
            signature_hex,
        };

        let mut shared = self.state.shared();
        if let Err(error) = shared.auth.verify(&request, now) {
            if provisional {
                shared.auth.registry_mut().forget(info.id);
            }
            drop(shared);
            return self.reject_auth(
                Some(info.id),
                fingerprint,
                security_reason(&error),
                rejection_for(&error),
                now,
            );
        }

        let signaling_session = SessionId::new();
        let attachment = shared.hub.attach((*info).clone(), self.outbound.clone());
        shared.origins.register_success(&self.remote);
        shared.audit.record(
            now,
            AuditEvent::Authentication {
                remote: self.remote.clone(),
                device_id: Some(device_id),
                key_fingerprint: fingerprint,
                result: if provisional {
                    AuthResult::Enrolled
                } else {
                    AuthResult::Accepted
                },
            },
        );
        drop(shared);

        let ice_servers = self.state.ice_servers(&device_id.short(), now);
        self.phase = Phase::Ready {
            device_id,
            info,
            capabilities,
            token: attachment.token,
        };
        self.reply(ServerToClient::AuthAccepted {
            session_id: signaling_session,
            ice_servers,
        })
    }

    /// Rechaza la autenticacion: lo anota con detalle y contesta sin detalle.
    ///
    /// Hacia el cliente el motivo es siempre el mismo salvo cuando hay que
    /// decirle cuanto esperar. Distinguir "dispositivo desconocido" de "firma
    /// invalida" o de "revocado" confirmaria a quien prueba que un
    /// identificador existe, y el orden de comprobaciones permite preguntarlo
    /// sin tener la clave privada.
    fn reject_auth(
        &mut self,
        device_id: Option<DeviceId>,
        key_fingerprint: Option<String>,
        reason: &'static str,
        rejection: Option<(AuthRejection, Option<u32>)>,
        now: Timestamp,
    ) -> Flow {
        let mut shared = self.state.shared();
        let lockout = shared.origins.register_failure(&self.remote, now);
        shared.audit.record(
            now,
            AuditEvent::Authentication {
                remote: self.remote.clone(),
                device_id,
                key_fingerprint,
                result: AuthResult::Rejected(reason),
            },
        );
        if let Some(wait) = lockout {
            shared.audit.record(
                now,
                AuditEvent::Throttled {
                    remote: self.remote.clone(),
                    retry_after_secs: wait.as_secs(),
                },
            );
        }
        drop(shared);

        // Si este intento es el que agoto la paciencia del servidor, se le dice
        // cuanto esperar: un cliente legitimo con un reloj desviado o una
        // configuracion mal puesta necesita saber que no debe insistir. El
        // motivo concreto del fallo sigue sin salir de aqui.
        let (reason, retry_after_secs) = match lockout {
            Some(wait) => (AuthRejection::RateLimited, Some(wait.as_secs() as u32)),
            None => rejection.unwrap_or((AuthRejection::InvalidCredentials, None)),
        };
        let _ = self.reply(ServerToClient::AuthRejected {
            reason,
            retry_after_secs,
        });
        Flow::Close("autenticacion rechazada")
    }

    // --- mensajes de sesion ---

    fn on_session_message(&mut self, payload: ClientToServer, now: Timestamp) -> Flow {
        let Phase::Ready {
            device_id,
            info,
            capabilities,
            ..
        } = &self.phase
        else {
            return self.fail("not_authenticated", "hace falta autenticarse primero");
        };
        let device_id = *device_id;
        let info = info.clone();
        let capabilities = capabilities.clone();

        match payload {
            ClientToServer::RegisterHost { monitor_count } => {
                self.on_register_host(device_id, monitor_count, now)
            }
            ClientToServer::RequestSession {
                host_id,
                requested_permissions,
            } => self.on_request_session(
                device_id,
                &info,
                &capabilities,
                host_id,
                requested_permissions,
                now,
            ),
            ClientToServer::AcceptSession {
                session_id,
                negotiated_capabilities,
                effective_permissions,
            } => self.on_accept_session(
                device_id,
                session_id,
                negotiated_capabilities,
                effective_permissions,
                now,
            ),
            ClientToServer::RejectSession { session_id, reason } => {
                self.on_reject_session(device_id, session_id, &reason, now)
            }
            ClientToServer::Offer { session_id, sdp } => {
                self.forward(device_id, session_id, move |_| ServerToClient::Offer {
                    session_id,
                    sdp,
                })
            }
            ClientToServer::Answer { session_id, sdp } => {
                self.forward(device_id, session_id, move |_| ServerToClient::Answer {
                    session_id,
                    sdp,
                })
            }
            ClientToServer::IceCandidate {
                session_id,
                candidate,
                sdp_mid,
            } => self.forward(device_id, session_id, move |_| {
                ServerToClient::IceCandidate {
                    session_id,
                    candidate,
                    sdp_mid,
                }
            }),
            ClientToServer::PeerAuth {
                session_id,
                fingerprint,
                signature_hex,
            } => {
                let flow = self.forward(device_id, session_id, move |from| {
                    ServerToClient::PeerAuth {
                        session_id,
                        device_id: from,
                        fingerprint,
                        signature_hex,
                    }
                });
                self.state.shared().audit.record(
                    now,
                    AuditEvent::BindingRelayed {
                        session_id,
                        from: device_id,
                    },
                );
                flow
            }
            ClientToServer::Bye { session_id } => self.on_bye(device_id, session_id, now),
            ClientToServer::RedeemPairingCode { .. } => self.reply(ServerToClient::Error {
                code: "pairing_relay_unavailable".to_owned(),
                message: "este servidor no retransmite codigos de emparejamiento".to_owned(),
            }),
            ClientToServer::Hello { .. } | ClientToServer::AuthProof { .. } => {
                self.fail("unexpected_message", "la autenticacion ya se completo")
            }
            ClientToServer::Ping => self.reply(ServerToClient::Pong),
        }
    }

    fn on_register_host(&mut self, device_id: DeviceId, monitor_count: u8, now: Timestamp) -> Flow {
        if monitor_count == 0 {
            return self.reply(ServerToClient::Error {
                code: "no_monitors".to_owned(),
                message: "un host sin monitores no puede compartir pantalla".to_owned(),
            });
        }

        let mut shared = self.state.shared();
        match shared.hub.register_host(device_id, monitor_count) {
            Ok(()) => {
                shared.audit.record(
                    now,
                    AuditEvent::HostRegistered {
                        device_id,
                        monitor_count,
                    },
                );
                drop(shared);
                self.reply(ServerToClient::HostRegistered { host_id: device_id })
            }
            Err(error) => {
                drop(shared);
                self.hub_error(error)
            }
        }
    }

    fn on_request_session(
        &mut self,
        controller_id: DeviceId,
        info: &DeviceInfo,
        capabilities: &CapabilitySet,
        host_id: DeviceId,
        requested_permissions: PermissionSet,
        now: Timestamp,
    ) -> Flow {
        let mut shared = self.state.shared();
        let entry = match shared.hub.open_session(controller_id, host_id, now) {
            Ok(entry) => entry,
            Err(error) => {
                drop(shared);
                return self.hub_error(error);
            }
        };

        let notified = shared.hub.send(
            host_id,
            ServerToClient::SessionRequested {
                session_id: entry.session_id,
                controller: info.clone(),
                controller_capabilities: capabilities.clone(),
                requested_permissions,
            },
        );

        match notified {
            Ok(()) => {
                shared.audit.record(
                    now,
                    AuditEvent::SessionRequested {
                        session_id: entry.session_id,
                        host: host_id,
                        controller: controller_id,
                    },
                );
                Flow::Continue
            }
            Err(error) => {
                // El host se cayo entre abrir la sesion y avisarle.
                let _ = shared.hub.close_session(entry.session_id, controller_id);
                drop(shared);
                self.hub_error(error)
            }
        }
    }

    fn on_accept_session(
        &mut self,
        host_id: DeviceId,
        session_id: SessionId,
        negotiated_capabilities: CapabilitySet,
        effective_permissions: PermissionSet,
        now: Timestamp,
    ) -> Flow {
        let mut shared = self.state.shared();
        if let Err(error) = shared.hub.accept_session(session_id, host_id) {
            drop(shared);
            return self.hub_error(error);
        }

        let relayed = shared
            .hub
            .relay(session_id, host_id, |_| ServerToClient::SessionAccepted {
                session_id,
                negotiated_capabilities,
                effective_permissions,
            });
        shared.audit.record(
            now,
            AuditEvent::SessionResolved {
                session_id,
                accepted: true,
            },
        );
        drop(shared);

        match relayed {
            Ok(_) => Flow::Continue,
            Err(error) => self.hub_error(error),
        }
    }

    fn on_reject_session(
        &mut self,
        host_id: DeviceId,
        session_id: SessionId,
        reason: &str,
        now: Timestamp,
    ) -> Flow {
        // El texto lo escribe el otro extremo y acaba en una pantalla y en un
        // registro: se limpia antes de reenviarlo.
        let reason = nexus_core::text::sanitize_line(reason, MAX_REASON_LEN, "sin motivo");

        let mut shared = self.state.shared();
        let session = match shared.hub.session(session_id) {
            Some(entry) if entry.host == host_id => entry.clone(),
            Some(_) => {
                drop(shared);
                return self.hub_error(HubError::NotAMember);
            }
            None => {
                drop(shared);
                return self.hub_error(HubError::UnknownSession);
            }
        };

        let relayed = shared.hub.send(
            session.controller,
            ServerToClient::SessionRejected { session_id, reason },
        );
        let _ = shared.hub.close_session(session_id, host_id);
        shared.audit.record(
            now,
            AuditEvent::SessionResolved {
                session_id,
                accepted: false,
            },
        );
        drop(shared);

        match relayed {
            Ok(()) => Flow::Continue,
            Err(error) => self.hub_error(error),
        }
    }

    fn on_bye(&mut self, device_id: DeviceId, session_id: SessionId, now: Timestamp) -> Flow {
        let mut shared = self.state.shared();
        match shared.hub.close_session(session_id, device_id) {
            Ok(entry) => {
                if let Some(other) = entry.counterpart(device_id) {
                    let _ = shared
                        .hub
                        .send(other, ServerToClient::PeerGone { session_id });
                }
                shared.audit.record(
                    now,
                    AuditEvent::SessionClosed {
                        session_id,
                        reason: "cierre ordenado",
                    },
                );
                Flow::Continue
            }
            Err(error) => {
                drop(shared);
                self.hub_error(error)
            }
        }
    }

    fn forward<F>(&mut self, from: DeviceId, session_id: SessionId, build: F) -> Flow
    where
        F: FnOnce(DeviceId) -> ServerToClient,
    {
        let result = self.state.shared().hub.relay(session_id, from, build);
        match result {
            Ok(_) => Flow::Continue,
            Err(error) => self.hub_error(error),
        }
    }

    // --- utilidades ---

    fn hub_error(&mut self, error: HubError) -> Flow {
        self.reply(ServerToClient::Error {
            code: error.code().to_owned(),
            message: error.message().to_owned(),
        })
    }

    /// Encola un mensaje para el cliente.
    fn reply(&mut self, message: ServerToClient) -> Flow {
        match self.outbound.try_send(message) {
            Ok(()) => Flow::Continue,
            Err(_) => Flow::Close("no se pudo encolar la respuesta"),
        }
    }

    /// Contesta un error de protocolo y cierra.
    fn fail(&mut self, code: &str, message: &str) -> Flow {
        let _ = self.reply(ServerToClient::Error {
            code: code.to_owned(),
            message: message.to_owned(),
        });
        Flow::Close(match self.phase {
            Phase::Greeting => "protocolo incorrecto sin autenticar",
            Phase::Challenged { .. } => "protocolo incorrecto durante el reto",
            Phase::Ready { .. } => "protocolo incorrecto ya autenticado",
        })
    }
}

const fn auth_role(role: DeviceRole) -> AuthRole {
    match role {
        DeviceRole::Host => AuthRole::Host,
        DeviceRole::Controller => AuthRole::Controller,
    }
}

/// Motivo fijo para el registro, derivado del fallo de seguridad.
const fn security_reason(error: &SecurityError) -> &'static str {
    match error {
        SecurityError::UnknownDevice => "dispositivo desconocido",
        SecurityError::DeviceRevoked => "dispositivo revocado",
        SecurityError::ChallengeNotValid => "reto caducado o ya usado",
        SecurityError::ClockSkew { .. } => "reloj desviado",
        SecurityError::ReplayDetected => "mensaje repetido",
        SecurityError::RateLimited { .. } => "demasiados intentos",
        SecurityError::InvalidSignature => "firma invalida",
        _ => "credenciales no validas",
    }
}

/// Lo unico que se le dice al cliente, cuando hay algo que decirle.
///
/// Solo el limite de intentos sale del rechazo generico, porque el cliente
/// necesita saber cuanto esperar para no insistir en bucle.
const fn rejection_for(error: &SecurityError) -> Option<(AuthRejection, Option<u32>)> {
    match error {
        SecurityError::RateLimited { retry_after_secs } => {
            Some((AuthRejection::RateLimited, Some(*retry_after_secs as u32)))
        }
        _ => None,
    }
}

const fn refusal_rejection(refusal: Refusal) -> Option<(AuthRejection, Option<u32>)> {
    match refusal {
        // Un dispositivo revocado en el servidor si recibe el motivo: aqui ya
        // ha presentado una clave que coincide con la registrada, de modo que
        // es el dispositivo de verdad y no se le revela nada que no sepa.
        Refusal::Revoked => Some((AuthRejection::Revoked, None)),
        _ => None,
    }
}

// --- capa asincrona: solo mueve bytes ---

/// Direccion del socket, si el servidor se monto con informacion de conexion.
///
/// Es un extractor propio y **infalible**. `ConnectInfo` a secas rechazaria la
/// peticion cuando no hay informacion de conexion, lo que convertiria un
/// detalle del montaje del servidor en un error 500 para el cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerAddress(pub Option<SocketAddr>);

impl<S> FromRequestParts<S> for PeerAddress
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0),
        ))
    }
}

/// Atiende `GET /v1/signaling`.
///
/// El limite de intentos se comprueba **antes** de aceptar la mejora a
/// WebSocket: a quien esta bloqueado no se le monta una conexion.
pub async fn handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    PeerAddress(address): PeerAddress,
    headers: HeaderMap,
) -> Response {
    let remote = remote_label(&state, address, &headers);
    let now = Timestamp::now();

    if let Err(error) = state.shared().origins.check(&remote, now) {
        let retry_after = match error {
            SecurityError::RateLimited { retry_after_secs } => retry_after_secs,
            _ => 60,
        };
        state.shared().audit.record(
            now,
            AuditEvent::Throttled {
                remote,
                retry_after_secs: retry_after,
            },
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", retry_after.to_string())],
            "demasiados intentos\n",
        )
            .into_response();
    }

    ws.on_upgrade(move |socket| connection_loop(socket, state, remote))
}

/// Direccion de origen con la que se identifica la conexion.
///
/// Por omision es la del socket. La cabecera `X-Forwarded-For` solo se tiene en
/// cuenta si la configuracion lo autoriza explicitamente: creerla sin mas
/// permitiria a cualquiera falsear su direccion y burlar el limite de intentos.
fn remote_label(state: &AppState, addr: Option<SocketAddr>, headers: &HeaderMap) -> String {
    if state.config.trust_proxy_header
        && let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
    {
        let cleaned = nexus_core::text::sanitize_line(forwarded, 64, "");
        if !cleaned.is_empty() {
            return cleaned;
        }
    }

    addr.map_or_else(|| "desconocido".to_owned(), |addr| addr.ip().to_string())
}

async fn connection_loop(socket: WebSocket, state: Arc<AppState>, remote: String) {
    let (mut sink, mut stream) = socket.split();
    let (outbound, mut inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
    let mut connection = Connection::new(Arc::clone(&state), remote, outbound, Timestamp::now());

    let reason = loop {
        tokio::select! {
            message = inbox.recv() => {
                let Some(message) = message else {
                    break "el servidor retiro la conexion";
                };
                if !write(&mut sink, message).await {
                    break "no se pudo escribir en el socket";
                }
            }
            frame = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => {
                let Ok(frame) = frame else {
                    break "inactividad";
                };
                match frame {
                    None | Some(Ok(Message::Close(_))) => break "el cliente cerro",
                    Some(Err(_)) => break "error de socket",
                    Some(Ok(Message::Text(text))) => {
                        if let Flow::Close(reason) =
                            connection.handle_text(&text, Timestamp::now())
                        {
                            break reason;
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        break "marco binario: el protocolo es texto";
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                }
            }
        }
    };

    // Lo que quedase encolado se entrega antes de cerrar: tipicamente es el
    // propio mensaje que explica por que se cierra.
    while let Ok(message) = inbox.try_recv() {
        if !write(&mut sink, message).await {
            break;
        }
    }

    connection.close(reason, Timestamp::now());
    let _ = sink.send(Message::Close(None)).await;
}

async fn write(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: ServerToClient,
) -> bool {
    match encode_server_message(&Envelope::new(message)) {
        Ok(raw) => sink.send(Message::Text(raw.into())).await.is_ok(),
        Err(error) => {
            tracing::error!("mensaje de salida no codificable: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, MIN_SECRET_LEN};
    use nexus_core::device::DevicePlatform;
    use nexus_core::{Capability, Permission};
    use nexus_security::{AuthTranscript, DeviceIdentity};

    const NOW: Timestamp = Timestamp(1_730_000_000_000);
    const TOKEN: &str = "testigo-de-alta-del-servidor-nexus";

    fn app_state() -> Arc<AppState> {
        let config = Config::from_lookup(|name| match name {
            "NEXUS_PUBLIC_URL" => Some("wss://nexus.example/signaling".to_owned()),
            "NEXUS_TURN_SHARED_SECRET" => Some("s".repeat(MIN_SECRET_LEN)),
            "NEXUS_TURN_URLS" => Some("turn:turn.nexus.example:3478".to_owned()),
            "NEXUS_STUN_URLS" => Some("stun:stun.nexus.example:3478".to_owned()),
            "NEXUS_HOST_ENROLLMENT_TOKEN" => Some(TOKEN.to_owned()),
            _ => None,
        })
        .expect("configuracion de prueba valida");
        Arc::new(AppState::new(config))
    }

    /// Un dispositivo de prueba con su conexion al servidor.
    struct Client {
        connection: Connection,
        inbox: mpsc::Receiver<ServerToClient>,
        identity: DeviceIdentity,
        info: DeviceInfo,
        capabilities: CapabilitySet,
    }

    impl Client {
        fn new(state: &Arc<AppState>, role: DeviceRole, name: &str, remote: &str) -> Self {
            let device_id = DeviceId::new();
            let info = DeviceInfo {
                id: device_id,
                display_name: name.to_owned(),
                platform: match role {
                    DeviceRole::Host => DevicePlatform::Windows,
                    DeviceRole::Controller => DevicePlatform::IpadOs,
                },
                role,
                os_version: "1.0".to_owned(),
                app_version: "0.1.0".to_owned(),
            };
            let mut capabilities = CapabilitySet::empty();
            capabilities.insert(Capability::ScreenStream);
            capabilities.insert(Capability::RemoteInput);

            let (outbound, inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
            Self {
                connection: Connection::new(Arc::clone(state), remote.to_owned(), outbound, NOW),
                inbox,
                identity: DeviceIdentity::generate(device_id),
                info,
                capabilities,
            }
        }

        fn id(&self) -> DeviceId {
            self.info.id
        }

        fn send_at(&mut self, payload: ClientToServer, sent_at: Timestamp, now: Timestamp) -> Flow {
            let raw = Envelope::at(sent_at, payload).encode().expect("codifica");
            self.connection.handle_text(&raw, now)
        }

        fn send(&mut self, payload: ClientToServer) -> Flow {
            self.send_at(payload, NOW, NOW)
        }

        fn next(&mut self) -> Option<ServerToClient> {
            self.inbox.try_recv().ok()
        }

        fn drain(&mut self) {
            while self.inbox.try_recv().is_ok() {}
        }

        fn hello(&mut self, token: Option<&str>) -> Flow {
            self.send(ClientToServer::Hello {
                device: self.info.clone(),
                capabilities: self.capabilities.clone(),
                public_key_hex: self.identity.public_key().to_hex(),
                enrollment_token: token.map(str::to_owned),
            })
        }

        /// Firma el reto recibido. Devuelve el mensaje listo para enviar.
        fn proof_for(&self, nonce_hex: &str, sent_at: Timestamp) -> ClientToServer {
            let server_nonce = Nonce::from_hex(nonce_hex).expect("nonce del servidor");
            let client_nonce = Nonce::generate();
            let transcript = AuthTranscript::new(
                server_nonce,
                client_nonce,
                self.info.id,
                auth_role(self.info.role),
                sent_at,
            );
            ClientToServer::AuthProof {
                device_id: self.info.id,
                client_nonce_hex: client_nonce.to_hex(),
                signature_hex: self.identity.sign_hex(&transcript.to_bytes()),
            }
        }

        /// Completa el saludo y la firma. Falla la prueba si no se autentica.
        fn authenticate(&mut self, token: Option<&str>) {
            assert_eq!(self.hello(token), Flow::Continue, "el saludo deberia valer");
            let nonce_hex = match self.next() {
                Some(ServerToClient::Challenge { nonce_hex, .. }) => nonce_hex,
                other => panic!("se esperaba un reto, llego {other:?}"),
            };
            let proof = self.proof_for(&nonce_hex, NOW);
            assert_eq!(self.send(proof), Flow::Continue);
            match self.next() {
                Some(ServerToClient::AuthAccepted { .. }) => {}
                other => panic!("se esperaba AuthAccepted, llego {other:?}"),
            }
        }
    }

    /// Host autenticado y declarado disponible.
    fn ready_host(state: &Arc<AppState>) -> Client {
        let mut host = Client::new(state, DeviceRole::Host, "PC del salon", "192.0.2.10");
        host.authenticate(Some(TOKEN));
        assert_eq!(
            host.send(ClientToServer::RegisterHost { monitor_count: 2 }),
            Flow::Continue
        );
        match host.next() {
            Some(ServerToClient::HostRegistered { .. }) => {}
            other => panic!("se esperaba HostRegistered, llego {other:?}"),
        }
        host
    }

    /// Controlador autenticado.
    fn ready_controller(state: &Arc<AppState>) -> Client {
        let mut controller = Client::new(state, DeviceRole::Controller, "iPad", "192.0.2.20");
        controller.authenticate(None);
        controller
    }

    // --- autenticacion ---

    #[test]
    fn a_host_with_the_right_token_authenticates() {
        let state = app_state();
        let mut host = Client::new(&state, DeviceRole::Host, "PC", "192.0.2.10");
        host.authenticate(Some(TOKEN));
        assert_eq!(host.connection.device_id(), Some(host.id()));
        assert!(state.shared().hub.is_connected(host.id()));
    }

    #[test]
    fn authentication_hands_over_usable_ice_servers() {
        // Sin servidores ICE utilizables la conexion solo funcionaria en la red
        // local, que es el fallo que este proyecto no debe tener.
        let state = app_state();
        let mut controller = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        controller.hello(None);
        let nonce_hex = match controller.next() {
            Some(ServerToClient::Challenge { nonce_hex, .. }) => nonce_hex,
            other => panic!("se esperaba reto, llego {other:?}"),
        };
        let proof = controller.proof_for(&nonce_hex, NOW);
        controller.send(proof);

        match controller.next() {
            Some(ServerToClient::AuthAccepted { ice_servers, .. }) => {
                assert!(!ice_servers.is_empty());
                assert!(
                    ice_servers
                        .iter()
                        .all(nexus_core::protocol::signaling::IceServer::is_usable)
                );
                assert!(
                    ice_servers
                        .iter()
                        .any(|s| s.urls.iter().any(|u| u.starts_with("turn:"))),
                    "debe incluir relay: sin el, un NAT simetrico corta la sesion"
                );
            }
            other => panic!("se esperaba AuthAccepted, llego {other:?}"),
        }
    }

    #[test]
    fn a_new_host_without_a_token_is_rejected_and_disconnected() {
        let state = app_state();
        let mut host = Client::new(&state, DeviceRole::Host, "PC", "192.0.2.10");
        assert_eq!(host.hello(None), Flow::Close("autenticacion rechazada"));
        assert!(matches!(
            host.next(),
            Some(ServerToClient::AuthRejected { .. })
        ));
        assert!(state.shared().auth.registry().is_empty());
    }

    #[test]
    fn a_rejection_does_not_say_what_went_wrong() {
        // Distinguir motivos permitiria averiguar si un identificador existe.
        let state = app_state();
        let mut host = Client::new(&state, DeviceRole::Host, "PC", "192.0.2.10");
        host.hello(Some("testigo-equivocado-pero-largo-igual"));
        match host.next() {
            Some(ServerToClient::AuthRejected { reason, .. }) => {
                assert_eq!(reason, AuthRejection::InvalidCredentials);
            }
            other => panic!("se esperaba AuthRejected, llego {other:?}"),
        }
    }

    #[test]
    fn an_unusable_public_key_is_rejected() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        let flow = client.send(ClientToServer::Hello {
            device: client.info.clone(),
            capabilities: CapabilitySet::empty(),
            // Clave de orden pequeno: `DevicePublicKey::from_hex` la rechaza.
            public_key_hex: "00".repeat(32),
            enrollment_token: None,
        });
        assert_eq!(flow, Flow::Close("autenticacion rechazada"));
    }

    #[test]
    fn a_wrong_signature_is_rejected_and_the_identity_is_forgotten() {
        // Quien se presenta y no demuestra la clave no debe dejar rastro en el
        // directorio: si lo dejase, bastaria presentarse en bucle para
        // llenarlo.
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        client.hello(None);
        assert_eq!(state.shared().auth.registry().len(), 1, "alta provisional");
        client.drain();

        let flow = client.send(ClientToServer::AuthProof {
            device_id: client.id(),
            client_nonce_hex: Nonce::generate().to_hex(),
            signature_hex: "ab".repeat(64),
        });

        assert_eq!(flow, Flow::Close("autenticacion rechazada"));
        assert!(
            state.shared().auth.registry().is_empty(),
            "el alta sin demostrar debe deshacerse"
        );
    }

    #[test]
    fn an_already_known_device_is_not_forgotten_after_a_failure() {
        // Lo contrario del caso anterior: un dispositivo legitimo no puede
        // perder su alta porque alguien falle una firma en su nombre.
        let state = app_state();
        let mut controller = ready_controller(&state);
        let info = controller.info.clone();
        let identity_key = controller.identity.public_key().to_hex();

        let (outbound, mut inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
        let mut impostor =
            Connection::new(Arc::clone(&state), "198.51.100.5".to_owned(), outbound, NOW);
        let raw = Envelope::at(
            NOW,
            ClientToServer::Hello {
                device: info,
                capabilities: CapabilitySet::empty(),
                public_key_hex: identity_key,
                enrollment_token: None,
            },
        )
        .encode()
        .expect("codifica");
        assert_eq!(impostor.handle_text(&raw, NOW), Flow::Continue);
        while inbox.try_recv().is_ok() {}

        let raw = Envelope::at(
            NOW,
            ClientToServer::AuthProof {
                device_id: controller.id(),
                client_nonce_hex: Nonce::generate().to_hex(),
                signature_hex: "cd".repeat(64),
            },
        )
        .encode()
        .expect("codifica");
        assert_eq!(
            impostor.handle_text(&raw, NOW),
            Flow::Close("autenticacion rechazada")
        );
        assert_eq!(
            state.shared().auth.registry().len(),
            1,
            "el dispositivo legitimo sigue dado de alta"
        );
        controller.drain();
    }

    #[test]
    fn taking_over_an_identifier_with_another_key_is_rejected() {
        let state = app_state();
        let victim = ready_controller(&state);

        let mut attacker = Client::new(&state, DeviceRole::Controller, "iPad", "198.51.100.5");
        attacker.info.id = victim.id();
        // `attacker.identity` es otra clave distinta para el mismo id.
        assert_eq!(attacker.hello(None), Flow::Close("autenticacion rechazada"));
    }

    #[test]
    fn signing_for_another_device_is_rejected() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        client.hello(None);
        client.drain();

        let flow = client.send(ClientToServer::AuthProof {
            device_id: DeviceId::new(),
            client_nonce_hex: Nonce::generate().to_hex(),
            signature_hex: "ab".repeat(64),
        });
        assert_eq!(flow, Flow::Close("autenticacion rechazada"));
    }

    #[test]
    fn a_challenge_cannot_be_answered_twice() {
        // Un handshake capturado no debe servir una segunda vez.
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        client.hello(None);
        let nonce_hex = match client.next() {
            Some(ServerToClient::Challenge { nonce_hex, .. }) => nonce_hex,
            other => panic!("se esperaba reto, llego {other:?}"),
        };
        let proof = client.proof_for(&nonce_hex, NOW);
        client.send(proof.clone());
        client.drain();

        // Otra conexion del mismo dispositivo reutilizando el mismo reto.
        let (outbound, _inbox) = mpsc::channel(PEER_QUEUE_DEPTH);
        let mut replay =
            Connection::new(Arc::clone(&state), "198.51.100.5".to_owned(), outbound, NOW);
        let hello = Envelope::at(
            NOW,
            ClientToServer::Hello {
                device: client.info.clone(),
                capabilities: CapabilitySet::empty(),
                public_key_hex: client.identity.public_key().to_hex(),
                enrollment_token: None,
            },
        )
        .encode()
        .expect("codifica");
        replay.handle_text(&hello, NOW);

        let raw = Envelope::at(NOW, proof).encode().expect("codifica");
        assert_eq!(
            replay.handle_text(&raw, NOW),
            Flow::Close("autenticacion rechazada"),
            "el reto es de un solo uso"
        );
    }

    #[test]
    fn a_proof_with_a_skewed_clock_is_rejected() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        client.hello(None);
        let nonce_hex = match client.next() {
            Some(ServerToClient::Challenge { nonce_hex, .. }) => nonce_hex,
            other => panic!("se esperaba reto, llego {other:?}"),
        };

        let desfase = Timestamp(NOW.0 - 10 * 60 * 1_000);
        let proof = client.proof_for(&nonce_hex, desfase);
        assert_eq!(
            client.send_at(proof, desfase, NOW),
            Flow::Close("autenticacion rechazada")
        );
    }

    #[test]
    fn repeated_failures_from_one_origin_end_up_rate_limited() {
        let state = app_state();
        let mut last = None;
        for _ in 0..12 {
            let mut client = Client::new(&state, DeviceRole::Host, "PC", "198.51.100.9");
            client.hello(Some("testigo-incorrecto-pero-suficientemente-largo"));
            last = client.next();
            if matches!(
                last,
                Some(ServerToClient::AuthRejected {
                    reason: AuthRejection::RateLimited,
                    ..
                })
            ) {
                break;
            }
        }

        match last {
            Some(ServerToClient::AuthRejected {
                reason: AuthRejection::RateLimited,
                retry_after_secs,
            }) => {
                assert!(
                    retry_after_secs.is_some_and(|secs| secs > 0),
                    "hay que decirle cuanto esperar"
                );
            }
            other => panic!("se esperaba un bloqueo por intentos, llego {other:?}"),
        }
    }

    #[test]
    fn a_revoked_device_is_told_so() {
        // Aqui si se le dice el motivo: ya presento la clave correcta, de modo
        // que es el dispositivo de verdad y no se le revela nada nuevo.
        let state = app_state();
        let controller = ready_controller(&state);
        state
            .shared()
            .auth
            .registry_mut()
            .revoke(controller.id(), NOW)
            .expect("revoca");

        let mut again = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        again.info = controller.info.clone();
        again.identity = DeviceIdentity::from_secret_bytes(
            controller.id(),
            controller.identity.export_secret().as_slice(),
        )
        .expect("misma clave");

        again.hello(None);
        match again.next() {
            Some(ServerToClient::AuthRejected { reason, .. }) => {
                assert_eq!(reason, AuthRejection::Revoked);
            }
            other => panic!("se esperaba AuthRejected, llego {other:?}"),
        }
    }

    // --- orden del protocolo ---

    #[test]
    fn a_proof_without_a_challenge_is_a_protocol_error() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        let flow = client.send(ClientToServer::AuthProof {
            device_id: client.id(),
            client_nonce_hex: Nonce::generate().to_hex(),
            signature_hex: "ab".repeat(64),
        });
        assert_eq!(flow, Flow::Close("protocolo incorrecto sin autenticar"));
    }

    #[test]
    fn session_messages_before_authentication_are_refused() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        let flow = client.send(ClientToServer::RequestSession {
            host_id: DeviceId::new(),
            requested_permissions: PermissionSet::standard(),
        });
        assert_eq!(flow, Flow::Close("protocolo incorrecto sin autenticar"));
        match client.next() {
            Some(ServerToClient::Error { code, .. }) => assert_eq!(code, "not_authenticated"),
            other => panic!("se esperaba un error, llego {other:?}"),
        }
    }

    #[test]
    fn greeting_twice_is_a_protocol_error() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        client.hello(None);
        client.drain();
        assert_eq!(
            client.hello(None),
            Flow::Close("protocolo incorrecto durante el reto")
        );
    }

    #[test]
    fn greeting_again_after_authenticating_is_a_protocol_error() {
        let state = app_state();
        let mut controller = ready_controller(&state);
        assert_eq!(
            controller.hello(None),
            Flow::Close("protocolo incorrecto ya autenticado")
        );
    }

    #[test]
    fn a_ping_is_answered_in_any_phase() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        assert_eq!(client.send(ClientToServer::Ping), Flow::Continue);
        assert!(matches!(client.next(), Some(ServerToClient::Pong)));
    }

    #[test]
    fn malformed_messages_are_refused_without_echoing_them() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");

        for bad in [
            "",
            "{",
            "null",
            r#"{"v":1,"ts":1,"payload":{"type":"escalar_privilegios"}}"#,
            r#"{"v":99,"ts":1,"payload":{"type":"ping"}}"#,
        ] {
            let flow = client.connection.handle_text(bad, NOW);
            assert_eq!(
                flow,
                Flow::Close("protocolo incorrecto sin autenticar"),
                "{bad:?} deberia rechazarse"
            );
            match client.next() {
                Some(ServerToClient::Error { code, message }) => {
                    assert_eq!(code, "bad_request");
                    assert!(
                        !message.contains(bad) || bad.is_empty(),
                        "el error no debe repetir lo que mando el cliente: {message:?}"
                    );
                }
                other => panic!("se esperaba un error, llego {other:?}"),
            }
            client.connection.phase = Phase::Greeting;
        }
    }

    #[test]
    fn an_oversized_message_is_refused() {
        let state = app_state();
        let mut client = Client::new(&state, DeviceRole::Controller, "iPad", "192.0.2.20");
        let huge = "x".repeat(nexus_transport::MAX_MESSAGE_BYTES + 1);
        assert_eq!(
            client.connection.handle_text(&huge, NOW),
            Flow::Close("protocolo incorrecto sin autenticar")
        );
    }

    // --- sesiones ---

    #[test]
    fn the_full_session_flow_works_end_to_end() {
        let state = app_state();
        let mut host = ready_host(&state);
        let mut controller = ready_controller(&state);

        // El controlador pide sesion y el host la ve llegar.
        assert_eq!(
            controller.send(ClientToServer::RequestSession {
                host_id: host.id(),
                requested_permissions: PermissionSet::standard(),
            }),
            Flow::Continue
        );
        let session_id = match host.next() {
            Some(ServerToClient::SessionRequested {
                session_id,
                controller: who,
                controller_capabilities,
                requested_permissions,
            }) => {
                assert_eq!(who.id, controller.id());
                assert!(
                    controller_capabilities.has(Capability::ScreenStream),
                    "el host necesita las capacidades del controlador para negociar"
                );
                assert!(requested_permissions.has(Permission::ControlPointer));
                session_id
            }
            other => panic!("se esperaba SessionRequested, llego {other:?}"),
        };

        // El host acepta y decide los permisos de verdad.
        let mut negotiated = CapabilitySet::empty();
        negotiated.insert(Capability::ScreenStream);
        assert_eq!(
            host.send(ClientToServer::AcceptSession {
                session_id,
                negotiated_capabilities: negotiated,
                effective_permissions: PermissionSet::view_only(),
            }),
            Flow::Continue
        );
        match controller.next() {
            Some(ServerToClient::SessionAccepted {
                effective_permissions,
                ..
            }) => {
                assert!(
                    !effective_permissions.has(Permission::ControlPointer),
                    "manda lo que concede el host, no lo que pidio el controlador"
                );
            }
            other => panic!("se esperaba SessionAccepted, llego {other:?}"),
        }

        // Negociacion WebRTC reenviada en las dos direcciones.
        controller.send(ClientToServer::Offer {
            session_id,
            sdp: "v=0 oferta".to_owned(),
        });
        assert!(matches!(host.next(), Some(ServerToClient::Offer { .. })));

        host.send(ClientToServer::Answer {
            session_id,
            sdp: "v=0 respuesta".to_owned(),
        });
        assert!(matches!(
            controller.next(),
            Some(ServerToClient::Answer { .. })
        ));

        controller.send(ClientToServer::IceCandidate {
            session_id,
            candidate: "candidate:1 1 UDP 1 192.0.2.1 3478 typ host".to_owned(),
            sdp_mid: Some("0".to_owned()),
        });
        assert!(matches!(
            host.next(),
            Some(ServerToClient::IceCandidate { .. })
        ));

        // Atadura de sesion: el servidor la reenvia y rellena quien la envia.
        host.send(ClientToServer::PeerAuth {
            session_id,
            fingerprint: "sha-256 AA:BB".to_owned(),
            signature_hex: "ef".repeat(64),
        });
        match controller.next() {
            Some(ServerToClient::PeerAuth { device_id, .. }) => {
                assert_eq!(device_id, host.id());
            }
            other => panic!("se esperaba PeerAuth, llego {other:?}"),
        }

        // Cierre ordenado.
        assert_eq!(
            controller.send(ClientToServer::Bye { session_id }),
            Flow::Continue
        );
        assert!(matches!(host.next(), Some(ServerToClient::PeerGone { .. })));
        assert_eq!(state.shared().hub.session_count(), 0);
    }

    #[test]
    fn a_host_can_reject_a_session_and_the_reason_is_cleaned() {
        let state = app_state();
        let mut host = ready_host(&state);
        let mut controller = ready_controller(&state);
        controller.send(ClientToServer::RequestSession {
            host_id: host.id(),
            requested_permissions: PermissionSet::standard(),
        });
        let session_id = match host.next() {
            Some(ServerToClient::SessionRequested { session_id, .. }) => session_id,
            other => panic!("se esperaba SessionRequested, llego {other:?}"),
        };

        host.send(ClientToServer::RejectSession {
            session_id,
            reason: format!("no autorizado\n[ADMIN] {}", "x".repeat(400)),
        });

        match controller.next() {
            Some(ServerToClient::SessionRejected { reason, .. }) => {
                assert!(!reason.contains('\n'), "{reason:?}");
                assert!(reason.chars().count() <= MAX_REASON_LEN, "{reason:?}");
            }
            other => panic!("se esperaba SessionRejected, llego {other:?}"),
        }
        assert_eq!(state.shared().hub.session_count(), 0);
    }

    #[test]
    fn a_host_with_no_monitors_cannot_register() {
        let state = app_state();
        let mut host = Client::new(&state, DeviceRole::Host, "PC", "192.0.2.10");
        host.authenticate(Some(TOKEN));
        host.send(ClientToServer::RegisterHost { monitor_count: 0 });
        match host.next() {
            Some(ServerToClient::Error { code, .. }) => assert_eq!(code, "no_monitors"),
            other => panic!("se esperaba un error, llego {other:?}"),
        }
    }

    #[test]
    fn a_controller_cannot_register_as_a_host() {
        let state = app_state();
        let mut controller = ready_controller(&state);
        controller.send(ClientToServer::RegisterHost { monitor_count: 1 });
        match controller.next() {
            Some(ServerToClient::Error { code, .. }) => assert_eq!(code, "wrong_role"),
            other => panic!("se esperaba un error, llego {other:?}"),
        }
    }

    #[test]
    fn requesting_a_session_against_an_unknown_host_says_unavailable() {
        let state = app_state();
        let mut controller = ready_controller(&state);
        controller.send(ClientToServer::RequestSession {
            host_id: DeviceId::new(),
            requested_permissions: PermissionSet::standard(),
        });
        match controller.next() {
            Some(ServerToClient::Error { code, .. }) => assert_eq!(code, "host_unavailable"),
            other => panic!("se esperaba un error, llego {other:?}"),
        }
    }

    #[test]
    fn relaying_into_someone_elses_session_is_refused() {
        let state = app_state();
        let mut host = ready_host(&state);
        let mut controller = ready_controller(&state);
        let mut intruder = Client::new(&state, DeviceRole::Controller, "iPhone", "198.51.100.7");
        intruder.authenticate(None);

        controller.send(ClientToServer::RequestSession {
            host_id: host.id(),
            requested_permissions: PermissionSet::standard(),
        });
        let session_id = match host.next() {
            Some(ServerToClient::SessionRequested { session_id, .. }) => session_id,
            other => panic!("se esperaba SessionRequested, llego {other:?}"),
        };

        intruder.send(ClientToServer::Offer {
            session_id,
            sdp: "v=0 intruso".to_owned(),
        });
        match intruder.next() {
            Some(ServerToClient::Error { code, .. }) => assert_eq!(code, "not_a_member"),
            other => panic!("se esperaba not_a_member, llego {other:?}"),
        }
        assert!(host.next().is_none(), "al host no debe llegarle nada");
    }

    #[test]
    fn a_controller_cannot_accept_its_own_session() {
        let state = app_state();
        let mut host = ready_host(&state);
        let mut controller = ready_controller(&state);
        controller.send(ClientToServer::RequestSession {
            host_id: host.id(),
            requested_permissions: PermissionSet::full(),
        });
        let session_id = match host.next() {
            Some(ServerToClient::SessionRequested { session_id, .. }) => session_id,
            other => panic!("se esperaba SessionRequested, llego {other:?}"),
        };

        controller.send(ClientToServer::AcceptSession {
            session_id,
            negotiated_capabilities: CapabilitySet::empty(),
            effective_permissions: PermissionSet::full(),
        });
        match controller.next() {
            Some(ServerToClient::Error { code, .. }) => assert_eq!(code, "not_a_member"),
            other => panic!("un controlador no puede concederse permisos: {other:?}"),
        }
    }

    #[test]
    fn pairing_codes_are_not_relayed_and_it_is_said_clearly() {
        let state = app_state();
        let mut controller = ready_controller(&state);
        controller.send(ClientToServer::RedeemPairingCode {
            host_id: DeviceId::new(),
            pairing_id: nexus_core::PairingId::new(),
            code: "ABCD2345".to_owned(),
            public_key_hex: controller.identity.public_key().to_hex(),
        });
        match controller.next() {
            Some(ServerToClient::Error { code, .. }) => {
                assert_eq!(code, "pairing_relay_unavailable");
            }
            other => panic!("se esperaba un rechazo explicito, llego {other:?}"),
        }
    }

    #[test]
    fn closing_a_connection_frees_the_session_and_warns_the_other_end() {
        let state = app_state();
        let mut host = ready_host(&state);
        let mut controller = ready_controller(&state);
        controller.send(ClientToServer::RequestSession {
            host_id: host.id(),
            requested_permissions: PermissionSet::standard(),
        });
        host.drain();

        host.connection.close("prueba", NOW);
        assert!(matches!(
            controller.next(),
            Some(ServerToClient::PeerGone { .. })
        ));
        assert_eq!(state.shared().hub.session_count(), 0);
        assert!(!state.shared().hub.is_connected(host.id()));
    }

    // --- registro ---

    #[test]
    fn the_audit_log_records_the_connection_and_the_authentication() {
        let state = app_state();
        let host = ready_host(&state);
        let lines: Vec<String> = state
            .shared()
            .audit
            .entries()
            .map(ToString::to_string)
            .collect();

        assert!(
            lines.iter().any(|l| l.contains("conexion abierta")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("autenticacion") && l.contains("alta aceptada")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("disponible") && l.contains(&host.id().short())),
            "{lines:?}"
        );
    }

    #[test]
    fn the_audit_log_never_contains_the_enrollment_token() {
        let state = app_state();
        let _host = ready_host(&state);
        let mut failed = Client::new(&state, DeviceRole::Host, "PC", "198.51.100.3");
        failed.hello(Some(TOKEN));

        for entry in state.shared().audit.entries() {
            let line = entry.to_string();
            assert!(
                !line.contains(TOKEN),
                "el testigo de alta no debe aparecer en el registro: {line}"
            );
        }
    }

    #[test]
    fn the_audit_log_records_a_throttled_origin() {
        let state = app_state();
        for _ in 0..12 {
            let mut client = Client::new(&state, DeviceRole::Host, "PC", "198.51.100.9");
            client.hello(Some("testigo-incorrecto-pero-largo-de-verdad"));
        }
        assert!(
            state
                .shared()
                .audit
                .entries()
                .any(|e| e.to_string().contains("bloqueado")),
            "un origen insistente debe quedar anotado"
        );
    }

    // --- direccion de origen ---

    #[test]
    fn the_proxy_header_is_ignored_unless_configured() {
        let state = app_state();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7".parse().expect("valida"));
        let addr: SocketAddr = "192.0.2.1:5000".parse().expect("valida");

        assert_eq!(
            remote_label(&state, Some(addr), &headers),
            "192.0.2.1",
            "creer la cabecera sin configurarlo permitiria falsear el origen"
        );
    }

    #[test]
    fn the_proxy_header_is_used_when_configured() {
        let mut state = app_state();
        Arc::get_mut(&mut state)
            .expect("unica referencia")
            .config
            .trust_proxy_header = true;

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.7, 192.0.2.1".parse().expect("valida"),
        );
        let addr: SocketAddr = "192.0.2.1:5000".parse().expect("valida");
        assert_eq!(remote_label(&state, Some(addr), &headers), "203.0.113.7");
    }

    #[test]
    fn a_connection_without_an_address_still_gets_a_label() {
        let state = app_state();
        assert_eq!(remote_label(&state, None, &HeaderMap::new()), "desconocido");
    }

    #[test]
    fn a_hostile_proxy_header_cannot_forge_log_lines() {
        let mut state = app_state();
        Arc::get_mut(&mut state)
            .expect("unica referencia")
            .config
            .trust_proxy_header = true;

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.7 \t falso".parse().expect("valida"),
        );
        let label = remote_label(&state, None, &headers);
        assert!(!label.contains('\t'), "{label:?}");
        assert!(label.chars().count() <= 64, "{label:?}");
    }
}
