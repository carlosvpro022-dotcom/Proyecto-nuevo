//! Informe completo de lo que este equipo puede hacer, en un solo comando.
//!
//! ```bash
//! cargo run -p nexus-windows-host --example diagnostico
//! ```
//!
//! Comprueba lo mismo que la pantalla *Escritorio remoto* del panel, pero sin
//! necesitar que la ventana llegue a abrirse. Eso importa: el panel es una
//! aplicacion Tauri, y si falla al compilar o al arrancar la interfaz, la
//! informacion que haria falta para arreglarlo se queda dentro. Esto es un
//! binario normal que escribe en la consola.
//!
//! No toca nada: solo mira y cuenta. No escribe en el disco, no abre el
//! portapapeles, no apaga nada y no se conecta a ningun servidor.
//!
//! La salida esta pensada para copiarse entera y pegarse tal cual.

use nexus_windows_host::commands::FeatureState;
use nexus_windows_host::host::EngineCheck;

/// Construye una comprobacion correcta.
fn ok(detail: impl Into<String>) -> EngineCheck {
    EngineCheck {
        working: true,
        detail: detail.into(),
    }
}

/// Construye una comprobacion fallida.
fn falla(detail: impl Into<String>) -> EngineCheck {
    EngineCheck {
        working: false,
        detail: detail.into(),
    }
}

/// Ancho de la etiqueta de cada linea, para que todo quede alineado.
const ETIQUETA: usize = 26;

fn main() {
    titulo("NEXUS REMOTE · diagnostico de este equipo");
    sistema();
    let monitores = motor();
    pantallas(&monitores);
    proyecto();
    titulo("Fin del informe. Copielo entero.");
}

/// Datos del equipo y de esta compilacion.
fn sistema() {
    seccion("Sistema");
    let estado = nexus_windows_host::system::read_status();
    dato("Sistema operativo", &estado.os_description);
    dato("Compilado para", std::env::consts::OS);
    dato("Nombre del equipo", &estado.hostname);
    dato("Nucleos logicos", &estado.cpu_cores.to_string());
    dato(
        "Memoria",
        &format!(
            "{} MiB de {} MiB libres",
            estado.memory_available_mib, estado.memory_total_mib
        ),
    );
    dato(
        "Sesion bloqueada",
        match estado.session_locked {
            Some(true) => "si",
            Some(false) => "no",
            // Es la respuesta honesta mientras no se implemente la deteccion.
            None => "no se sabe (sin implementar)",
        },
    );
    dato("Version", env!("CARGO_PKG_VERSION"));
    dato(
        "Version del protocolo",
        &nexus_core::PROTOCOL_VERSION.to_string(),
    );
}

/// Lo que este equipo puede hacer ahora mismo. Devuelve los monitores para no
/// enumerarlos dos veces.
fn motor() -> nexus_capture::CaptureResult<nexus_core::monitor::MonitorLayout> {
    seccion("Motor: lo que este equipo puede hacer ahora mismo");
    let monitores = nexus_capture::enumerate_monitors();
    comprobacion(
        "Captura de pantalla",
        &match &monitores {
            Ok(layout) if layout.is_empty() => falla("el sistema no reporto ninguna pantalla"),
            Ok(layout) => ok(format!("{} pantalla(s)", layout.len())),
            Err(error) => falla(error.to_string()),
        },
    );
    comprobacion(
        "Raton y teclado",
        &match nexus_input::default_injector() {
            Ok(injector) if injector.name() == "unsupported" => {
                falla(format!("sin backend en {}", std::env::consts::OS))
            }
            Ok(injector) => ok(format!("backend {}", injector.name())),
            Err(error) => falla(error.to_string()),
        },
    );
    comprobacion(
        "Portapapeles",
        // Sin abrirlo y sin leer su contenido: solo si responde.
        &match nexus_clipboard::default_clipboard().probe() {
            Ok(true) => ok("responde, y ahora tiene texto".to_owned()),
            Ok(false) => ok("responde, y ahora no tiene texto".to_owned()),
            Err(error) => falla(error.to_string()),
        },
    );
    comprobacion(
        "Apagar y reiniciar",
        // Comprueba el privilegio y lo vuelve a quitar. No apaga nada.
        &match nexus_power::default_power().probe() {
            Ok(()) => ok("esta cuenta puede apagar el equipo".to_owned()),
            Err(error) => falla(error.to_string()),
        },
    );
    comprobacion("Cifrado de la clave", &{
        let vault = nexus_keyvault::default_vault();
        if vault.protects_at_rest() {
            ok(format!("almacen {}", vault.name()))
        } else {
            falla(format!(
                "almacen {}: la clave quedaria sin cifrar",
                vault.name()
            ))
        }
    });

    monitores
}

