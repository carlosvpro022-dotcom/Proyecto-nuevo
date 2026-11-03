//! Quien decide si una sesion se acepta, y con que permisos.
//!
//! Es la decision mas importante del host: a partir de aqui alguien puede mover
//! el raton de este PC. Por eso vive en un modulo propio, es sincrona y no sabe
//! nada de redes, de modo que se puede comprobar entera sin montar nada.
//!
//! # Lo que no decide el servidor
//!
//! El servidor de senalizacion pone en contacto a los dos extremos y nada mas.
//! Lo que un dispositivo puede hacer se decide **aqui**, contra el registro
//! local de dispositivos emparejados. Un controlador puede pedir todos los
//! permisos del mundo: se le conceden los que tenga concedidos, y ni uno mas.
//!
//! # El orden de las comprobaciones
//!
//! Importa, y es este:
//!
//! 1. ¿Esta el dispositivo en el registro? Si no, no hay nada que hablar.
//! 2. ¿Esta revocado?
//! 3. ¿Queda algun permiso tras cruzar lo concedido con lo pedido?
//! 4. ¿Sirve de algo la sesion con las capacidades que comparten?
//! 5. ¿Puede entrar sin que nadie acepte en el PC?
//!
//! Las cuatro primeras se resuelven con datos locales. La quinta es la que
//! distingue una sesion desatendida de una que necesita a alguien delante.

use nexus_core::device::DeviceInfo;
use nexus_core::{Capability, CapabilitySet, DeviceId, Permission, PermissionSet};
use nexus_security::DevicePublicKey;
use nexus_security::registry::DeviceRecord;

/// Que hacer con una peticion de sesion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Se acepta, con estos permisos y estas capacidades.
    Accept {
        /// Permisos efectivos: lo concedido cruzado con lo pedido.
        permissions: PermissionSet,
        /// Capacidades que comparten los dos extremos.
        capabilities: CapabilitySet,
        /// Clave publica con la que se autenticara al otro extremo.
        peer_key: Box<DevicePublicKey>,
    },
    /// Hace falta que alguien acepte en el PC.
    ///
    /// El dispositivo esta autorizado, pero no para entrar solo.
    NeedsConfirmation {
        /// Permisos que tendria la sesion si se acepta.
        permissions: PermissionSet,
        /// Capacidades que comparten.
        capabilities: CapabilitySet,
        /// Clave publica del dispositivo.
        peer_key: Box<DevicePublicKey>,
    },
    /// Se rechaza.
    Reject {
        /// Motivo que se envia al otro extremo.
        ///
        /// Es deliberadamente escueto: no distingue "no estas en la lista" de
        /// "estas revocado", porque un dispositivo no autorizado no debe poder
        /// averiguar nada sobre el registro de este PC.
        reason: &'static str,
        /// Motivo detallado, solo para el registro local.
        detail: String,
    },
}

impl Decision {
    /// Motivo escueto si se rechazo.
    #[must_use]
    pub const fn rejection(&self) -> Option<&'static str> {
        match self {
            Self::Reject { reason, .. } => Some(reason),
            _ => None,
        }
    }

    /// Permisos de la sesion, si hay sesion.
    #[must_use]
    pub const fn permissions(&self) -> Option<&PermissionSet> {
        match self {
            Self::Accept { permissions, .. } | Self::NeedsConfirmation { permissions, .. } => {
                Some(permissions)
            }
            Self::Reject { .. } => None,
        }
    }
}

/// Mensaje unico de rechazo hacia el otro extremo.
const RECHAZO: &str = "este equipo no ha autorizado la sesion";

