//! Descripcion de los dispositivos que participan en una sesion.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::DeviceId;
use crate::permissions::PermissionSet;
use crate::time::Timestamp;

/// Papel que juega un dispositivo en el sistema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    /// Comparte su pantalla y acepta control (el PC Windows).
    Host,
    /// Visualiza y controla al host (iPhone o iPad).
    Controller,
}

/// Plataforma concreta del dispositivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    /// PC con Windows.
    Windows,
    /// iPhone.
    Ios,
    /// iPad o iPad Pro.
    IpadOs,
    /// Plataforma usada solo para desarrollo y pruebas.
    Linux,
    /// Plataforma no reconocida por esta version.
    #[serde(other)]
    Unknown,
}

impl DevicePlatform {
    /// Papel natural de la plataforma.
    #[must_use]
    pub const fn default_role(self) -> DeviceRole {
        match self {
            Self::Windows | Self::Linux => DeviceRole::Host,
            Self::Ios | Self::IpadOs | Self::Unknown => DeviceRole::Controller,
        }
    }

    /// Nombre legible para la interfaz.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Ios => "iPhone",
            Self::IpadOs => "iPad",
            Self::Linux => "Linux",
            Self::Unknown => "Desconocido",
        }
    }
}

/// Datos que un dispositivo anuncia sobre si mismo.
///
/// Es informacion declarada por el par y por tanto **no** es de confianza: la
/// unica prueba de identidad es la firma criptografica del handshake. Se guarda
/// por comodidad de la interfaz, no para tomar decisiones de seguridad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Identificador estable de la instalacion.
    pub id: DeviceId,
    /// Nombre elegido por la persona ("iPhone de Carlos").
    pub display_name: String,
    /// Plataforma declarada.
    pub platform: DevicePlatform,
    /// Papel declarado.
    pub role: DeviceRole,
    /// Version del sistema operativo, informativa.
    pub os_version: String,
    /// Version de la aplicacion NEXUS instalada.
    pub app_version: String,
}

impl DeviceInfo {
    /// Longitud maxima admitida para el nombre visible.
    pub const MAX_NAME_LEN: usize = 64;

    /// Recorta y limpia el nombre visible declarado por el par.
    ///
    /// Un nombre llega desde la red, asi que se normaliza antes de mostrarlo o
    /// almacenarlo: se eliminan controles, se colapsan espacios y se limita la
    /// longitud. Si queda vacio se sustituye por la etiqueta de plataforma.
    #[must_use]
    pub fn sanitized_name(&self) -> String {
        let cleaned: String = self
            .display_name
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        if cleaned.is_empty() {
            return self.platform.label().to_owned();
        }

        cleaned.chars().take(Self::MAX_NAME_LEN).collect()
    }
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.sanitized_name(), self.platform.label())
    }
}

/// Dispositivo que ha completado el emparejamiento y figura en la lista blanca.
///
/// Esta estructura es la fuente de verdad de la autorizacion: si un dispositivo
/// no esta aqui, o esta marcado como revocado, no puede abrir sesion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizedDevice {
    /// Datos declarados por el dispositivo.
    pub info: DeviceInfo,
    /// Clave publica Ed25519 en hexadecimal (64 caracteres).
    pub public_key_hex: String,
    /// Instante en que se completo el emparejamiento.
    pub paired_at: Timestamp,
    /// Ultima vez que se vio conectado.
    pub last_seen: Option<Timestamp>,
    /// Permisos concedidos a este dispositivo.
    pub permissions: PermissionSet,
    /// Si esta revocado, instante de la revocacion.
    pub revoked_at: Option<Timestamp>,
    /// Si puede conectarse sin que haya nadie delante del host.
    pub unattended_access: bool,
}

impl AuthorizedDevice {
    /// Indica si el dispositivo puede iniciar sesion ahora mismo.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }

    /// Huella corta de la clave publica, para que la persona la compare a ojo.
    ///
    /// Formato `AB:CD:EF:01` usando los primeros bytes de la clave. No sustituye
    /// a la verificacion criptografica, solo ayuda a detectar un cambio.
    #[must_use]
    pub fn short_fingerprint(&self) -> String {
        self.public_key_hex
            .chars()
            .take(8)
            .collect::<Vec<_>>()
            .chunks(2)
            .map(|pair| pair.iter().collect::<String>().to_uppercase())
            .collect::<Vec<_>>()
            .join(":")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_info(name: &str) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId::new(),
            display_name: name.to_owned(),
            platform: DevicePlatform::Ios,
            role: DeviceRole::Controller,
            os_version: "18.0".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    #[test]
    fn platform_picks_sensible_default_role() {
        assert_eq!(DevicePlatform::Windows.default_role(), DeviceRole::Host);
        assert_eq!(
            DevicePlatform::IpadOs.default_role(),
            DeviceRole::Controller
        );
    }

    #[test]
    fn unknown_platform_deserializes_instead_of_failing() {
        let platform: DevicePlatform =
            serde_json::from_str("\"android\"").expect("variante desconocida es tolerada");
        assert_eq!(platform, DevicePlatform::Unknown);
    }

    #[test]
    fn name_is_trimmed_and_collapsed() {
        let info = sample_info("  iPhone   de\tCarlos  ");
        assert_eq!(info.sanitized_name(), "iPhone de Carlos");
    }

    #[test]
    fn control_characters_are_stripped_from_name() {
        let info = sample_info("iPad\u{0}\u{1b}[31m Pro");
        let name = info.sanitized_name();
        assert!(!name.contains('\u{0}'), "no debe quedar NUL: {name:?}");
        assert!(!name.contains('\u{1b}'), "no debe quedar ESC: {name:?}");
    }

    #[test]
    fn empty_name_falls_back_to_platform_label() {
        let info = sample_info("   ");
        assert_eq!(info.sanitized_name(), "iPhone");
    }

    #[test]
    fn long_name_is_truncated() {
        let info = sample_info(&"a".repeat(500));
        assert_eq!(
            info.sanitized_name().chars().count(),
            DeviceInfo::MAX_NAME_LEN
        );
    }

    #[test]
    fn revoked_device_is_not_active() {
        let device = AuthorizedDevice {
            info: sample_info("iPad"),
            public_key_hex: "abcdef0123456789".repeat(4),
            paired_at: Timestamp(1),
            last_seen: None,
            permissions: PermissionSet::view_only(),
            revoked_at: Some(Timestamp(2)),
            unattended_access: false,
        };
        assert!(!device.is_active());
        assert_eq!(device.short_fingerprint(), "AB:CD:EF:01");
    }
}