/// Detalle de cada pantalla, que es donde se ve si la escala cuadra.
fn pantallas(monitores: &nexus_capture::CaptureResult<nexus_core::monitor::MonitorLayout>) {
    seccion("Pantallas detectadas");
    match monitores {
        Ok(layout) if layout.is_empty() => {
            println!("  Ninguna.");
        }
        Ok(layout) => {
            for m in &layout.0 {
                println!(
                    "  [{}] {:<22} {}x{} en ({}, {})  escala {} %  {}{}",
                    m.index,
                    recortar(&m.name, 22),
                    m.width,
                    m.height,
                    m.origin_x,
                    m.origin_y,
                    m.scale_percent,
                    m.refresh_hz.map_or_else(
                        || "refresco desconocido".to_owned(),
                        |hz| format!("{hz} Hz")
                    ),
                    if m.is_primary { "  (principal)" } else { "" }
                );
            }
            println!();
            println!("  Compruebe que la escala coincide con la de Windows: las");
            println!("  coordenadas del raton se calculan con ese numero.");
        }
        Err(error) => println!("  No se pudieron enumerar: {error}"),
    }
}

/// Lo que el propio programa declara sobre su estado.
fn proyecto() {
    seccion("Estado del proyecto, segun el propio programa");
    let matriz = nexus_windows_host::commands::feature_matrix();
    let probadas = matriz
        .iter()
        .filter(|f| f.state == FeatureState::Working)
        .count();
    let sin_probar = matriz
        .iter()
        .filter(|f| f.state == FeatureState::Unverified)
        .count();
    let pendientes = matriz
        .iter()
        .filter(|f| f.state == FeatureState::Pending)
        .count();
    dato(
        "Funciones",
        &format!(
            "{probadas} probadas, {sin_probar} sin ejecutar en Windows, {pendientes} pendientes"
        ),
    );
    println!();
    for f in &matriz {
        let marca = match f.state {
            FeatureState::Working => "[probado]   ",
            FeatureState::Unverified => "[sin probar]",
            FeatureState::Pending => "[pendiente] ",
        };
        println!("  {marca} E{}  {}", f.stage, f.label);
    }
}

fn titulo(texto: &str) {
    println!();
    println!("{}", "=".repeat(72));
    println!("  {texto}");
    println!("{}", "=".repeat(72));
}

fn seccion(texto: &str) {
    println!();
    println!(
        "-- {texto} {}",
        "-".repeat(68_usize.saturating_sub(texto.len()))
    );
}

fn dato(etiqueta: &str, valor: &str) {
    println!("  {etiqueta:<ETIQUETA$}  {valor}");
}

fn comprobacion(etiqueta: &str, check: &EngineCheck) {
    let marca = if check.working { "OK  " } else { "FALLA" };
    println!("  {marca} {etiqueta:<ETIQUETA$}  {}", check.detail);
}

/// Recorta un nombre largo para que la tabla no se desmonte.
fn recortar(texto: &str, ancho: usize) -> String {
    if texto.chars().count() <= ancho {
        return texto.to_owned();
    }
    texto
        .chars()
        .take(ancho.saturating_sub(1))
        .collect::<String>()
        + "…"
}