/// Decide que hacer con una peticion de sesion.
///
/// `record` es lo que el registro local sabe del controlador: `None` si no esta
/// emparejado. `host_capabilities` son las que este PC puede ofrecer de verdad.
#[must_use]
pub fn decide(
    record: Option<&DeviceRecord>,
    controller: &DeviceInfo,
    requested_permissions: &PermissionSet,
    controller_capabilities: &CapabilitySet,
    host_capabilities: &CapabilitySet,
) -> Decision {
    let Some(record) = record else {
        return Decision::Reject {
            reason: RECHAZO,
            detail: format!(
                "el dispositivo {} no esta emparejado con este equipo",
                controller.id.short()
            ),
        };
    };

    if !record.is_active() {
        return Decision::Reject {
            reason: RECHAZO,
            detail: format!("el dispositivo {} esta revocado", controller.id.short()),
        };
    }

    // Lo concedido cruzado con lo pedido. Nunca mas de lo concedido, y tampoco
    // mas de lo que el controlador dice querer: si pide solo ver, no se le da
    // el teclado aunque lo tenga concedido.
    let permissions = record.permissions.intersect(requested_permissions);
    if permissions.is_empty() {
        return Decision::Reject {
            reason: RECHAZO,
            detail: format!(
                "no queda ningun permiso tras cruzar lo concedido con lo pedido por {}",
                controller.id.short()
            ),
        };
    }

    let capabilities = host_capabilities.negotiate(controller_capabilities);
    if !capabilities.is_viable() {
        return Decision::Reject {
            reason: "los dos dispositivos no comparten lo necesario para una sesion",
            detail: format!(
                "capacidades negociadas insuficientes con {}",
                controller.id.short()
            ),
        };
    }

    // Ver la pantalla es lo minimo: sin ese permiso la sesion no tiene sentido
    // aunque el dispositivo tenga otros.
    if !permissions.has(Permission::ViewScreen) {
        return Decision::Reject {
            reason: RECHAZO,
            detail: format!(
                "el dispositivo {} no tiene permiso para ver la pantalla",
                controller.id.short()
            ),
        };
    }

    let peer_key = Box::new(record.public_key.clone());
    if record.unattended_access {
        Decision::Accept {
            permissions,
            capabilities,
            peer_key,
        }
    } else {
        Decision::NeedsConfirmation {
            permissions,
            capabilities,
            peer_key,
        }
    }
}

/// Comprueba que la clave que presenta el otro extremo es la emparejada.
///
/// Es la ultima linea: aunque el servidor dijese que el dispositivo es otro, la
/// firma de la atadura de sesion se verifica contra **esta** clave, que sale
/// del registro local y no de la red.
///
/// # Errors
///
/// Devuelve el identificador esperado si no coincide.
pub fn expected_key(
    record: Option<&DeviceRecord>,
    device_id: DeviceId,
) -> Result<DevicePublicKey, DeviceId> {
    match record {
        Some(record) if record.info.id == device_id && record.is_active() => {
            Ok(record.public_key.clone())
        }
        _ => Err(device_id),
    }
}

