//! Atadura de la identidad del dispositivo al canal cifrado de la sesion.
//!
//! # El problema que resuelve
//!
//! El servidor de senalizacion reenvia las descripciones SDP entre los dos
//! dispositivos, y una descripcion SDP contiene la **huella del certificado
//! DTLS** del emisor. Esa huella es lo que cada extremo usa para comprobar que
//! esta hablando por DTLS con quien cree.
//!
//! Un servidor de senalizacion comprometido podria sustituir esa huella por la
//! suya, abrir dos sesiones DTLS (una con cada extremo) y leer todo el trafico:
//! la pantalla, las pulsaciones de teclado, los archivos. El cifrado seguiria
//! activo y los dos extremos no notarian nada. Es el ataque clasico de hombre
//! en el medio sobre WebRTC, y no lo evita el TLS del WebSocket, porque el
//! atacante en este escenario es el propio servidor.
//!
//! # Como se evita
//!
//! Despues de intercambiar oferta y respuesta, cada extremo firma con su clave
//! Ed25519 una transcripcion que incluye **las dos huellas DTLS** de la
//! sesion, los dos identificadores de dispositivo y el identificador de sesion.
//! Al recibir la firma del otro extremo se comprueba que cubre exactamente la
//! huella que se negocio de verdad.
//!
//! Si el servidor sustituye una huella, la firma deja de cuadrar y la sesion se
//! aborta con [`SecurityError::FingerprintMismatch`]. El servidor no puede
//! forjar la firma porque no tiene las claves privadas, que no salen nunca de
//! los dispositivos.
//!
//! # Frescura
//!
//! No hace falta un nonce adicional: la biblioteca WebRTC genera un certificado
//! DTLS nuevo por conexion, de modo que el par de huellas ya es distinto en
//! cada sesion. Reproducir una firma antigua obligaria al atacante a presentar
//! el certificado antiguo, y para completar el DTLS necesitaria su clave
//! privada, que no tiene.
//!
//! # Lo que esto no cubre
//!
//! Esta comprobacion garantiza que el canal DTLS va contra el dispositivo que
//! posee la clave esperada. No sustituye al emparejamiento: quien decide que
//! una clave es de confianza es el host, en su registro de dispositivos.

use std::fmt;

use nexus_core::{DeviceId, SessionId};

use crate::challenge::AuthRole;
use crate::error::{SecurityError, SecurityResult};
use crate::identity::{DeviceIdentity, DevicePublicKey};

/// Dominio de separacion de las firmas de atadura de sesion.
///
/// Que sea distinto del dominio de autenticacion
/// ([`AUTH_DOMAIN`](crate::challenge::AUTH_DOMAIN)) impide que una firma hecha
/// para un proposito valga para el otro.
pub const BINDING_DOMAIN: &[u8] = b"NEXUS-REMOTE/v1/session-binding";

/// Funcion de resumen con la que se calcula una huella DTLS.
///
/// `SHA-1` y `MD5` quedan fuera a proposito: aunque RFC 4572 los admite, estan
/// descartados para uso criptografico y aceptarlos abriria la puerta a un
/// atacante que buscase una colision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FingerprintAlgorithm {
    /// `SHA-256`, la que usan las bibliotecas WebRTC por omision.
    Sha256,
    /// `SHA-384`.
    Sha384,
    /// `SHA-512`.
    Sha512,
}

impl FingerprintAlgorithm {
    /// Nombre tal como aparece en el SDP.
    #[must_use]
    pub const fn sdp_name(self) -> &'static str {
        match self {
            Self::Sha256 => "sha-256",
            Self::Sha384 => "sha-384",
            Self::Sha512 => "sha-512",
        }
    }

    /// Longitud en bytes del resumen.
    #[must_use]
    pub const fn digest_len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }

    /// Byte que identifica el algoritmo dentro de la transcripcion firmada.
    const fn tag(self) -> u8 {
        match self {
            Self::Sha256 => 0x01,
            Self::Sha384 => 0x02,
            Self::Sha512 => 0x03,
        }
    }

    /// Interpreta el nombre que aparece en el SDP.
    ///
    /// # Errors
    ///
    /// [`SecurityError::InvalidFingerprint`] si el nombre no corresponde a un
    /// algoritmo admitido.
    pub fn from_sdp_name(name: &str) -> SecurityResult<Self> {
        match name.to_ascii_lowercase().as_str() {
            "sha-256" => Ok(Self::Sha256),
            "sha-384" => Ok(Self::Sha384),
            "sha-512" => Ok(Self::Sha512),
            other => Err(SecurityError::InvalidFingerprint(format!(
                "algoritmo no admitido: {other}"
            ))),
        }
    }
}

