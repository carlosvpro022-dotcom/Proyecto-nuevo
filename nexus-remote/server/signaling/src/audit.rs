//! Registro de conexiones del servidor.
//!
//! Anota quien se conecta, cuando, y como acaba cada intento. Sirve para dos
//! cosas concretas: ver si alguien esta probando a entrar, y poder reconstruir
//! que paso cuando una sesion no funciona.
//!
//! # Que no se anota
//!
//! Nunca se escriben secretos. En particular **no** se anotan el testigo de
//! alta, las firmas, los nonces, las credenciales TURN ni ninguna descripcion
//! SDP. Los tipos de este modulo no tienen ningun campo donde quepan: la
//! garantia es estructural, no una norma que haya que recordar.
//!
//! De las claves publicas se anota solo la huella corta, que basta para
//! identificar un dispositivo en el registro sin volcar la clave entera.
//!
//! # Donde acaba
//!
//! En la salida estandar del proceso, via `tracing`, que es de donde lo recoge
//! el gestor de servicios. Y en un anillo en memoria de tamano fijo, para poder
//! consultar lo reciente.
//!
//! **No hay ninguna ruta HTTP que exponga este registro.** Una interfaz de
//! administracion alcanzable desde Internet es exactamente lo que no debe
//! tener este servidor; quien necesite leerlo, entra en la maquina.

use std::collections::VecDeque;
use std::fmt;

use nexus_core::{DeviceId, SessionId, Timestamp};

/// Entradas que guarda el anillo en memoria.
pub const DEFAULT_CAPACITY: usize = 512;

/// Resultado de un intento de autenticacion, para el registro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthResult {
    /// Identidad nueva, registrada en el directorio del servidor.
    Enrolled,
    /// Identidad ya conocida, firma correcta.
    Accepted,
    /// Rechazada. El motivo es texto fijo, nunca datos del cliente.
    Rejected(&'static str),
}

impl fmt::Display for AuthResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enrolled => f.write_str("alta aceptada"),
            Self::Accepted => f.write_str("autenticado"),
            Self::Rejected(reason) => write!(f, "rechazado ({reason})"),
        }
    }
}

/// Hecho digno de registro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEvent {
    /// Se abrio un WebSocket, todavia sin autenticar.
    ConnectionOpened {
        /// Direccion de origen, tal como la ve el servidor.
        remote: String,
    },
    /// Termino un intento de autenticacion.
    Authentication {
        /// Direccion de origen.
        remote: String,
        /// Dispositivo que lo intento, si llego a declararse.
        device_id: Option<DeviceId>,
        /// Huella corta de su clave publica, si llego a presentarla.
        key_fingerprint: Option<String>,
        /// Como acabo.
        result: AuthResult,
    },
    /// Un host se declaro disponible.
    HostRegistered {
        /// Host que se declaro.
        device_id: DeviceId,
        /// Monitores que anuncio.
        monitor_count: u8,
    },
    /// Un controlador pidio sesion contra un host.
    SessionRequested {
        /// Sesion propuesta.
        session_id: SessionId,
        /// Host objetivo.
        host: DeviceId,
        /// Controlador que la pide.
        controller: DeviceId,
    },
    /// El host resolvio la peticion.
    SessionResolved {
        /// Sesion afectada.
        session_id: SessionId,
        /// Si el host la acepto.
        accepted: bool,
    },
    /// Se cerro una sesion.
    SessionClosed {
        /// Sesion cerrada.
        session_id: SessionId,
        /// Motivo, texto fijo.
        reason: &'static str,
    },
    /// Se reenvio una prueba de atadura entre los extremos.
    ///
    /// Se anota que ocurrio, no su contenido: la firma no aporta nada al
    /// registro y la huella es larga.
    BindingRelayed {
        /// Sesion afectada.
        session_id: SessionId,
        /// Quien la envio.
        from: DeviceId,
    },
    /// Se cerro una conexion.
    ConnectionClosed {
        /// Dispositivo, si estaba autenticado.
        device_id: Option<DeviceId>,
        /// Motivo, texto fijo.
        reason: &'static str,
    },
    /// Se bloqueo un origen por exceso de intentos.
    Throttled {
        /// Direccion de origen.
        remote: String,
        /// Segundos que debe esperar.
        retry_after_secs: u64,
    },
}

