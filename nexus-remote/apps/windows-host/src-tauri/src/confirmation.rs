//! La confirmacion que una persona tiene que dar en el PC.
//!
//! ## El agujero que esto cierra
//!
//! El protocolo contesta a una peticion de apagado con
//! [`ControlResponse::ConfirmationRequired`], que lleva un token para
//! reenviar. Implementado de la forma obvia —emitir el token y aceptarlo
//! cuando vuelva— la confirmacion **no confirma nada**: quien pide el apagado
//! recibe el token al instante y lo reenvia acto seguido, sin que nadie haya
//! aprobado nada en el equipo. Seria un paso de mas que da sensacion de
//! seguridad sin anadir ninguna.
//!
//! Aqui el token **nace sin validez**. Se emite, si, pero canjearlo no hace
//! nada hasta que una persona delante del PC lo apruebe en el panel. Hasta
//! entonces el host contesta que sigue esperando.
//!
//! ## El reparto de secretos
//!
//! Hay dos identificadores a proposito, y no se pueden intercambiar:
//!
//! - El **token** es secreto y se envia solo a quien pidio la operacion.
//!   Demuestra «soy el mismo que lo pidio».
//! - El **identificador** es publico y se envia solo al panel del host.
//!   Sirve para aprobar o rechazar.
//!
//! Quien pide nunca ve el identificador, asi que no puede aprobarse a si
//! mismo; el panel nunca necesita el token.
//!
//! [`ControlResponse::ConfirmationRequired`]: nexus_core::protocol::control::ControlResponse::ConfirmationRequired

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexus_core::Timestamp;
use nexus_core::protocol::control::PowerAction;
use nexus_security::tokens::{generate_secret, secret_matches};

/// Cuanto vale una confirmacion desde que se pide.
///
/// Cubre el tiempo de acercarse al PC y mirar, sin dejar una aprobacion viva
/// indefinidamente: una aprobacion de hace media hora no deberia poder
/// apagar el equipo ahora.
pub const DEFAULT_TTL: Duration = Duration::from_secs(120);

/// Lo que ha decidido la persona del PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Todavia no ha dicho nada.
    Waiting,
    /// Ha dicho que si.
    Approved,
    /// Ha dicho que no.
    Refused,
}

/// Una confirmacion a la espera.
#[derive(Debug, Clone)]
pub struct Pending {
    /// Identificador publico, el que ve el panel.
    pub id: String,
    /// Operacion que se confirmaria.
    pub action: PowerAction,
    /// Texto que lee la persona.
    pub prompt: String,
    /// Cuando se pidio.
    pub created_at: Timestamp,
    /// Token secreto. Solo lo conoce quien pidio la operacion.
    token: String,
    decision: Decision,
}

impl Pending {
    /// Lo que ha decidido la persona.
    #[must_use]
    pub const fn decision(&self) -> Decision {
        self.decision
    }

    /// Indica si ya caduco.
    #[must_use]
    pub fn expired(&self, now: Timestamp, ttl: Duration) -> bool {
        now.0.saturating_sub(self.created_at.0) >= ttl.as_millis().min(u128::from(u64::MAX)) as u64
    }
}

/// Lo que el host contesta a quien pide una operacion que exige confirmacion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requested {
    /// Token secreto a reenviar cuando se reintente.
    pub token: String,
    /// Texto que el host mostrara a la persona.
    pub prompt: String,
}

/// Resultado de presentar un token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redeem {
    /// Aprobado por una persona: la operacion puede ejecutarse.
    Execute,
    /// El token vale, pero nadie ha aprobado nada todavia.
    Waiting,
    /// La persona del PC dijo que no.
    Refused,
    /// El token no sirve, con el motivo.
    Invalid(&'static str),
}

/// Las confirmaciones de una sesion.
///
/// Solo hay una a la vez. Permitir varias dejaria a quien pide llenar el panel
/// de avisos hasta que alguien aprobara uno por cansancio, que es justo lo que
/// una confirmacion debe evitar.
#[derive(Debug)]
pub struct ConfirmationGate {
    pending: Option<Pending>,
    ttl: Duration,
    next_id: u64,
}