/// Huella del certificado DTLS de un extremo.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DtlsFingerprint {
    algorithm: FingerprintAlgorithm,
    digest: Vec<u8>,
}

impl DtlsFingerprint {
    /// Interpreta el valor de un atributo `a=fingerprint` del SDP.
    ///
    /// El formato es el de RFC 4572: el nombre del algoritmo, un espacio y el
    /// resumen en hexadecimal separado por dos puntos. Se admiten mayusculas y
    /// minusculas.
    ///
    /// # Errors
    ///
    /// [`SecurityError::InvalidFingerprint`] si falta el algoritmo, si el
    /// algoritmo no esta admitido, si algun byte no es hexadecimal o si la
    /// longitud no corresponde al algoritmo declarado.
    pub fn parse(value: &str) -> SecurityResult<Self> {
        let trimmed = value.trim();
        let (name, digits) = trimmed.split_once(char::is_whitespace).ok_or_else(|| {
            SecurityError::InvalidFingerprint(
                "falta el separador entre algoritmo y resumen".to_owned(),
            )
        })?;

        let algorithm = FingerprintAlgorithm::from_sdp_name(name)?;

        let mut digest = Vec::with_capacity(algorithm.digest_len());
        for part in digits.trim().split(':') {
            if part.len() != 2 {
                return Err(SecurityError::InvalidFingerprint(format!(
                    "los bytes deben ir en pares hexadecimales separados por dos puntos: {part:?}"
                )));
            }
            let byte = u8::from_str_radix(part, 16).map_err(|_| {
                SecurityError::InvalidFingerprint(format!("byte no hexadecimal: {part:?}"))
            })?;
            digest.push(byte);
        }

        if digest.len() != algorithm.digest_len() {
            return Err(SecurityError::InvalidFingerprint(format!(
                "{} necesita {} bytes y llegaron {}",
                algorithm.sdp_name(),
                algorithm.digest_len(),
                digest.len()
            )));
        }

        Ok(Self { algorithm, digest })
    }

    /// Extrae la huella de una descripcion SDP completa.
    ///
    /// Una descripcion SDP puede llevar el atributo a nivel de sesion y
    /// repetido en cada linea de medio. Aqui se exige que **todas** las
    /// apariciones coincidan: si no lo hacen, la descripcion se rechaza. Esto
    /// no es un purismo de formato, es una defensa: admitir la primera huella e
    /// ignorar el resto permitiria a un atacante colar una segunda huella y
    /// aprovechar que cada extremo mire una distinta.
    ///
    /// # Errors
    ///
    /// - [`SecurityError::InvalidFingerprint`] si no hay ningun atributo
    ///   `a=fingerprint`, si alguno esta mal formado, o si hay varios que no
    ///   coinciden entre si.
    pub fn from_sdp(sdp: &str) -> SecurityResult<Self> {
        let mut found: Option<Self> = None;

        for line in sdp.lines() {
            let line = line.trim_end_matches('\r');
            let Some(value) = line.strip_prefix("a=fingerprint:") else {
                continue;
            };
            let parsed = Self::parse(value)?;
            match &found {
                None => found = Some(parsed),
                Some(previous) if *previous == parsed => {}
                Some(previous) => {
                    return Err(SecurityError::InvalidFingerprint(format!(
                        "la descripcion declara dos huellas distintas: {previous} y {parsed}"
                    )));
                }
            }
        }

        found.ok_or_else(|| {
            SecurityError::InvalidFingerprint(
                "la descripcion SDP no declara ninguna huella DTLS".to_owned(),
            )
        })
    }

    /// Algoritmo con el que se calculo.
    #[must_use]
    pub const fn algorithm(&self) -> FingerprintAlgorithm {
        self.algorithm
    }

    /// Bytes del resumen.
    #[must_use]
    pub fn digest(&self) -> &[u8] {
        &self.digest
    }

    /// Representacion canonica, en el mismo formato que el SDP y en mayusculas.
    #[must_use]
    pub fn canonical(&self) -> String {
        use fmt::Write as _;
        let hex = self.digest.iter().enumerate().fold(
            String::with_capacity(self.digest.len() * 3),
            |mut out, (index, byte)| {
                if index > 0 {
                    out.push(':');
                }
                let _ = write!(out, "{byte:02X}");
                out
            },
        );
        format!("{} {hex}", self.algorithm.sdp_name())
    }

