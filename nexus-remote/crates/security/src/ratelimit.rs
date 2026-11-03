//! Limite de intentos de autenticacion.
//!
//! Un codigo de emparejamiento o una clave filtrada valen poco si quien los
//! prueba solo puede intentarlo unas pocas veces. Este modulo implementa
//! bloqueo progresivo por clave (identificador de dispositivo o direccion de
//! origen): cada tanda de fallos duplica la espera, hasta un tope.
//!
//! El bloqueo se mide sobre el reloj que pasa el llamante, no sobre el del
//! sistema, para que las pruebas sean deterministas y para que el servidor
//! pueda usar su propia fuente de tiempo.

use std::collections::HashMap;
use std::time::Duration;

use nexus_core::Timestamp;

use crate::error::{SecurityError, SecurityResult};

/// Politica de bloqueo por intentos fallidos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitPolicy {
    /// Fallos permitidos antes del primer bloqueo.
    pub max_failures: u32,
    /// Duracion del primer bloqueo.
    pub base_lockout: Duration,
    /// Tope de la duracion del bloqueo.
    pub max_lockout: Duration,
    /// Tiempo sin fallos tras el cual el contador se reinicia.
    pub failure_window: Duration,
}

impl RateLimitPolicy {
    /// Politica estricta para el canje de codigos de emparejamiento.
    ///
    /// Un codigo tiene menos entropia que una clave, asi que se le da menos
    /// margen: cinco fallos y bloqueo creciente desde cinco minutos.
    #[must_use]
    pub const fn pairing() -> Self {
        Self {
            max_failures: 5,
            base_lockout: Duration::from_secs(300),
            max_lockout: Duration::from_secs(3_600),
            failure_window: Duration::from_secs(900),
        }
    }

    /// Politica para la autenticacion por firma de dispositivos ya emparejados.
    ///
    /// Aqui un fallo casi siempre es un error de software o un reloj desviado,
    /// no un ataque viable contra Ed25519, asi que se permite mas margen.
    #[must_use]
    pub const fn device_auth() -> Self {
        Self {
            max_failures: 10,
            base_lockout: Duration::from_secs(30),
            max_lockout: Duration::from_secs(900),
            failure_window: Duration::from_secs(600),
        }
    }

    /// Duracion del bloqueo para una tanda dada (1 = primera).
    #[must_use]
    pub fn lockout_for_round(&self, round: u32) -> Duration {
        if round == 0 {
            return Duration::ZERO;
        }
        let shift = (round - 1).min(20);
        let millis = self
            .base_lockout
            .as_millis()
            .saturating_mul(1_u128 << shift);
        Duration::from_millis(millis.min(self.max_lockout.as_millis()) as u64)
    }
}

impl Default for RateLimitPolicy {
    fn default() -> Self {
        Self::device_auth()
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    lockout_rounds: u32,
    last_failure: Timestamp,
    locked_until: Option<Timestamp>,
}

/// Contador de intentos fallidos con bloqueo progresivo.
#[derive(Debug)]
pub struct AttemptLimiter {
    entries: HashMap<String, Entry>,
    policy: RateLimitPolicy,
    max_keys: usize,
}

impl AttemptLimiter {
    /// Numero maximo de claves vigiladas por omision.
    pub const DEFAULT_MAX_KEYS: usize = 8_192;

    /// Crea un contador con la politica indicada.
    #[must_use]
    pub fn new(policy: RateLimitPolicy) -> Self {
        Self {
            entries: HashMap::new(),
            policy,
            max_keys: Self::DEFAULT_MAX_KEYS,
        }
    }

    /// Comprueba si la clave puede intentarlo ahora.
    ///
    /// # Errors
    ///
    /// [`SecurityError::RateLimited`] con los segundos que faltan si esta
    /// bloqueada.
    pub fn check(&self, key: &str, now: Timestamp) -> SecurityResult<()> {
        let Some(entry) = self.entries.get(key) else {
            return Ok(());
        };
        let Some(locked_until) = entry.locked_until else {
            return Ok(());
        };
        if now.0 >= locked_until.0 {
            return Ok(());
        }
        // Se redondea hacia arriba para no decir nunca "0 segundos" a quien
        // todavia esta bloqueado.
        let remaining_ms = locked_until.0 - now.0;
        Err(SecurityError::RateLimited {
            retry_after_secs: remaining_ms.div_ceil(1_000),
        })
    }