impl ConfirmationGate {
    /// Crea la puerta con la caducidad por omision.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: None,
            ttl: DEFAULT_TTL,
            next_id: 1,
        }
    }

    /// Crea la puerta con otra caducidad. Util en pruebas.
    #[must_use]
    pub const fn with_ttl(ttl: Duration) -> Self {
        Self {
            pending: None,
            ttl,
            next_id: 1,
        }
    }

    /// Pide confirmacion para una operacion.
    ///
    /// Si ya habia una esperando para la **misma** operacion, devuelve el
    /// mismo token: reintentar no debe llenar el panel de avisos repetidos.
    /// Si era para otra, la sustituye, y el token anterior deja de valer.
    pub fn request(&mut self, action: PowerAction, now: Timestamp) -> Requested {
        if let Some(actual) = &self.pending
            && actual.action == action
            && actual.decision == Decision::Waiting
            && !actual.expired(now, self.ttl)
        {
            return Requested {
                token: actual.token.clone(),
                prompt: actual.prompt.clone(),
            };
        }

        let token = generate_secret();
        let id = format!("conf-{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);

        let pending = Pending {
            id,
            action,
            prompt: prompt_for(action),
            created_at: now,
            token: token.clone(),
            decision: Decision::Waiting,
        };
        let prompt = pending.prompt.clone();
        self.pending = Some(pending);

        Requested { token, prompt }
    }

    /// La confirmacion que el panel tiene que mostrar, si hay alguna viva.
    #[must_use]
    pub fn pending(&self, now: Timestamp) -> Option<&Pending> {
        self.pending
            .as_ref()
            .filter(|p| !p.expired(now, self.ttl) && p.decision == Decision::Waiting)
    }

    /// Una persona del PC aprueba la confirmacion.
    ///
    /// Devuelve `false` si el identificador no corresponde a ninguna viva.
    pub fn approve(&mut self, id: &str, now: Timestamp) -> bool {
        self.decide(id, now, Decision::Approved)
    }

    /// Una persona del PC la rechaza.
    pub fn refuse(&mut self, id: &str, now: Timestamp) -> bool {
        self.decide(id, now, Decision::Refused)
    }

    fn decide(&mut self, id: &str, now: Timestamp, decision: Decision) -> bool {
        let ttl = self.ttl;
        let Some(pending) = self.pending.as_mut() else {
            return false;
        };
        // El identificador es publico, no un secreto: compararlo normal esta
        // bien. Lo que no puede compararse asi es el token.
        if pending.id != id || pending.decision != Decision::Waiting || pending.expired(now, ttl) {
            return false;
        }
        pending.decision = decision;
        true
    }

    /// Presenta un token para ejecutar la operacion.
    ///
    /// Un canje que sale bien **consume** la confirmacion: el mismo token no
    /// puede apagar el equipo dos veces.
    pub fn redeem(&mut self, action: PowerAction, token: &str, now: Timestamp) -> Redeem {
        let ttl = self.ttl;
        let Some(pending) = self.pending.as_ref() else {
            return Redeem::Invalid("no hay ninguna confirmacion pendiente");
        };

        // El token es un secreto: se compara en tiempo constante para no
        // filtrar byte a byte cual era.
        if !secret_matches(&pending.token, token) {
            return Redeem::Invalid("el token no corresponde a ninguna confirmacion");
        }
        if pending.action != action {
            return Redeem::Invalid("el token es de otra operacion");
        }
        if pending.expired(now, ttl) {
            self.pending = None;
            return Redeem::Invalid("la confirmacion caduco");
        }

        match pending.decision {
            Decision::Waiting => Redeem::Waiting,
            Decision::Refused => {
                self.pending = None;
                Redeem::Refused
            }
            Decision::Approved => {
                // Un solo uso: se consume al canjearla.
                self.pending = None;
                Redeem::Execute
            }
        }
    }

    /// Olvida la confirmacion pendiente.
    pub fn clear(&mut self) {
        self.pending = None;
    }
}

impl Default for ConfirmationGate {
    fn default() -> Self {
        Self::new()
    }
}

