//! Permisos concedidos a un dispositivo autorizado.
//!
//! El modelo es una lista blanca explicita: lo que no se concede, no se puede
//! hacer. Las operaciones destructivas (reiniciar, apagar) exigen ademas una
//! confirmacion interactiva en el host, aunque el permiso este concedido.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Capacidad concreta que un dispositivo puede ejercer sobre el host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Ver la pantalla del host.
    ViewScreen,
    /// Mover el raton y pulsar botones.
    ControlPointer,
    /// Enviar pulsaciones de teclado.
    ControlKeyboard,
    /// Recibir el audio del host.
    ReceiveAudio,
    /// Leer el portapapeles del host.
    ClipboardRead,
    /// Escribir en el portapapeles del host.
    ClipboardWrite,
    /// Listar directorios y descargar archivos.
    FileRead,
    /// Subir y sobrescribir archivos.
    FileWrite,
    /// Consultar informacion del sistema.
    SystemInfo,
    /// Bloquear la sesion de Windows.
    LockWorkstation,
    /// Reiniciar el equipo.
    Reboot,
    /// Apagar el equipo.
    Shutdown,
    /// Conectarse sin que nadie acepte la sesion en el host.
    UnattendedAccess,
}

impl Permission {
    /// Todos los permisos existentes, en orden estable.
    pub const ALL: [Self; 13] = [
        Self::ViewScreen,
        Self::ControlPointer,
        Self::ControlKeyboard,
        Self::ReceiveAudio,
        Self::ClipboardRead,
        Self::ClipboardWrite,
        Self::FileRead,
        Self::FileWrite,
        Self::SystemInfo,
        Self::LockWorkstation,
        Self::Reboot,
        Self::Shutdown,
        Self::UnattendedAccess,
    ];

    /// Nombre estable para mensajes de error y registros.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ViewScreen => "view_screen",
            Self::ControlPointer => "control_pointer",
            Self::ControlKeyboard => "control_keyboard",
            Self::ReceiveAudio => "receive_audio",
            Self::ClipboardRead => "clipboard_read",
            Self::ClipboardWrite => "clipboard_write",
            Self::FileRead => "file_read",
            Self::FileWrite => "file_write",
            Self::SystemInfo => "system_info",
            Self::LockWorkstation => "lock_workstation",
            Self::Reboot => "reboot",
            Self::Shutdown => "shutdown",
            Self::UnattendedAccess => "unattended_access",
        }
    }

    /// Indica si la operacion exige confirmacion explicita en el host.
    ///
    /// Reiniciar o apagar interrumpe el trabajo de quien este delante del PC y
    /// corta la propia sesion remota, asi que nunca se ejecuta en silencio.
    #[must_use]
    pub const fn requires_host_confirmation(self) -> bool {
        matches!(self, Self::Reboot | Self::Shutdown)
    }

    /// Indica si conceder el permiso amplia de forma notable la superficie de riesgo.
    ///
    /// La interfaz marca estos permisos en la pantalla de seguridad.
    #[must_use]
    pub const fn is_sensitive(self) -> bool {
        matches!(
            self,
            Self::FileWrite
                | Self::Reboot
                | Self::Shutdown
                | Self::UnattendedAccess
                | Self::ControlKeyboard
        )
    }
}

/// Conjunto de permisos concedidos.
///
/// Se usa `BTreeSet` para que la serializacion sea deterministica: eso hace que
/// los registros de auditoria y las pruebas sean estables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionSet(BTreeSet<Permission>);

impl PermissionSet {
    /// Conjunto vacio: el dispositivo no puede hacer nada.
    #[must_use]
    pub const fn empty() -> Self {
        Self(BTreeSet::new())
    }

    /// Solo visualizacion, sin control ni datos.
    #[must_use]
    pub fn view_only() -> Self {
        Self::from_iter([Permission::ViewScreen, Permission::SystemInfo])
    }

    /// Perfil de uso habitual: control completo de escritorio sin administracion.
    #[must_use]
    pub fn standard() -> Self {
        Self::from_iter([
            Permission::ViewScreen,
            Permission::ControlPointer,
            Permission::ControlKeyboard,
            Permission::ReceiveAudio,
            Permission::ClipboardRead,
            Permission::ClipboardWrite,
            Permission::FileRead,
            Permission::SystemInfo,
        ])
    }

    /// Todos los permisos. Reservado para el dispositivo principal.
    #[must_use]
    pub fn full() -> Self {
        Self::from_iter(Permission::ALL)
    }