    /// Anota un intento fallido y devuelve el bloqueo aplicado, si lo hay.
    pub fn register_failure(&mut self, key: &str, now: Timestamp) -> Option<Duration> {
        self.prune(now);

        if !self.entries.contains_key(key) && self.entries.len() >= self.max_keys {
            // Con la tabla llena se aplica el bloqueo mas estricto posible a la
            // clave nueva sin anotarla: asi un atacante que rote claves no
            // consigue ni memoria ni intentos gratis.
            return Some(self.policy.lockout_for_round(1));
        }

        let policy = self.policy;
        let entry = self.entries.entry(key.to_owned()).or_insert(Entry {
            failures: 0,
            lockout_rounds: 0,
            last_failure: now,
            locked_until: None,
        });

        // Si llevaba mucho sin fallar, la tanda anterior ya no cuenta.
        if now.abs_diff(entry.last_failure) > policy.failure_window {
            entry.failures = 0;
        }

        entry.failures += 1;
        entry.last_failure = now;

        if entry.failures >= policy.max_failures {
            entry.failures = 0;
            entry.lockout_rounds += 1;
            let lockout = policy.lockout_for_round(entry.lockout_rounds);
            entry.locked_until = Some(now.saturating_add(lockout));
            return Some(lockout);
        }

        None
    }