/// La puerta de confirmaciones, compartida entre la sesion y el panel.
///
/// Las dos mitades de una confirmacion ocurren en sitios distintos: la pide la
/// sesion remota y la aprueba una persona en el panel del host. Sin algo que
/// las una, la confirmacion no tendria donde resolverse.
///
/// El cerrojo se toma y se suelta dentro de cada operacion, que son todas
/// cortas: ninguna espera ni hace entrada/salida con el tomado.
#[derive(Debug, Clone, Default)]
pub struct SharedConfirmations(Arc<Mutex<ConfirmationGate>>);

impl SharedConfirmations {
    /// Crea una puerta compartida nueva.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Crea una con otra caducidad. Util en pruebas.
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self(Arc::new(Mutex::new(ConfirmationGate::with_ttl(ttl))))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ConfirmationGate> {
        // Un cerrojo envenenado significa que otro hilo entro en panico con el
        // tomado. La puerta no guarda nada que quede a medias, asi que se
        // sigue con lo que hay en lugar de propagar el panico a esta sesion.
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Pide confirmacion para una operacion.
    pub fn request(&self, action: PowerAction, now: Timestamp) -> Requested {
        self.lock().request(action, now)
    }

    /// La confirmacion que el panel tiene que mostrar, si hay alguna viva.
    #[must_use]
    pub fn pending(&self, now: Timestamp) -> Option<Pending> {
        self.lock().pending(now).cloned()
    }

    /// Una persona del PC la aprueba.
    pub fn approve(&self, id: &str, now: Timestamp) -> bool {
        self.lock().approve(id, now)
    }

    /// Una persona del PC la rechaza.
    pub fn refuse(&self, id: &str, now: Timestamp) -> bool {
        self.lock().refuse(id, now)
    }

    /// Presenta un token para ejecutar la operacion.
    pub fn redeem(&self, action: PowerAction, token: &str, now: Timestamp) -> Redeem {
        self.lock().redeem(action, token, now)
    }

    /// Olvida lo pendiente.
    pub fn clear(&self) {
        self.lock().clear();
    }
}

/// Texto que lee la persona del PC.
fn prompt_for(action: PowerAction) -> String {
    match action {
        PowerAction::Lock => "Bloquear la sesion de este equipo",
        PowerAction::Reboot => "Reiniciar este equipo",
        PowerAction::Shutdown => "Apagar este equipo",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp(1_000_000);

    fn gate() -> ConfirmationGate {
        ConfirmationGate::with_ttl(Duration::from_secs(60))
    }

    /// Avanza el reloj los segundos que se digan.
    fn luego(secs: u64) -> Timestamp {
        Timestamp(NOW.0 + secs * 1_000)
    }

    #[test]
    fn pedir_confirmacion_no_ejecuta_nada_todavia() {
        // La prueba que da sentido a todo el modulo: quien pide recibe el
        // token al instante, y con el token en la mano no puede hacer nada.
        let mut g = gate();
        let pedido = g.request(PowerAction::Reboot, NOW);

        assert!(!pedido.token.is_empty());
        assert_eq!(
            g.redeem(PowerAction::Reboot, &pedido.token, NOW),
            Redeem::Waiting,
            "sin que nadie apruebe, el token no debe valer"
        );
    }

    #[test]
    fn solo_tras_aprobarlo_una_persona_se_puede_ejecutar() {
        let mut g = gate();
        let pedido = g.request(PowerAction::Shutdown, NOW);

        let id = g.pending(NOW).expect("hay una esperando").id.clone();
        assert!(g.approve(&id, NOW), "el panel la aprueba");

        assert_eq!(
            g.redeem(PowerAction::Shutdown, &pedido.token, luego(1)),
            Redeem::Execute
        );
    }

    #[test]
    fn quien_pide_no_puede_aprobarse_a_si_mismo() {
        // Quien pide tiene el token, no el identificador. Intentar aprobar con
        // el token no vale: son cosas distintas a proposito.
        let mut g = gate();
        let pedido = g.request(PowerAction::Reboot, NOW);

        assert!(
            !g.approve(&pedido.token, NOW),
            "el token no sirve para aprobar"
        );
        assert_eq!(
            g.redeem(PowerAction::Reboot, &pedido.token, NOW),
            Redeem::Waiting
        );
    }

    #[test]
    fn un_token_inventado_no_vale() {
        let mut g = gate();
        g.request(PowerAction::Reboot, NOW);
        let id = g.pending(NOW).expect("hay una").id.clone();
        g.approve(&id, NOW);

        for falso in ["", "0", "conf-1", &"a".repeat(64)] {
            assert!(
                matches!(
                    g.redeem(PowerAction::Reboot, falso, NOW),
                    Redeem::Invalid(_)
                ),
                "{falso:?} no deberia valer"
            );
        }
    }

    #[test]
    fn un_token_de_otra_operacion_no_vale() {
        // Si no se comprobara, confirmar un reinicio serviria para apagar.
        let mut g = gate();
        let pedido = g.request(PowerAction::Reboot, NOW);
        let id = g.pending(NOW).expect("hay una").id.clone();
        g.approve(&id, NOW);

        assert!(matches!(
            g.redeem(PowerAction::Shutdown, &pedido.token, NOW),
            Redeem::Invalid(_)
        ));
        // Y la buena sigue valiendo.
        assert_eq!(
            g.redeem(PowerAction::Reboot, &pedido.token, NOW),
            Redeem::Execute
        );
    }

    #[test]
    fn una_confirmacion_aprobada_solo_sirve_una_vez() {
        let mut g = gate();
        let pedido = g.request(PowerAction::Reboot, NOW);
        let id = g.pending(NOW).expect("hay una").id.clone();
        g.approve(&id, NOW);

        assert_eq!(
            g.redeem(PowerAction::Reboot, &pedido.token, NOW),
            Redeem::Execute
        );
        assert!(
            matches!(
                g.redeem(PowerAction::Reboot, &pedido.token, NOW),
                Redeem::Invalid(_)
            ),
            "el mismo token no puede reiniciar dos veces"
        );
    }

    #[test]
    fn una_confirmacion_caducada_no_vale_aunque_estuviera_aprobada() {
        // El caso incomodo: alguien aprobo hace media hora y se olvido.
        let mut g = gate();
        let pedido = g.request(PowerAction::Shutdown, NOW);
        let id = g.pending(NOW).expect("hay una").id.clone();
        g.approve(&id, NOW);

        assert!(matches!(
            g.redeem(PowerAction::Shutdown, &pedido.token, luego(61)),
            Redeem::Invalid(_)
        ));
    }

    #[test]
    fn una_confirmacion_caducada_desaparece_del_panel() {
        let mut g = gate();
        g.request(PowerAction::Reboot, NOW);
        assert!(g.pending(NOW).is_some());
        assert!(
            g.pending(luego(61)).is_none(),
            "el panel no debe seguir pidiendo algo que ya caduco"
        );
    }

    #[test]
    fn si_la_persona_dice_que_no_se_dice_que_no() {
        let mut g = gate();
        let pedido = g.request(PowerAction::Shutdown, NOW);
        let id = g.pending(NOW).expect("hay una").id.clone();
        assert!(g.refuse(&id, NOW));

        assert_eq!(
            g.redeem(PowerAction::Shutdown, &pedido.token, NOW),
            Redeem::Refused
        );
        // Y no se puede insistir con el mismo token.
        assert!(matches!(
            g.redeem(PowerAction::Shutdown, &pedido.token, NOW),
            Redeem::Invalid(_)
        ));
    }

    #[test]
    fn una_rechazada_ya_no_se_puede_aprobar() {
        let mut g = gate();
        g.request(PowerAction::Reboot, NOW);
        let id = g.pending(NOW).expect("hay una").id.clone();
        assert!(g.refuse(&id, NOW));
        assert!(
            !g.approve(&id, NOW),
            "no se puede cambiar un no por un si a posteriori"
        );
    }

    #[test]
    fn reintentar_la_misma_operacion_no_llena_el_panel_de_avisos() {
        let mut g = gate();
        let primera = g.request(PowerAction::Reboot, NOW);
        let id_primera = g.pending(NOW).expect("hay una").id.clone();

        let segunda = g.request(PowerAction::Reboot, luego(5));
        let id_segunda = g.pending(luego(5)).expect("sigue habiendo una").id.clone();

        assert_eq!(primera.token, segunda.token, "deberia ser la misma");
        assert_eq!(id_primera, id_segunda);
    }

    #[test]
    fn pedir_otra_operacion_sustituye_la_anterior() {
        let mut g = gate();
        let reinicio = g.request(PowerAction::Reboot, NOW);
        let apagado = g.request(PowerAction::Shutdown, luego(1));
        assert_ne!(reinicio.token, apagado.token);

        let id = g.pending(luego(1)).expect("hay una").id.clone();
        g.approve(&id, luego(1));

        // La nueva vale...
        assert_eq!(
            g.redeem(PowerAction::Shutdown, &apagado.token, luego(2)),
            Redeem::Execute
        );
        // ...y el token viejo ya no existe.
        assert!(matches!(
            g.redeem(PowerAction::Reboot, &reinicio.token, luego(2)),
            Redeem::Invalid(_)
        ));
    }

    #[test]
    fn aprobar_un_identificador_que_no_existe_no_hace_nada() {
        let mut g = gate();
        assert!(!g.approve("conf-99", NOW), "sin nada pendiente");

        let pedido = g.request(PowerAction::Reboot, NOW);
        assert!(!g.approve("conf-99", NOW), "identificador equivocado");
        assert_eq!(
            g.redeem(PowerAction::Reboot, &pedido.token, NOW),
            Redeem::Waiting
        );
    }

    #[test]
    fn sin_nada_pendiente_cualquier_token_se_rechaza() {
        let mut g = gate();
        assert!(matches!(
            g.redeem(PowerAction::Reboot, "cualquiera", NOW),
            Redeem::Invalid(_)
        ));
    }

    #[test]
    fn cerrar_la_sesion_olvida_lo_pendiente() {
        let mut g = gate();
        let pedido = g.request(PowerAction::Shutdown, NOW);
        let id = g.pending(NOW).expect("hay una").id.clone();
        g.approve(&id, NOW);

        g.clear();
        assert!(g.pending(NOW).is_none());
        assert!(
            matches!(
                g.redeem(PowerAction::Shutdown, &pedido.token, NOW),
                Redeem::Invalid(_)
            ),
            "una aprobacion no debe sobrevivir a la sesion que la pidio"
        );
    }

    #[test]
    fn cada_confirmacion_tiene_su_propio_token() {
        let mut g = gate();
        let a = g.request(PowerAction::Reboot, NOW);
        g.clear();
        let b = g.request(PowerAction::Reboot, NOW);
        assert_ne!(a.token, b.token);
    }

    #[test]
    fn la_puerta_compartida_une_las_dos_mitades() {
        // Es la razon de que exista: quien pide y quien aprueba son dos sitios
        // distintos del programa.
        let sesion = SharedConfirmations::with_ttl(Duration::from_secs(60));
        let panel = sesion.clone();

        let pedido = sesion.request(PowerAction::Shutdown, NOW);
        assert_eq!(
            sesion.redeem(PowerAction::Shutdown, &pedido.token, NOW),
            Redeem::Waiting
        );

        let id = panel.pending(NOW).expect("el panel la ve").id;
        assert!(panel.approve(&id, NOW));

        assert_eq!(
            sesion.redeem(PowerAction::Shutdown, &pedido.token, NOW),
            Redeem::Execute
        );
    }

    #[test]
    fn la_puerta_compartida_se_puede_usar_desde_varios_hilos() {
        let puerta = SharedConfirmations::new();
        let pedido = puerta.request(PowerAction::Reboot, Timestamp::now());

        let otro = puerta.clone();
        let hilo = std::thread::spawn(move || {
            let id = otro.pending(Timestamp::now()).expect("hay una").id;
            otro.approve(&id, Timestamp::now())
        });
        assert!(hilo.join().expect("el hilo termina"));

        assert_eq!(
            puerta.redeem(PowerAction::Reboot, &pedido.token, Timestamp::now()),
            Redeem::Execute
        );
    }

    #[test]
    fn el_texto_dice_que_va_a_pasar() {
        for (action, esperado) in [
            (PowerAction::Lock, "Bloquear"),
            (PowerAction::Reboot, "Reiniciar"),
            (PowerAction::Shutdown, "Apagar"),
        ] {
            assert!(
                prompt_for(action).starts_with(esperado),
                "{action:?} deberia decir {esperado}"
            );
        }
    }
}