    /// Bytes de la huella dentro de la transcripcion firmada.
    ///
    /// Van con etiqueta de algoritmo y longitud explicita para que dos huellas
    /// de algoritmos distintos no puedan producir la misma secuencia.
    fn transcript_bytes(&self, out: &mut Vec<u8>) {
        out.push(self.algorithm.tag());
        out.push(self.digest.len() as u8);
        out.extend_from_slice(&self.digest);
    }
}

impl fmt::Display for DtlsFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

impl fmt::Debug for DtlsFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DtlsFingerprint({})", self.canonical())
    }
}

/// Prueba que un extremo envia al otro para cerrar la atadura.
///
/// Viaja por el canal de senalizacion, que el servidor puede leer. No contiene
/// nada secreto: la huella y el identificador son publicos, y la firma solo
/// sirve para la combinacion exacta de esta sesion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingProof {
    /// Dispositivo que firma.
    pub device_id: DeviceId,
    /// Huella DTLS que el firmante dice tener, en formato canonico.
    pub fingerprint: String,
    /// Firma Ed25519 en hexadecimal.
    pub signature_hex: String,
}

/// Transcripcion que ata la sesion al canal cifrado, vista por un extremo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBinding {
    /// Sesion a la que corresponde.
    pub session_id: SessionId,
    /// Papel del extremo que tiene esta vista.
    pub role: AuthRole,
    /// Dispositivo local.
    pub local_device: DeviceId,
    /// Dispositivo remoto.
    pub remote_device: DeviceId,
    /// Huella DTLS local, tomada de la descripcion local.
    pub local_fingerprint: DtlsFingerprint,
    /// Huella DTLS remota, tomada de la descripcion remota recibida.
    pub remote_fingerprint: DtlsFingerprint,
}

impl SessionBinding {
    /// La misma sesion vista desde el otro extremo.
    ///
    /// Se intercambian los papeles, los dispositivos y las huellas. Es lo que
    /// el otro extremo tuvo que firmar, y por eso una firma no vale reflejada:
    /// el byte de papel y el orden de los campos son distintos.
    #[must_use]
    pub fn counterpart(&self) -> Self {
        Self {
            session_id: self.session_id,
            role: self.role.counterpart(),
            local_device: self.remote_device,
            remote_device: self.local_device,
            local_fingerprint: self.remote_fingerprint.clone(),
            remote_fingerprint: self.local_fingerprint.clone(),
        }
    }