    /// Anota un intento correcto, que limpia el historial de la clave.
    pub fn register_success(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// Fallos acumulados en la tanda actual.
    #[must_use]
    pub fn failures(&self, key: &str) -> u32 {
        self.entries.get(key).map_or(0, |e| e.failures)
    }

    /// Indica si la clave esta bloqueada ahora mismo.
    #[must_use]
    pub fn is_locked(&self, key: &str, now: Timestamp) -> bool {
        self.check(key, now).is_err()
    }

    /// Olvida las claves cuyo bloqueo y ventana de fallos ya pasaron.
    pub fn prune(&mut self, now: Timestamp) {
        let window = self.policy.failure_window;
        let max_lockout = self.policy.max_lockout;
        self.entries.retain(|_, e| {
            let still_locked = e.locked_until.is_some_and(|until| now.0 < until.0);
            // Se conserva un rato despues del bloqueo para que la escalada no
            // se reinicie en cuanto expira: si no, bastaria esperar el bloqueo
            // y volver a empezar con la espera minima.
            let recent = now.abs_diff(e.last_failure) <= window + max_lockout;
            still_locked || recent
        });
    }

    /// Numero de claves vigiladas.
    #[must_use]
    pub fn tracked_keys(&self) -> usize {
        self.entries.len()
    }
}

impl Default for AttemptLimiter {
    fn default() -> Self {
        Self::new(RateLimitPolicy::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "dev-1";

    fn policy() -> RateLimitPolicy {
        RateLimitPolicy {
            max_failures: 3,
            base_lockout: Duration::from_secs(10),
            max_lockout: Duration::from_secs(80),
            failure_window: Duration::from_secs(60),
        }
    }

    #[test]
    fn an_unknown_key_is_allowed() {
        let limiter = AttemptLimiter::new(policy());
        limiter
            .check("nadie", Timestamp(0))
            .expect("sin historial, adelante");
    }

    #[test]
    fn failures_below_the_limit_do_not_lock() {
        let mut limiter = AttemptLimiter::new(policy());
        assert!(limiter.register_failure(KEY, Timestamp(0)).is_none());
        assert!(limiter.register_failure(KEY, Timestamp(1_000)).is_none());
        assert_eq!(limiter.failures(KEY), 2);
        limiter
            .check(KEY, Timestamp(1_000))
            .expect("aun no bloqueado");
    }

    #[test]
    fn reaching_the_limit_locks_the_key() {
        let mut limiter = AttemptLimiter::new(policy());
        for ms in [0, 1_000, 2_000] {
            limiter.register_failure(KEY, Timestamp(ms));
        }
        let err = limiter.check(KEY, Timestamp(2_000)).unwrap_err();
        assert_eq!(
            err,
            SecurityError::RateLimited {
                retry_after_secs: 10
            }
        );
        assert!(limiter.is_locked(KEY, Timestamp(2_000)));
    }

    #[test]
    fn the_lock_expires_on_its_own() {
        let mut limiter = AttemptLimiter::new(policy());
        for ms in [0, 1_000, 2_000] {
            limiter.register_failure(KEY, Timestamp(ms));
        }
        assert!(limiter.is_locked(KEY, Timestamp(11_999)));
        assert!(
            !limiter.is_locked(KEY, Timestamp(12_000)),
            "10 s desde el ultimo fallo"
        );
    }

    #[test]
    fn remaining_time_is_rounded_up() {
        let mut limiter = AttemptLimiter::new(policy());
        for ms in [0, 10, 20] {
            limiter.register_failure(KEY, Timestamp(ms));
        }
        // Quedan 9,98 s: nunca debe informarse de 9 ni de 0.
        let err = limiter.check(KEY, Timestamp(40)).unwrap_err();
        assert_eq!(
            err,
            SecurityError::RateLimited {
                retry_after_secs: 10
            }
        );
    }

    #[test]
    fn successive_lockouts_grow_exponentially() {
        let p = policy();
        assert_eq!(p.lockout_for_round(0), Duration::ZERO);
        assert_eq!(p.lockout_for_round(1), Duration::from_secs(10));
        assert_eq!(p.lockout_for_round(2), Duration::from_secs(20));
        assert_eq!(p.lockout_for_round(3), Duration::from_secs(40));
        assert_eq!(p.lockout_for_round(4), Duration::from_secs(80));
        assert_eq!(
            p.lockout_for_round(5),
            Duration::from_secs(80),
            "queda acotado"
        );
        assert_eq!(p.lockout_for_round(999), Duration::from_secs(80));
    }

    #[test]
    fn waiting_out_a_lock_does_not_reset_the_escalation() {
        let mut limiter = AttemptLimiter::new(policy());
        let mut now = 0_u64;

        // Primera tanda: bloqueo de 10 s.
        for _ in 0..3 {
            now += 100;
            limiter.register_failure(KEY, Timestamp(now));
        }
        assert!(limiter.is_locked(KEY, Timestamp(now)));

        // Se espera a que expire y se vuelve a fallar: el bloqueo es mayor.
        now += 11_000;
        let mut second_lockout = None;
        for _ in 0..3 {
            now += 100;
            second_lockout = limiter
                .register_failure(KEY, Timestamp(now))
                .or(second_lockout);
        }
        assert_eq!(
            second_lockout,
            Some(Duration::from_secs(20)),
            "la segunda tanda debe costar el doble"
        );
    }

    #[test]
    fn a_success_clears_the_history() {
        let mut limiter = AttemptLimiter::new(policy());
        limiter.register_failure(KEY, Timestamp(0));
        limiter.register_failure(KEY, Timestamp(100));
        assert_eq!(limiter.failures(KEY), 2);

        limiter.register_success(KEY);
        assert_eq!(limiter.failures(KEY), 0);
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn isolated_failures_do_not_accumulate_forever() {
        let mut limiter = AttemptLimiter::new(policy());
        // Dos fallos separados por mas que la ventana no suman.
        limiter.register_failure(KEY, Timestamp(0));
        limiter.register_failure(KEY, Timestamp(61_000));
        assert_eq!(limiter.failures(KEY), 1, "el contador se reinicio");
        assert!(!limiter.is_locked(KEY, Timestamp(61_000)));
    }

    #[test]
    fn keys_are_independent() {
        let mut limiter = AttemptLimiter::new(policy());
        for _ in 0..3 {
            limiter.register_failure("atacante", Timestamp(0));
        }
        assert!(limiter.is_locked("atacante", Timestamp(0)));
        assert!(
            !limiter.is_locked("mi-ipad", Timestamp(0)),
            "no debe salpicar a otros"
        );
    }

    #[test]
    fn rotating_keys_does_not_grant_free_attempts() {
        let mut limiter = AttemptLimiter {
            max_keys: 2,
            ..AttemptLimiter::new(policy())
        };
        limiter.register_failure("a", Timestamp(0));
        limiter.register_failure("b", Timestamp(0));

        // Con la tabla llena, una clave nueva recibe bloqueo directamente.
        let lockout = limiter.register_failure("c", Timestamp(0));
        assert_eq!(lockout, Some(Duration::from_secs(10)));
        assert_eq!(limiter.tracked_keys(), 2, "no crece por encima del tope");
    }

    #[test]
    fn pruning_forgets_old_entries() {
        let mut limiter = AttemptLimiter::new(policy());
        limiter.register_failure(KEY, Timestamp(0));
        assert_eq!(limiter.tracked_keys(), 1);

        // Mas alla de la ventana mas el bloqueo maximo, el historial se olvida.
        limiter.prune(Timestamp(60_000 + 80_000 + 1));
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn the_pairing_policy_is_stricter_than_device_auth() {
        let pairing = RateLimitPolicy::pairing();
        let device = RateLimitPolicy::device_auth();
        assert!(pairing.max_failures < device.max_failures);
        assert!(pairing.base_lockout > device.base_lockout);
    }
}