/// Capacidades que este host puede ofrecer de verdad hoy.
///
/// No se anuncia lo que no esta implementado: si el host dijese que sabe
/// transmitir audio, el controlador lo ofreceria en su interfaz y luego no
/// funcionaria.
#[must_use]
pub fn host_capabilities() -> CapabilitySet {
    let mut set = CapabilitySet::empty();
    set.insert(Capability::ScreenStream);
    set.insert(Capability::MultiMonitor);
    set.insert(Capability::RemoteInput);
    set.insert(Capability::TouchGestures);
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::Timestamp;
    use nexus_core::device::{DevicePlatform, DeviceRole};
    use nexus_security::DeviceIdentity;

    const NOW: Timestamp = Timestamp(1_730_000_000_000);

    fn controller() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId::new(),
            display_name: "iPad Pro".to_owned(),
            platform: DevicePlatform::IpadOs,
            role: DeviceRole::Controller,
            os_version: "18.1".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    fn record(info: &DeviceInfo, permissions: PermissionSet, unattended: bool) -> DeviceRecord {
        DeviceRecord::new(
            info.clone(),
            DeviceIdentity::generate(info.id).public_key(),
            permissions,
            unattended,
            NOW,
        )
    }

    fn controller_capabilities() -> CapabilitySet {
        let mut set = CapabilitySet::empty();
        set.insert(Capability::ScreenStream);
        set.insert(Capability::RemoteInput);
        set.insert(Capability::AudioStream);
        set
    }

    fn decide_with(
        record: Option<&DeviceRecord>,
        info: &DeviceInfo,
        pedido: &PermissionSet,
    ) -> Decision {
        decide(
            record,
            info,
            pedido,
            &controller_capabilities(),
            &host_capabilities(),
        )
    }

    #[test]
    fn un_dispositivo_desconocido_se_rechaza() {
        let info = controller();
        let decision = decide_with(None, &info, &PermissionSet::full());
        assert_eq!(decision.rejection(), Some(RECHAZO));
    }

    #[test]
    fn un_dispositivo_revocado_se_rechaza_igual_que_uno_desconocido() {
        // El motivo que se envia es el mismo a proposito: un dispositivo no
        // autorizado no debe poder averiguar si alguna vez lo estuvo.
        let info = controller();
        let mut registro = record(&info, PermissionSet::full(), true);
        registro.revoked_at = Some(NOW);

        let revocado = decide_with(Some(&registro), &info, &PermissionSet::full());
        let desconocido = decide_with(None, &info, &PermissionSet::full());
        assert_eq!(revocado.rejection(), desconocido.rejection());
    }

    #[test]
    fn el_detalle_del_rechazo_si_distingue_los_casos() {
        // Hacia fuera el motivo es el mismo; en el registro local hace falta
        // saber que paso de verdad.
        let info = controller();
        let mut registro = record(&info, PermissionSet::full(), true);
        registro.revoked_at = Some(NOW);

        let Decision::Reject { detail, .. } =
            decide_with(Some(&registro), &info, &PermissionSet::full())
        else {
            panic!("deberia rechazarse");
        };
        assert!(detail.contains("revocado"), "{detail}");
    }

    #[test]
    fn un_dispositivo_desatendido_entra_solo() {
        let info = controller();
        let registro = record(&info, PermissionSet::standard(), true);
        assert!(matches!(
            decide_with(Some(&registro), &info, &PermissionSet::standard()),
            Decision::Accept { .. }
        ));
    }

    #[test]
    fn sin_acceso_desatendido_hace_falta_aceptar_en_el_pc() {
        let info = controller();
        let registro = record(&info, PermissionSet::standard(), false);
        assert!(matches!(
            decide_with(Some(&registro), &info, &PermissionSet::standard()),
            Decision::NeedsConfirmation { .. }
        ));
    }

    #[test]
    fn nunca_se_conceden_mas_permisos_de_los_concedidos() {
        // El controlador los pide todos; tiene concedido solo ver.
        let info = controller();
        let registro = record(&info, PermissionSet::view_only(), true);

        let decision = decide_with(Some(&registro), &info, &PermissionSet::full());
        let permisos = decision.permissions().expect("hay sesion");
        assert!(permisos.has(Permission::ViewScreen));
        assert!(
            !permisos.has(Permission::ControlPointer),
            "no se puede conceder lo que no esta concedido"
        );
        assert!(!permisos.has(Permission::Shutdown));
    }

    #[test]
    fn tampoco_se_conceden_mas_de_los_pedidos() {
        // Si el controlador solo quiere ver, no se le da el teclado aunque lo
        // tenga concedido: una aplicacion que pide poco debe recibir poco.
        let info = controller();
        let registro = record(&info, PermissionSet::full(), true);

        let decision = decide_with(Some(&registro), &info, &PermissionSet::view_only());
        let permisos = decision.permissions().expect("hay sesion");
        assert!(permisos.has(Permission::ViewScreen));
        assert!(!permisos.has(Permission::ControlKeyboard));
    }

    #[test]
    fn una_sesion_sin_ningun_permiso_se_rechaza() {
        let info = controller();
        let registro = record(&info, PermissionSet::view_only(), true);
        // Pide solo lo que no tiene.
        let mut pedido = PermissionSet::empty();
        pedido.grant(Permission::Shutdown);

        assert_eq!(
            decide_with(Some(&registro), &info, &pedido).rejection(),
            Some(RECHAZO)
        );
    }

    #[test]
    fn una_sesion_sin_permiso_para_ver_la_pantalla_se_rechaza() {
        // Controlar sin ver no es una sesion, es un riesgo.
        let info = controller();
        let mut concedido = PermissionSet::empty();
        concedido.grant(Permission::ControlPointer);
        let registro = record(&info, concedido.clone(), true);

        assert_eq!(
            decide_with(Some(&registro), &info, &concedido).rejection(),
            Some(RECHAZO)
        );
    }

    #[test]
    fn sin_capacidades_compartidas_no_hay_sesion() {
        let info = controller();
        let registro = record(&info, PermissionSet::standard(), true);

        // El controlador no sabe recibir pantalla.
        let mut pobre = CapabilitySet::empty();
        pobre.insert(Capability::ClipboardText);

        let decision = decide(
            Some(&registro),
            &info,
            &PermissionSet::standard(),
            &pobre,
            &host_capabilities(),
        );
        assert!(decision.rejection().is_some());
        assert_ne!(
            decision.rejection(),
            Some(RECHAZO),
            "este caso si se puede explicar: no es una cuestion de autorizacion"
        );
    }

    #[test]
    fn las_capacidades_negociadas_son_la_interseccion() {
        let info = controller();
        let registro = record(&info, PermissionSet::standard(), true);

        let Decision::Accept { capabilities, .. } =
            decide_with(Some(&registro), &info, &PermissionSet::standard())
        else {
            panic!("deberia aceptarse");
        };
        assert!(capabilities.has(Capability::ScreenStream));
        assert!(capabilities.has(Capability::RemoteInput));
        assert!(
            !capabilities.has(Capability::AudioStream),
            "el controlador la pide pero el host no la tiene"
        );
        assert!(
            !capabilities.has(Capability::MultiMonitor),
            "el host la tiene pero el controlador no la pide"
        );
    }

    #[test]
    fn la_clave_sale_del_registro_local_no_de_la_red() {
        let info = controller();
        let registro = record(&info, PermissionSet::standard(), true);

        let Decision::Accept { peer_key, .. } =
            decide_with(Some(&registro), &info, &PermissionSet::standard())
        else {
            panic!("deberia aceptarse");
        };
        assert_eq!(*peer_key, registro.public_key);
    }

    #[test]
    fn la_clave_esperada_se_niega_para_un_dispositivo_que_no_esta() {
        assert!(expected_key(None, DeviceId::new()).is_err());
    }

    #[test]
    fn la_clave_esperada_se_niega_para_un_dispositivo_revocado() {
        let info = controller();
        let mut registro = record(&info, PermissionSet::full(), true);
        registro.revoked_at = Some(NOW);
        assert!(expected_key(Some(&registro), info.id).is_err());
    }

    #[test]
    fn la_clave_esperada_se_niega_si_el_identificador_no_cuadra() {
        // El servidor dice que habla uno y el registro es de otro.
        let info = controller();
        let registro = record(&info, PermissionSet::full(), true);
        assert!(expected_key(Some(&registro), DeviceId::new()).is_err());
    }

    #[test]
    fn el_host_no_anuncia_lo_que_no_tiene() {
        let capacidades = host_capabilities();
        for todavia_no in [
            Capability::AudioStream,
            Capability::FileTransfer,
            Capability::ClipboardText,
            Capability::PowerControl,
        ] {
            assert!(
                !capacidades.has(todavia_no),
                "{todavia_no:?} no esta implementada y no debe anunciarse"
            );
        }
    }
}