    /// Serializa la transcripcion a los bytes que se firman.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BINDING_DOMAIN.len() + 128);
        out.extend_from_slice(BINDING_DOMAIN);
        out.push(0x00);
        out.push(self.role.tag());
        out.extend_from_slice(&nexus_core::PROTOCOL_VERSION.to_le_bytes());
        out.extend_from_slice(self.session_id.as_uuid().as_bytes());
        out.extend_from_slice(self.local_device.as_uuid().as_bytes());
        out.extend_from_slice(self.remote_device.as_uuid().as_bytes());
        self.local_fingerprint.transcript_bytes(&mut out);
        self.remote_fingerprint.transcript_bytes(&mut out);
        out
    }

    /// Firma la transcripcion y construye la prueba que se envia al otro extremo.
    #[must_use]
    pub fn prove(&self, identity: &DeviceIdentity) -> BindingProof {
        BindingProof {
            device_id: identity.device_id(),
            fingerprint: self.local_fingerprint.canonical(),
            signature_hex: identity.sign_hex(&self.to_bytes()),
        }
    }

    /// Comprueba la prueba recibida del otro extremo.
    ///
    /// Las comprobaciones van en este orden:
    ///
    /// 1. Que la prueba venga del dispositivo que se espera.
    /// 2. Que la huella firmada sea **exactamente** la que se negocio por DTLS.
    ///    Aqui es donde se detecta la sustitucion de huella.
    /// 3. Que la firma sea valida sobre la transcripcion contraria.
    ///
    /// # Errors
    ///
    /// - [`SecurityError::UnknownDevice`] si la prueba la firma otro dispositivo.
    /// - [`SecurityError::FingerprintMismatch`] si la huella firmada no es la
    ///   negociada, lo que indica interceptacion.
    /// - [`SecurityError::InvalidFingerprint`] si la huella viene mal formada.
    /// - [`SecurityError::InvalidSignature`] si la firma no cuadra.
    pub fn verify_peer(
        &self,
        proof: &BindingProof,
        peer_key: &DevicePublicKey,
    ) -> SecurityResult<()> {
        if proof.device_id != self.remote_device {
            return Err(SecurityError::UnknownDevice);
        }

        let claimed = DtlsFingerprint::parse(&proof.fingerprint)?;
        if claimed != self.remote_fingerprint {
            return Err(SecurityError::FingerprintMismatch);
        }

        peer_key.verify_hex(&self.counterpart().to_bytes(), &proof.signature_hex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST_FP: &str = "sha-256 \
        11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:\
        11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00";
    const CONTROLLER_FP: &str = "sha-256 \
        AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:\
        AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99";
    const ATTACKER_FP: &str = "sha-256 \
        DE:AD:BE:EF:DE:AD:BE:EF:DE:AD:BE:EF:DE:AD:BE:EF:\
        DE:AD:BE:EF:DE:AD:BE:EF:DE:AD:BE:EF:DE:AD:BE:EF";

    struct Pair {
        host: DeviceIdentity,
        controller: DeviceIdentity,
        session_id: SessionId,
    }

    impl Pair {
        fn new() -> Self {
            Self {
                host: DeviceIdentity::generate(DeviceId::new()),
                controller: DeviceIdentity::generate(DeviceId::new()),
                session_id: SessionId::new(),
            }
        }

        /// Vista del host cuando la negociacion fue limpia.
        fn host_view(&self) -> SessionBinding {
            SessionBinding {
                session_id: self.session_id,
                role: AuthRole::Host,
                local_device: self.host.device_id(),
                remote_device: self.controller.device_id(),
                local_fingerprint: DtlsFingerprint::parse(HOST_FP).expect("huella valida"),
                remote_fingerprint: DtlsFingerprint::parse(CONTROLLER_FP).expect("huella valida"),
            }
        }

        /// Vista del controlador cuando la negociacion fue limpia.
        fn controller_view(&self) -> SessionBinding {
            self.host_view().counterpart()
        }
    }

    #[test]
    fn a_clean_negotiation_authenticates_both_ends() {
        let pair = Pair::new();
        let host_view = pair.host_view();
        let controller_view = pair.controller_view();

        let host_proof = host_view.prove(&pair.host);
        let controller_proof = controller_view.prove(&pair.controller);

        controller_view
            .verify_peer(&host_proof, &pair.host.public_key())
            .expect("el controlador debe autenticar al host");
        host_view
            .verify_peer(&controller_proof, &pair.controller.public_key())
            .expect("el host debe autenticar al controlador");
    }

    #[test]
    fn a_swapped_fingerprint_is_detected() {
        // El escenario completo: el servidor de senalizacion reenvia la firma
        // autentica del host, pero sustituye la huella del SDP por la suya para
        // poder descifrar. El controlador negocia DTLS contra el atacante, de
        // modo que la huella que observa es la del atacante.
        let pair = Pair::new();
        let host_proof = pair.host_view().prove(&pair.host);

        let intercepted = SessionBinding {
            remote_fingerprint: DtlsFingerprint::parse(ATTACKER_FP).expect("huella valida"),
            ..pair.controller_view()
        };

        let err = intercepted
            .verify_peer(&host_proof, &pair.host.public_key())
            .unwrap_err();
        assert_eq!(
            err,
            SecurityError::FingerprintMismatch,
            "sustituir la huella debe abortar la sesion"
        );
    }

    #[test]
    fn an_attacker_cannot_sign_its_own_fingerprint() {
        // Variante del anterior: el atacante no reenvia la firma del host, sino
        // que firma el mismo con una clave propia. Falla por firma invalida,
        // porque el controlador verifica contra la clave que tiene del host.
        let pair = Pair::new();
        let attacker = DeviceIdentity::generate(pair.host.device_id());

        let attacker_view = SessionBinding {
            local_fingerprint: DtlsFingerprint::parse(ATTACKER_FP).expect("huella valida"),
            ..pair.host_view()
        };
        let forged = attacker_view.prove(&attacker);

        let controller_view = SessionBinding {
            remote_fingerprint: DtlsFingerprint::parse(ATTACKER_FP).expect("huella valida"),
            ..pair.controller_view()
        };

        let err = controller_view
            .verify_peer(&forged, &pair.host.public_key())
            .unwrap_err();
        assert_eq!(err, SecurityError::InvalidSignature);
    }

    #[test]
    fn a_proof_cannot_be_reflected_back() {
        // Si la transcripcion fuese simetrica, el servidor podria devolver al
        // host su propia firma y hacerle creer que el controlador contesto. El
        // byte de papel y el orden de los campos lo impiden.
        let pair = Pair::new();
        let host_view = pair.host_view();
        let host_proof = host_view.prove(&pair.host);

        let err = host_view
            .verify_peer(&host_proof, &pair.host.public_key())
            .unwrap_err();
        // La prueba viene del propio host, que no es el dispositivo remoto.
        assert_eq!(err, SecurityError::UnknownDevice);
    }

    #[test]
    fn the_role_byte_changes_the_signed_bytes() {
        let pair = Pair::new();
        let host_view = pair.host_view();
        let mut same_but_controller = host_view.clone();
        same_but_controller.role = AuthRole::Controller;
        assert_ne!(
            host_view.to_bytes(),
            same_but_controller.to_bytes(),
            "el papel debe formar parte de lo firmado"
        );
    }

    #[test]
    fn a_proof_from_another_session_does_not_transfer() {
        let pair = Pair::new();
        let host_proof = pair.host_view().prove(&pair.host);

        let other_session = SessionBinding {
            session_id: SessionId::new(),
            ..pair.controller_view()
        };

        let err = other_session
            .verify_peer(&host_proof, &pair.host.public_key())
            .unwrap_err();
        assert_eq!(err, SecurityError::InvalidSignature);
    }

    #[test]
    fn a_proof_from_another_device_pair_does_not_transfer() {
        let pair = Pair::new();
        let host_proof = pair.host_view().prove(&pair.host);

        // Mismo host y misma sesion, pero el controlador es otro dispositivo.
        let other_controller = DeviceIdentity::generate(DeviceId::new());
        let mut view = pair.controller_view();
        view.local_device = other_controller.device_id();

        let err = view
            .verify_peer(&host_proof, &pair.host.public_key())
            .unwrap_err();
        assert_eq!(err, SecurityError::InvalidSignature);
    }

    #[test]
    fn a_proof_signed_by_the_wrong_device_id_is_refused() {
        let pair = Pair::new();
        let mut proof = pair.host_view().prove(&pair.host);
        proof.device_id = DeviceId::new();

        let err = pair
            .controller_view()
            .verify_peer(&proof, &pair.host.public_key())
            .unwrap_err();
        assert_eq!(err, SecurityError::UnknownDevice);
    }

    #[test]
    fn the_counterpart_of_the_counterpart_is_the_original() {
        let pair = Pair::new();
        let view = pair.host_view();
        assert_eq!(view.counterpart().counterpart(), view);
    }

    #[test]
    fn the_binding_domain_differs_from_the_auth_domain() {
        assert_ne!(
            BINDING_DOMAIN,
            crate::challenge::AUTH_DOMAIN,
            "una firma de autenticacion no debe valer como atadura de sesion"
        );
    }

    #[test]
    fn an_auth_signature_is_not_accepted_as_a_binding() {
        // Comprobacion efectiva de la separacion de dominios: se firma una
        // transcripcion de autenticacion y se intenta colar como atadura.
        use crate::challenge::{AuthTranscript, Nonce};
        use nexus_core::Timestamp;

        let pair = Pair::new();
        let auth = AuthTranscript::new(
            Nonce::generate(),
            Nonce::generate(),
            pair.host.device_id(),
            AuthRole::Host,
            Timestamp(1_730_000_000_000),
        );
        let proof = BindingProof {
            device_id: pair.host.device_id(),
            fingerprint: DtlsFingerprint::parse(HOST_FP)
                .expect("huella valida")
                .canonical(),
            signature_hex: pair.host.sign_hex(&auth.to_bytes()),
        };

        let err = pair
            .controller_view()
            .verify_peer(&proof, &pair.host.public_key())
            .unwrap_err();
        assert_eq!(err, SecurityError::InvalidSignature);
    }

    // --- interpretacion de huellas ---

    #[test]
    fn a_sha256_fingerprint_parses() {
        let fp = DtlsFingerprint::parse(HOST_FP).expect("valida");
        assert_eq!(fp.algorithm(), FingerprintAlgorithm::Sha256);
        assert_eq!(fp.digest().len(), 32);
    }

    #[test]
    fn parsing_is_case_insensitive_and_canonicalises_to_uppercase() {
        let lower = DtlsFingerprint::parse(&HOST_FP.to_lowercase()).expect("valida");
        let upper = DtlsFingerprint::parse(HOST_FP).expect("valida");
        assert_eq!(lower, upper);
        assert!(
            upper.canonical().contains("11:22:33"),
            "{}",
            upper.canonical()
        );
        assert!(upper.canonical().starts_with("sha-256 "));
    }

    #[test]
    fn sha1_and_md5_fingerprints_are_refused() {
        for name in ["sha-1", "md5", "SHA-1"] {
            let value = format!("{name} {}", "AA:".repeat(19) + "AA");
            let err = DtlsFingerprint::parse(&value).unwrap_err();
            assert!(
                matches!(err, SecurityError::InvalidFingerprint(_)),
                "{name} deberia rechazarse: {err:?}"
            );
        }
    }

    #[test]
    fn a_truncated_fingerprint_is_refused() {
        let err = DtlsFingerprint::parse("sha-256 AA:BB:CC").unwrap_err();
        assert!(
            matches!(err, SecurityError::InvalidFingerprint(_)),
            "{err:?}"
        );
    }

    #[test]
    fn malformed_fingerprints_are_refused() {
        for bad in [
            "",
            "sha-256",
            "sha-256 ",
            "sha-256 ZZ:BB",
            "sha-256 A:B:C",
            "AA:BB:CC",
        ] {
            assert!(
                DtlsFingerprint::parse(bad).is_err(),
                "{bad:?} deberia rechazarse"
            );
        }
    }

    #[test]
    fn a_fingerprint_is_extracted_from_a_real_sdp() {
        // Fragmento con la forma de una oferta real: atributo a nivel de medio.
        let sdp = format!(
            "v=0\r\n\
             o=- 4611731400430051336 2 IN IP4 127.0.0.1\r\n\
             s=-\r\n\
             t=0 0\r\n\
             m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
             c=IN IP4 0.0.0.0\r\n\
             a=ice-ufrag:4ZcD\r\n\
             a=fingerprint:{HOST_FP}\r\n\
             a=setup:actpass\r\n"
        );
        let fp = DtlsFingerprint::from_sdp(&sdp).expect("deberia encontrarla");
        assert_eq!(fp, DtlsFingerprint::parse(HOST_FP).expect("valida"));
    }

    #[test]
    fn repeated_but_identical_fingerprints_are_fine() {
        let sdp = format!("a=fingerprint:{HOST_FP}\r\na=fingerprint:{HOST_FP}\r\n");
        DtlsFingerprint::from_sdp(&sdp).expect("coinciden, no hay problema");
    }

    #[test]
    fn two_different_fingerprints_in_one_sdp_are_refused() {
        // Un atacante podria anadir una segunda huella esperando que cada
        // extremo mire una distinta.
        let sdp = format!("a=fingerprint:{HOST_FP}\r\na=fingerprint:{ATTACKER_FP}\r\n");
        let err = DtlsFingerprint::from_sdp(&sdp).unwrap_err();
        assert!(
            matches!(err, SecurityError::InvalidFingerprint(_)),
            "{err:?}"
        );
    }

    #[test]
    fn an_sdp_without_a_fingerprint_is_refused() {
        let err =
            DtlsFingerprint::from_sdp("v=0\r\nm=application 9 UDP/DTLS/SCTP\r\n").unwrap_err();
        assert!(
            matches!(err, SecurityError::InvalidFingerprint(_)),
            "{err:?}"
        );
    }

    #[test]
    fn the_canonical_form_reparses_to_the_same_value() {
        let fp = DtlsFingerprint::parse(HOST_FP).expect("valida");
        assert_eq!(
            DtlsFingerprint::parse(&fp.canonical()).expect("recarga"),
            fp
        );
    }

    #[test]
    fn different_algorithms_never_share_transcript_bytes() {
        // Dos huellas con el mismo prefijo de bytes pero distinto algoritmo
        // deben producir transcripciones distintas.
        let sha256 = DtlsFingerprint {
            algorithm: FingerprintAlgorithm::Sha256,
            digest: vec![0xAB; 32],
        };
        let sha384 = DtlsFingerprint {
            algorithm: FingerprintAlgorithm::Sha384,
            digest: vec![0xAB; 48],
        };
        let mut a = Vec::new();
        let mut b = Vec::new();
        sha256.transcript_bytes(&mut a);
        sha384.transcript_bytes(&mut b);
        assert_ne!(a, b);
        assert_ne!(a[0], b[0], "la etiqueta de algoritmo debe diferir");
    }
}
