//! Conversion de los datos crudos del sistema a [`MonitorInfo`].
//!
//! Windows describe cada pantalla con un rectangulo del escritorio virtual, un
//! nombre de dispositivo y unos puntos por pulgada. Aqui se convierte todo eso
//! al tipo del protocolo.
//!
//! El modulo **no depende de Windows**: recibe los valores ya leidos y los
//! transforma, de modo que la aritmetica y los casos raros (puntos por pulgada
//! absurdos, rectangulos invertidos, nombres vacios) se prueban en cualquier
//! plataforma. El backend de plataforma solo lee los valores y llama aqui.

use nexus_core::monitor::MonitorInfo;

/// Puntos por pulgada que Windows considera escala 100 %.
pub const BASE_DPI: u32 = 96;

/// Convierte puntos por pulgada a porcentaje de escala.
///
/// Windows usa 96 ppp como 100 %. Los valores habituales son 96, 120, 144, 168
/// y 192, que corresponden a 100 %, 125 %, 150 %, 175 % y 200 %.
///
/// Un valor de cero (que el sistema no deberia devolver, pero devuelve si la
/// llamada falla) se trata como 100 %: es preferible una escala equivocada a
/// una division por cero o a un monitor descartado.
#[must_use]
pub fn dpi_to_scale_percent(dpi: u32) -> u16 {
    if dpi == 0 {
        return 100;
    }
    let percent = (u64::from(dpi) * 100).div_ceil(u64::from(BASE_DPI));
    u16::try_from(percent).unwrap_or(u16::MAX)
}

/// Da un nombre legible a partir del nombre de dispositivo de Windows.
///
/// Windows entrega cadenas como `\\.\DISPLAY1`, que no se pueden enseñar a
/// nadie. Se convierte en `Pantalla 1`. Si el nombre no tiene esa forma, se
/// devuelve limpio tal cual; y si viene vacio, se usa el indice.
#[must_use]
pub fn friendly_name(device_name: &str, index: u8) -> String {
    let cleaned = device_name.trim_end_matches('\0').trim();

    if let Some(number) = cleaned
        .rsplit('\\')
        .next()
        .and_then(|s| s.strip_prefix("DISPLAY"))
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
    {
        return format!("Pantalla {number}");
    }

    if cleaned.is_empty() {
        return format!("Pantalla {}", index + 1);
    }

    cleaned.to_owned()
}

/// Dimensiones de un rectangulo del escritorio virtual.
///
/// Devuelve `None` si el rectangulo esta invertido o es degenerado, que es
/// como se detecta un monitor que el sistema reporto mal.
#[must_use]
pub fn rect_dimensions(left: i32, top: i32, right: i32, bottom: i32) -> Option<(u32, u32)> {
    let width = right.checked_sub(left)?;
    let height = bottom.checked_sub(top)?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some((width.unsigned_abs(), height.unsigned_abs()))
}

/// Datos crudos de una pantalla, tal como los devuelve el sistema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMonitor {
    /// Nombre de dispositivo (`\\.\DISPLAY1`).
    pub device_name: String,
    /// Borde izquierdo en el escritorio virtual.
    pub left: i32,
    /// Borde superior en el escritorio virtual.
    pub top: i32,
    /// Borde derecho, exclusivo.
    pub right: i32,
    /// Borde inferior, exclusivo.
    pub bottom: i32,
    /// Si es el monitor principal.
    pub is_primary: bool,
    /// Puntos por pulgada efectivos.
    pub dpi: u32,
    /// Frecuencia de refresco en hercios, si el sistema la reporta.
    pub refresh_hz: Option<u16>,
}

/// Convierte los datos crudos al tipo del protocolo.
///
/// Devuelve `None` para una pantalla con dimensiones imposibles, que se
/// descarta en lugar de propagarse: un monitor de ancho cero reventaria la
/// captura mas adelante.
#[must_use]
pub fn to_monitor_info(raw: &RawMonitor, index: u8) -> Option<MonitorInfo> {
    let (width, height) = rect_dimensions(raw.left, raw.top, raw.right, raw.bottom)?;

    Some(MonitorInfo {
        index,
        name: friendly_name(&raw.device_name, index),
        width,
        height,
        origin_x: raw.left,
        origin_y: raw.top,
        scale_percent: dpi_to_scale_percent(raw.dpi),
        is_primary: raw.is_primary,
        // Una frecuencia de 0 o 1 Hz significa "el sistema no lo sabe", no que
        // la pantalla vaya a 1 Hz.
        refresh_hz: raw.refresh_hz.filter(|hz| *hz > 1),
    })
}