    /// Comprueba si el permiso esta concedido.
    #[must_use]
    pub fn has(&self, permission: Permission) -> bool {
        self.0.contains(&permission)
    }

    /// Exige el permiso y devuelve error si falta.
    ///
    /// # Errors
    ///
    /// [`CoreError::PermissionDenied`] nombrando el permiso que falta.
    pub fn require(&self, permission: Permission) -> CoreResult<()> {
        if self.has(permission) {
            Ok(())
        } else {
            Err(CoreError::PermissionDenied {
                required: permission.name(),
            })
        }
    }

    /// Concede un permiso. Devuelve `true` si no lo tenia antes.
    pub fn grant(&mut self, permission: Permission) -> bool {
        self.0.insert(permission)
    }

    /// Retira un permiso. Devuelve `true` si lo tenia.
    pub fn revoke(&mut self, permission: Permission) -> bool {
        self.0.remove(&permission)
    }

    /// Numero de permisos concedidos.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Indica si no hay ningun permiso concedido.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Itera los permisos en orden estable.
    pub fn iter(&self) -> impl Iterator<Item = Permission> + '_ {
        self.0.iter().copied()
    }

    /// Interseccion con otro conjunto: nunca concede mas de lo que ya habia.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        Self(self.0.intersection(&other.0).copied().collect())
    }
}

impl Default for PermissionSet {
    fn default() -> Self {
        Self::view_only()
    }
}

impl FromIterator<Permission> for PermissionSet {
    fn from_iter<T: IntoIterator<Item = Permission>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_denies_everything() {
        let set = PermissionSet::empty();
        for permission in Permission::ALL {
            assert!(
                !set.has(permission),
                "{} no deberia estar",
                permission.name()
            );
        }
        assert!(set.is_empty());
    }

    #[test]
    fn view_only_cannot_control_or_write() {
        let set = PermissionSet::view_only();
        assert!(set.has(Permission::ViewScreen));
        assert!(!set.has(Permission::ControlPointer));
        assert!(!set.has(Permission::FileWrite));
        assert!(!set.has(Permission::UnattendedAccess));
    }

    #[test]
    fn standard_profile_excludes_power_and_unattended() {
        let set = PermissionSet::standard();
        assert!(set.has(Permission::ControlKeyboard));
        assert!(!set.has(Permission::Reboot));
        assert!(!set.has(Permission::Shutdown));
        assert!(!set.has(Permission::UnattendedAccess));
        assert!(!set.has(Permission::FileWrite));
    }

    #[test]
    fn full_profile_contains_every_permission() {
        let set = PermissionSet::full();
        assert_eq!(set.len(), Permission::ALL.len());
    }

    #[test]
    fn require_reports_the_missing_permission() {
        let set = PermissionSet::view_only();
        let err = set.require(Permission::Shutdown).unwrap_err();
        assert_eq!(
            err,
            CoreError::PermissionDenied {
                required: "shutdown"
            }
        );
    }

    #[test]
    fn grant_and_revoke_report_changes() {
        let mut set = PermissionSet::empty();
        assert!(set.grant(Permission::ViewScreen));
        assert!(!set.grant(Permission::ViewScreen));
        assert!(set.revoke(Permission::ViewScreen));
        assert!(!set.revoke(Permission::ViewScreen));
    }

    #[test]
    fn intersect_never_escalates() {
        let granted = PermissionSet::standard();
        let requested = PermissionSet::full();
        let effective = granted.intersect(&requested);
        assert_eq!(effective, granted);
        assert!(!effective.has(Permission::Shutdown));
    }

    #[test]
    fn destructive_permissions_need_confirmation() {
        assert!(Permission::Reboot.requires_host_confirmation());
        assert!(Permission::Shutdown.requires_host_confirmation());
        assert!(!Permission::ViewScreen.requires_host_confirmation());
        // Bloquear es reversible y no destruye trabajo: no exige confirmacion.
        assert!(!Permission::LockWorkstation.requires_host_confirmation());
    }

    #[test]
    fn serialization_is_deterministic() {
        let set = PermissionSet::view_only();
        let first = serde_json::to_string(&set).expect("serializa");
        let second = serde_json::to_string(&set).expect("serializa");
        assert_eq!(first, second);
        let back: PermissionSet = serde_json::from_str(&first).expect("deserializa");
        assert_eq!(set, back);
    }
}
