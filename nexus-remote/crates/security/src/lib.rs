//! # nexus-security
//!
//! Identidad criptografica, emparejamiento, autenticacion mutua y defensas
//! contra reenvio y fuerza bruta.
//!
//! No contiene criptografia propia: usa `ed25519-dalek` para firmas, `argon2`
//! para derivar los codigos de emparejamiento, `sha2` para huellas y `subtle`
//! para comparaciones en tiempo constante.

#![forbid(unsafe_code)]

pub mod binding;
pub mod challenge;
pub mod error;
pub mod identity;
pub mod pairing;
pub mod ratelimit;
pub mod registry;
pub mod replay;
pub mod tokens;

pub use binding::{BindingProof, DtlsFingerprint, FingerprintAlgorithm, SessionBinding};
pub use challenge::{AuthRole, AuthTranscript, Challenge, ChallengeStore, Nonce};
pub use error::{SecurityError, SecurityResult};
pub use identity::{
    DeviceIdentity, DevicePublicKey, PUBLIC_KEY_LEN, SECRET_KEY_LEN, SIGNATURE_LEN,
};
pub use pairing::{PairingCode, PairingInvite, PairingSession, PairingState};
pub use ratelimit::{AttemptLimiter, RateLimitPolicy};
pub use registry::{
    AuthOutcome, AuthRequest, Authenticator, DeviceRecord, DeviceRegistry, InMemoryRegistry,
};
pub use replay::ReplayGuard;
pub use tokens::secret_matches;