impl fmt::Display for AuditEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionOpened { remote } => {
                write!(f, "conexion abierta desde {remote}")
            }
            Self::Authentication {
                remote,
                device_id,
                key_fingerprint,
                result,
            } => {
                write!(f, "autenticacion desde {remote}")?;
                if let Some(id) = device_id {
                    write!(f, " de {}", id.short())?;
                }
                if let Some(fingerprint) = key_fingerprint {
                    write!(f, " (clave {fingerprint})")?;
                }
                write!(f, ": {result}")
            }
            Self::HostRegistered {
                device_id,
                monitor_count,
            } => write!(
                f,
                "host {} disponible con {monitor_count} monitores",
                device_id.short()
            ),
            Self::SessionRequested {
                session_id,
                host,
                controller,
            } => write!(
                f,
                "sesion {} pedida por {} contra {}",
                session_id.short(),
                controller.short(),
                host.short()
            ),
            Self::SessionResolved {
                session_id,
                accepted,
            } => write!(
                f,
                "sesion {} {}",
                session_id.short(),
                if *accepted { "aceptada" } else { "rechazada" }
            ),
            Self::SessionClosed { session_id, reason } => {
                write!(f, "sesion {} cerrada ({reason})", session_id.short())
            }
            Self::BindingRelayed { session_id, from } => write!(
                f,
                "atadura de la sesion {} reenviada desde {}",
                session_id.short(),
                from.short()
            ),
            Self::ConnectionClosed { device_id, reason } => {
                write!(f, "conexion cerrada")?;
                if let Some(id) = device_id {
                    write!(f, " de {}", id.short())?;
                }
                write!(f, " ({reason})")
            }
            Self::Throttled {
                remote,
                retry_after_secs,
            } => write!(
                f,
                "origen {remote} bloqueado durante {retry_after_secs} s por exceso de intentos"
            ),
        }
    }
}

/// Entrada del registro, con su instante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// Cuando ocurrio.
    pub at: Timestamp,
    /// Que ocurrio.
    pub event: AuditEvent,
}

impl fmt::Display for AuditEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.at.0, self.event)
    }
}

/// Anillo de entradas recientes.
#[derive(Debug)]
pub struct AuditLog {
    entries: VecDeque<AuditEntry>,
    capacity: usize,
    total: u64,
}