/// Convierte una lista de pantallas, descartando las imposibles y numerandolas.
///
/// El indice es la posicion en la lista resultante, no la del sistema: asi los
/// indices siempre son consecutivos y el cliente puede fiarse de ellos.
#[must_use]
pub fn to_layout(raws: &[RawMonitor]) -> nexus_core::MonitorLayout {
    let mut monitors = Vec::with_capacity(raws.len());
    for raw in raws {
        let index = u8::try_from(monitors.len()).unwrap_or(u8::MAX);
        if let Some(info) = to_monitor_info(raw, index) {
            monitors.push(info);
        }
    }
    nexus_core::MonitorLayout(monitors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, l: i32, t: i32, r: i32, b: i32, primary: bool) -> RawMonitor {
        RawMonitor {
            device_name: name.to_owned(),
            left: l,
            top: t,
            right: r,
            bottom: b,
            is_primary: primary,
            dpi: 96,
            refresh_hz: Some(60),
        }
    }

    #[test]
    fn las_escalas_habituales_de_windows_salen_exactas() {
        assert_eq!(dpi_to_scale_percent(96), 100);
        assert_eq!(dpi_to_scale_percent(120), 125);
        assert_eq!(dpi_to_scale_percent(144), 150);
        assert_eq!(dpi_to_scale_percent(168), 175);
        assert_eq!(dpi_to_scale_percent(192), 200);
        assert_eq!(dpi_to_scale_percent(240), 250);
        assert_eq!(dpi_to_scale_percent(288), 300);
    }

    #[test]
    fn un_dpi_de_cero_no_divide_ni_descarta_el_monitor() {
        // Es lo que queda si `GetDpiForMonitor` falla: mejor 100 % que nada.
        assert_eq!(dpi_to_scale_percent(0), 100);
    }

    #[test]
    fn un_dpi_absurdo_no_desborda() {
        assert_eq!(dpi_to_scale_percent(u32::MAX), u16::MAX);
    }

    #[test]
    fn los_nombres_de_dispositivo_se_vuelven_legibles() {
        assert_eq!(friendly_name(r"\\.\DISPLAY1", 0), "Pantalla 1");
        assert_eq!(friendly_name(r"\\.\DISPLAY2", 1), "Pantalla 2");
        assert_eq!(friendly_name(r"\\.\DISPLAY12", 0), "Pantalla 12");
    }

    #[test]
    fn el_relleno_de_ceros_del_buffer_se_recorta() {
        // `szDevice` es un array fijo de 32 caracteres relleno con ceros.
        assert_eq!(friendly_name("\\\\.\\DISPLAY1\0\0\0\0\0", 0), "Pantalla 1");
    }

    #[test]
    fn un_nombre_inesperado_se_devuelve_limpio() {
        assert_eq!(
            friendly_name("Monitor Dell U2720Q", 0),
            "Monitor Dell U2720Q"
        );
        assert_eq!(friendly_name(r"\\.\DISPLAYX", 0), r"\\.\DISPLAYX");
    }

    #[test]
    fn un_nombre_vacio_cae_en_el_indice() {
        assert_eq!(friendly_name("", 0), "Pantalla 1");
        assert_eq!(friendly_name("   ", 2), "Pantalla 3");
        assert_eq!(friendly_name("\0\0\0", 1), "Pantalla 2");
    }

    #[test]
    fn las_dimensiones_salen_del_rectangulo() {
        assert_eq!(rect_dimensions(0, 0, 1_920, 1_080), Some((1_920, 1_080)));
        // Monitor a la izquierda del principal: origen negativo.
        assert_eq!(rect_dimensions(-1_920, 0, 0, 1_080), Some((1_920, 1_080)));
        // Y por encima.
        assert_eq!(rect_dimensions(0, -1_080, 1_920, 0), Some((1_920, 1_080)));
    }

    #[test]
    fn un_rectangulo_degenerado_o_invertido_se_descarta() {
        assert_eq!(rect_dimensions(0, 0, 0, 1_080), None);
        assert_eq!(rect_dimensions(0, 0, 1_920, 0), None);
        assert_eq!(rect_dimensions(100, 0, 50, 1_080), None, "invertido en X");
        assert_eq!(rect_dimensions(0, 100, 1_920, 50), None, "invertido en Y");
    }

    #[test]
    fn un_rectangulo_que_desbordaria_se_descarta_en_vez_de_envolver() {
        assert_eq!(rect_dimensions(i32::MIN, 0, i32::MAX, 100), None);
    }

    #[test]
    fn una_pantalla_normal_se_convierte_entera() {
        let info = to_monitor_info(&raw(r"\\.\DISPLAY1", 0, 0, 2_560, 1_440, true), 0)
            .expect("deberia convertirse");

        assert_eq!(info.index, 0);
        assert_eq!(info.name, "Pantalla 1");
        assert_eq!((info.width, info.height), (2_560, 1_440));
        assert_eq!((info.origin_x, info.origin_y), (0, 0));
        assert_eq!(info.scale_percent, 100);
        assert!(info.is_primary);
        assert_eq!(info.refresh_hz, Some(60));
        info.validate().expect("deberia ser valida");
    }

    #[test]
    fn una_frecuencia_de_cero_o_uno_se_trata_como_desconocida() {
        for hz in [Some(0), Some(1), None] {
            let mut r = raw(r"\\.\DISPLAY1", 0, 0, 1_920, 1_080, true);
            r.refresh_hz = hz;
            let info = to_monitor_info(&r, 0).expect("convierte");
            assert_eq!(info.refresh_hz, None, "con {hz:?} deberia ser desconocida");
        }
    }

    #[test]
    fn una_pantalla_imposible_se_descarta() {
        assert!(to_monitor_info(&raw(r"\\.\DISPLAY1", 0, 0, 0, 0, true), 0).is_none());
    }

    #[test]
    fn la_disposicion_numera_de_forma_consecutiva() {
        let raws = vec![
            raw(r"\\.\DISPLAY1", 0, 0, 1_920, 1_080, true),
            raw(r"\\.\DISPLAY2", -2_560, 0, 0, 1_440, false),
            raw(r"\\.\DISPLAY3", 1_920, 0, 3_840, 1_080, false),
        ];
        let layout = to_layout(&raws);

        assert_eq!(layout.len(), 3);
        for (esperado, monitor) in layout.0.iter().enumerate() {
            assert_eq!(usize::from(monitor.index), esperado);
        }
        assert_eq!(layout.primary().expect("hay principal").name, "Pantalla 1");
        layout.validate().expect("todas validas");
    }

    #[test]
    fn descartar_una_pantalla_no_deja_huecos_en_los_indices() {
        let raws = vec![
            raw(r"\\.\DISPLAY1", 0, 0, 1_920, 1_080, true),
            raw(r"\\.\DISPLAY2", 0, 0, 0, 0, false), // imposible, se descarta
            raw(r"\\.\DISPLAY3", 1_920, 0, 3_840, 1_080, false),
        ];
        let layout = to_layout(&raws);

        assert_eq!(layout.len(), 2);
        assert_eq!(layout.0[0].index, 0);
        assert_eq!(
            layout.0[1].index, 1,
            "los indices deben seguir siendo consecutivos"
        );
        assert_eq!(layout.0[1].name, "Pantalla 3");
        // Y se pueden pedir por indice sin sorpresas.
        assert!(layout.require(0).is_ok());
        assert!(layout.require(1).is_ok());
        assert!(layout.require(2).is_err());
    }

    #[test]
    fn una_disposicion_vacia_no_rompe_nada() {
        let layout = to_layout(&[]);
        assert!(layout.is_empty());
        assert!(layout.primary().is_none());
    }

    #[test]
    fn una_pantalla_de_alta_densidad_reporta_su_escala() {
        let mut r = raw(r"\\.\DISPLAY1", 0, 0, 3_840, 2_160, true);
        r.dpi = 192;
        let info = to_monitor_info(&r, 0).expect("convierte");
        assert_eq!(info.scale_percent, 200);
        // La resolucion sigue siendo la fisica, no la escalada.
        assert_eq!((info.width, info.height), (3_840, 2_160));
    }
}