impl AuditLog {
    /// Crea el registro con la capacidad por omision.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Crea el registro con una capacidad concreta.
    ///
    /// Una capacidad de cero deja el anillo vacio pero sigue emitiendo a la
    /// salida del proceso.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.min(DEFAULT_CAPACITY)),
            capacity,
            total: 0,
        }
    }

    /// Anota un hecho.
    pub fn record(&mut self, at: Timestamp, event: AuditEvent) {
        let entry = AuditEntry { at, event };
        tracing::info!(target: "nexus::audit", "{}", entry.event);

        self.total = self.total.saturating_add(1);
        if self.capacity == 0 {
            return;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Entradas guardadas, de la mas antigua a la mas reciente.
    pub fn entries(&self) -> impl Iterator<Item = &AuditEntry> {
        self.entries.iter()
    }

    /// Entradas guardadas ahora mismo.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Indica si no hay ninguna entrada guardada.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hechos anotados desde que arranco el proceso, aunque ya no se guarden.
    #[must_use]
    pub const fn total_recorded(&self) -> u64 {
        self.total
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp(1_730_000_000_000);

    fn opened(remote: &str) -> AuditEvent {
        AuditEvent::ConnectionOpened {
            remote: remote.to_owned(),
        }
    }

    #[test]
    fn events_are_recorded_in_order() {
        let mut log = AuditLog::new();
        log.record(NOW, opened("192.0.2.1:5000"));
        log.record(Timestamp(NOW.0 + 1), opened("192.0.2.2:5000"));

        let rendered: Vec<String> = log.entries().map(ToString::to_string).collect();
        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].contains("192.0.2.1"), "{:?}", rendered[0]);
        assert!(rendered[1].contains("192.0.2.2"), "{:?}", rendered[1]);
    }

    #[test]
    fn the_ring_drops_the_oldest_entries() {
        let mut log = AuditLog::with_capacity(2);
        for n in 0..5 {
            log.record(Timestamp(NOW.0 + n), opened(&format!("192.0.2.{n}:1")));
        }
        assert_eq!(log.len(), 2, "no debe crecer sin limite");
        assert_eq!(log.total_recorded(), 5, "el contador no se pierde");

        let first = log.entries().next().expect("hay entradas").to_string();
        assert!(first.contains("192.0.2.3"), "{first}");
    }

    #[test]
    fn a_zero_capacity_log_keeps_nothing_but_still_counts() {
        let mut log = AuditLog::with_capacity(0);
        log.record(NOW, opened("192.0.2.1:1"));
        assert!(log.is_empty());
        assert_eq!(log.total_recorded(), 1);
    }

    #[test]
    fn a_rejection_renders_its_fixed_reason() {
        let mut log = AuditLog::new();
        log.record(
            NOW,
            AuditEvent::Authentication {
                remote: "192.0.2.1:5000".to_owned(),
                device_id: Some(DeviceId::new()),
                key_fingerprint: Some("ab:cd:ef".to_owned()),
                result: AuthResult::Rejected("firma invalida"),
            },
        );
        let line = log.entries().next().expect("hay entrada").to_string();
        assert!(line.contains("rechazado (firma invalida)"), "{line}");
    }

    #[test]
    fn a_device_is_identified_by_its_short_form() {
        // El registro no debe volcar identificadores completos ni claves: con
        // la forma corta se sigue la pista igual de bien.
        let device_id = DeviceId::new();
        let mut log = AuditLog::new();
        log.record(
            NOW,
            AuditEvent::HostRegistered {
                device_id,
                monitor_count: 2,
            },
        );
        let line = log.entries().next().expect("hay entrada").to_string();
        assert!(line.contains(&device_id.short()), "{line}");
        assert!(
            !line.contains(&device_id.to_string()),
            "no debe aparecer el identificador completo: {line}"
        );
    }

    #[test]
    fn every_event_renders_something_legible() {
        let session_id = SessionId::new();
        let device_id = DeviceId::new();
        let events = [
            opened("192.0.2.1:1"),
            AuditEvent::Authentication {
                remote: "192.0.2.1:1".to_owned(),
                device_id: None,
                key_fingerprint: None,
                result: AuthResult::Enrolled,
            },
            AuditEvent::Authentication {
                remote: "192.0.2.1:1".to_owned(),
                device_id: Some(device_id),
                key_fingerprint: Some("ab:cd".to_owned()),
                result: AuthResult::Accepted,
            },
            AuditEvent::HostRegistered {
                device_id,
                monitor_count: 1,
            },
            AuditEvent::SessionRequested {
                session_id,
                host: device_id,
                controller: DeviceId::new(),
            },
            AuditEvent::SessionResolved {
                session_id,
                accepted: true,
            },
            AuditEvent::SessionResolved {
                session_id,
                accepted: false,
            },
            AuditEvent::SessionClosed {
                session_id,
                reason: "cierre ordenado",
            },
            AuditEvent::BindingRelayed {
                session_id,
                from: device_id,
            },
            AuditEvent::ConnectionClosed {
                device_id: Some(device_id),
                reason: "cierre ordenado",
            },
            AuditEvent::ConnectionClosed {
                device_id: None,
                reason: "sin autenticar",
            },
            AuditEvent::Throttled {
                remote: "192.0.2.1:1".to_owned(),
                retry_after_secs: 30,
            },
        ];

        for event in events {
            let rendered = event.to_string();
            assert!(rendered.len() > 10, "demasiado escueto: {rendered:?}");
            assert!(
                !rendered.contains("None") && !rendered.contains("Some("),
                "no debe asomar la representacion de Rust: {rendered:?}"
            );
        }
    }
}
