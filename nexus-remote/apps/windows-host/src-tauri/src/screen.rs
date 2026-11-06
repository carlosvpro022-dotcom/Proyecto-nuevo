//! El bombeo de la pantalla: capturar, codificar y entregar.
//!
//! Une tres piezas que ya existian por separado —el capturador, el regulador
//! de ritmo y el codificador— y toma las decisiones que ninguna de ellas puede
//! tomar sola.
//!
//! ## Las decisiones que se toman aqui
//!
//! **Cuando capturar.** No en cuanto se pueda: al ritmo que pida el perfil de
//! calidad. Capturar mas deprisa de lo que se envia solo gasta CPU para tirar
//! el resultado.
//!
//! **Que hacer cuando no hay nada nuevo.** El capturador avisa de que la
//! pantalla no ha cambiado. Eso no es un fallo y no se trata como tal: no
//! enviar nada es la mejor compresion que existe.
//!
//! **Cuando rendirse.** Un fallo de captura suelto es normal: ocurre al
//! cambiar la resolucion, al bloquear la sesion o al entrar un juego a
//! pantalla completa. Cortar la sesion por uno seria aparatoso. Pero fallar
//! siempre significa que algo se rompio de verdad, y seguir intentandolo en
//! silencio dejaria al otro extremo mirando una imagen congelada sin saber por
//! que. Se aguantan unos cuantos y despues se dice.
//!
//! ## Por que esto es sincrono
//!
//! Capturar y codificar son trabajo de CPU, no de espera. Ponerlos en una
//! funcion asincrona daria la falsa impresion de que se pueden intercalar con
//! otra cosa: no se pueden, ocupan el hilo mientras duran. Quien llama lo
//! ejecuta en el grupo de hilos que existe para eso.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use nexus_capture::capturer::ScreenCapturer;
use nexus_capture::error::CaptureError;
use nexus_capture::pacing::FramePacer;
use nexus_core::monitor::MonitorLayout;
use nexus_core::{QualityProfile, Timestamp};
use nexus_video::{EncodedFrame, Encoder, OpenH264Encoder};

/// Fallos de captura seguidos que se aguantan antes de darse por vencido.
///
/// Cambiar la resolucion o bloquear la sesion produce unos pocos seguidos y se
/// recupera solo. Treinta seguidos, a treinta por segundo, es un segundo sin
/// una sola captura: eso ya no se arregla esperando.
const MAX_FALLOS_SEGUIDOS: u32 = 30;

/// Lo que produce una vuelta del bombeo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pumped {
    /// Hay un fotograma listo para enviar.
    Frame(EncodedFrame),
    /// Todavia no toca, o la pantalla no ha cambiado. No es un fallo.
    Nothing,
    /// La captura se rompio y no se recupera.
    Broken {
        /// Motivo, para el registro y para el otro extremo.
        detail: String,
    },
}

/// Captura la pantalla, la codifica y la entrega lista para enviar.
pub struct ScreenPump {
    capturer: Box<dyn ScreenCapturer>,
    encoder: Box<dyn Encoder>,
    pacer: FramePacer,
    profile: QualityProfile,
    monitor: u8,
    fallos: u32,
    /// Fotogramas entregados desde que empezo.
    delivered: u64,
}

impl std::fmt::Debug for ScreenPump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenPump")
            .field("capturador", &self.capturer.name())
            .field("codificador", &self.encoder.name())
            .field("monitor", &self.monitor)
            .field("entregados", &self.delivered)
            .finish_non_exhaustive()
    }
}

impl ScreenPump {
    /// Prepara el bombeo sobre un monitor concreto.
    ///
    /// # Errors
    ///
    /// Lo que devuelva el capturador al enumerar o seleccionar, y
    /// [`CaptureError::System`] si el codificador no se puede preparar para el
    /// tamano de ese monitor.
    pub fn new(
        mut capturer: Box<dyn ScreenCapturer>,
        monitor: u8,
        profile: &QualityProfile,
    ) -> Result<Self, CaptureError> {
        let layout = capturer.enumerate_monitors()?;
        let (width, height) = size_of(&layout, monitor)?;
        capturer.select_monitor(monitor)?;

        let encoder = OpenH264Encoder::new(width, height, profile)
            .map_err(|e| CaptureError::System(e.to_string()))?;

        Ok(Self {
            capturer,
            encoder: Box::new(encoder),
            pacer: FramePacer::from_profile(profile),
            profile: *profile,
            monitor,
            fallos: 0,
            delivered: 0,
        })
    }

    /// Monitor que se esta capturando.
    #[must_use]
    pub const fn monitor(&self) -> u8 {
        self.monitor
    }

    /// Fotogramas entregados desde que empezo.
    #[must_use]
    pub const fn delivered(&self) -> u64 {
        self.delivered
    }

    /// Da una vuelta: captura si toca, codifica si hay algo nuevo.
    pub fn tick(&mut self, now: Timestamp) -> Pumped {
        if !self.pacer.should_capture(now) {
            return Pumped::Nothing;
        }

        let frame = match self.capturer.capture_frame(now) {
            Ok(frame) => {
                self.fallos = 0;
                frame
            }
            // La pantalla no ha cambiado. Es lo normal y lo deseable.
            Err(CaptureError::WouldBlock) => {
                self.pacer.record_drop();
                return Pumped::Nothing;
            }
            Err(error) => {
                self.fallos = self.fallos.saturating_add(1);
                if self.fallos >= MAX_FALLOS_SEGUIDOS {
                    return Pumped::Broken {
                        detail: format!("la captura fallo {} veces seguidas: {error}", self.fallos),
                    };
                }
                tracing::debug!(%error, intentos = self.fallos, "fallo de captura, se reintenta");
                return Pumped::Nothing;
            }
        };

        self.pacer.record_capture(now);

        match self.encoder.encode(&frame) {
            Ok(Some(codificado)) => {
                self.delivered = self.delivered.saturating_add(1);
                Pumped::Frame(codificado)
            }
            // El codificador decidio que no hacia falta enviar nada.
            Ok(None) => Pumped::Nothing,
            Err(error) => Pumped::Broken {
                detail: format!("el codificador fallo: {error}"),
            },
        }
    }

    /// Cambia el perfil de calidad.
    ///
    /// Rehace el codificador: el tamano y la tasa de bits son parte de como se
    /// preparo, y no se pueden cambiar a mitad. El siguiente fotograma sera
    /// completo, porque el otro extremo no puede decodificar lo nuevo a partir
    /// de lo viejo.
    ///
    /// # Errors
    ///
    /// [`CaptureError::System`] si el codificador no se puede rehacer; en ese
    /// caso se **mantiene** el que habia, que es mejor que quedarse sin
    /// ninguno.
    pub fn set_quality(&mut self, profile: &QualityProfile) -> Result<(), CaptureError> {
        let layout = self.capturer.enumerate_monitors()?;
        let (width, height) = size_of(&layout, self.monitor)?;

        let encoder = OpenH264Encoder::new(width, height, profile)
            .map_err(|e| CaptureError::System(e.to_string()))?;

        self.encoder = Box::new(encoder);
        self.pacer.set_target_fps(profile.target_fps);
        self.profile = *profile;
        Ok(())
    }

    /// Cambia el monitor que se captura.
    ///
    /// # Errors
    ///
    /// [`CaptureError::UnknownMonitor`] si no existe, o
    /// [`CaptureError::System`] si el codificador no se puede rehacer para el
    /// nuevo tamano. En los dos casos se sigue capturando el de antes.
    pub fn select_monitor(&mut self, index: u8) -> Result<(), CaptureError> {
        let layout = self.capturer.enumerate_monitors()?;
        let (width, height) = size_of(&layout, index)?;

        // El codificador primero: si falla, no se ha cambiado de monitor y la
        // sesion sigue viendo lo que veia.
        let encoder = OpenH264Encoder::new(width, height, &self.profile)
            .map_err(|e| CaptureError::System(e.to_string()))?;
        self.capturer.select_monitor(index)?;

        self.encoder = Box::new(encoder);
        self.monitor = index;
        Ok(())
    }

    /// Pide que el siguiente fotograma sea completo.
    ///
    /// Lo necesita quien acaba de conectarse o quien perdio el hilo: sin un
    /// fotograma completo no hay nada que decodificar.
    pub fn request_keyframe(&mut self) {
        self.encoder.request_keyframe();
    }

    /// Perfil de calidad en uso.
    #[must_use]
    pub const fn profile(&self) -> QualityProfile {
        self.profile
    }
}

/// Espera minima entre vueltas cuando la pantalla no cambia.
///
/// Sin un suelo el bucle giraria sin descanso preguntando "¿hay algo nuevo?":
/// el regulador no cuenta como capturado lo que no cambio, asi que la espera
/// que recomienda es cero. Dos milisegundos son medio millar de preguntas por
/// segundo, barato de sobra, y mantienen la respuesta viva en cuanto la
/// pantalla se mueve otra vez.
const ESPERA_MINIMA: Duration = Duration::from_millis(2);

/// Ordenes que la sesion puede dar al bombeo mientras corre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenCommand {
    /// Cambiar el perfil de calidad.
    Quality(QualityProfile),
    /// Cambiar el monitor que se captura.
    Monitor(u8),
    /// Enviar un fotograma completo en la siguiente vuelta.
    Keyframe,
}

/// Lo que el bombeo entrega hacia la sesion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenOutput {
    /// Un fotograma codificado, listo para la pista de video.
    Frame {
        /// Flujo de bytes del codec.
        data: Vec<u8>,
        /// Lo que el fotograma dura en pantalla.
        duration: Duration,
        /// Si es completo, es decir, si no depende de los anteriores.
        keyframe: bool,
    },
    /// Una orden no se pudo aplicar. El bombeo sigue con lo que tenia.
    Refused {
        /// Motivo, para contestar a quien lo pidio.
        detail: String,
    },
    /// La captura se rompio: el bombeo ha terminado.
    Broken {
        /// Motivo, para el registro y para el otro extremo.
        detail: String,
    },
}

/// Lo que toca hacer despues de una vuelta del bombeo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Hay fotograma que enviar.
    Send(EncodedFrame),
    /// No hay nada: esperar esto antes de volver a intentarlo.
    Wait(Duration),
    /// Se acabo.
    Stop {
        /// Motivo.
        detail: String,
    },
}

impl ScreenPump {
    /// Lo que dura cada fotograma al ritmo actual.
    #[must_use]
    pub const fn frame_interval(&self) -> Duration {
        self.pacer.frame_interval()
    }

    /// Da una vuelta y dice que hacer a continuacion.
    ///
    /// Es [`Self::tick`] mas la espera: cuando no hay nada que enviar, quien
    /// llama necesita saber cuanto puede dormir sin llegar tarde al siguiente
    /// fotograma.
    pub fn step(&mut self, now: Timestamp) -> Step {
        match self.tick(now) {
            Pumped::Frame(frame) => Step::Send(frame),
            Pumped::Nothing => Step::Wait(self.pacer.time_until_next(now).max(ESPERA_MINIMA)),
            Pumped::Broken { detail } => Step::Stop { detail },
        }
    }

    /// Aplica una orden de la sesion.
    ///
    /// # Errors
    ///
    /// Lo que devuelvan [`Self::set_quality`] o [`Self::select_monitor`]. En
    /// los dos casos el bombeo sigue capturando lo que capturaba.
    pub fn apply(&mut self, command: &ScreenCommand) -> Result<(), CaptureError> {
        match command {
            ScreenCommand::Quality(profile) => self.set_quality(profile),
            ScreenCommand::Monitor(index) => self.select_monitor(*index),
            ScreenCommand::Keyframe => {
                self.request_keyframe();
                Ok(())
            }
        }
    }
}

/// Bombea la pantalla hasta que la sesion termina.
///
/// Esta funcion **bloquea el hilo**: capturar y codificar son trabajo de CPU y
/// no sueltan el hilo mientras duran. Se ejecuta en un hilo propio o en el
/// grupo de hilos de bloqueo, nunca en el bucle asincrono.
///
/// ## Las decisiones que se toman aqui
///
/// **Un fotograma codificado no se tira nunca.** En H.264 cada fotograma
/// normal se describe como la diferencia con los anteriores, asi que perder
/// uno estropea todos los que vengan detras hasta el siguiente completo. Si
/// la entrega va mas lenta que la captura, se espera: la espera frena el
/// ritmo, que es justo lo que hay que hacer, mientras tirar fotogramas
/// romperia la imagen.
///
/// **Las ordenes no esperan su turno.** La espera entre fotogramas se hace
/// escuchando el canal de ordenes, de modo que cambiar de monitor o pedir un
/// fotograma completo surte efecto en cuanto llega y no al final del intervalo.
///
/// **El reloj es monotono.** Se mide desde que empezo el bombeo con
/// [`Instant`], no con la hora del sistema: una correccion de hora (NTP,
/// cambio de zona) dejaria el ritmo parado o disparado si se midiera con ella.
///
/// **Se termina cuando se cierra cualquiera de los dos canales.** Soltar el
/// extremo de ordenes es la forma de parar el bombeo; que nadie recoja los
/// fotogramas significa que la sesion ya no existe.
pub fn run(
    mut pump: ScreenPump,
    commands: &mpsc::Receiver<ScreenCommand>,
    frames: &tokio::sync::mpsc::Sender<ScreenOutput>,
) {
    let inicio = Instant::now();
    let mut espera = Duration::ZERO;

    loop {
        // Las ordenes, primero: cambiar de monitor antes de capturar evita
        // entregar un fotograma de lo que ya no se quiere ver. La espera de la
        // vuelta anterior se cumple aqui mismo.
        match recoger(commands, espera) {
            Recogida::Orden(orden) => {
                espera = Duration::ZERO;
                if let Some(aviso) = atender(&mut pump, &orden)
                    && frames.blocking_send(aviso).is_err()
                {
                    return;
                }
                continue;
            }
            Recogida::Nada => {}
            Recogida::Cerrado => return,
        }

        let ahora = Timestamp(u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX));
        match pump.step(ahora) {
            Step::Send(frame) => {
                espera = Duration::ZERO;
                let salida = ScreenOutput::Frame {
                    duration: pump.frame_interval(),
                    keyframe: frame.keyframe,
                    data: frame.data,
                };
                if frames.blocking_send(salida).is_err() {
                    return;
                }
            }
            Step::Wait(cuanto) => espera = cuanto,
            Step::Stop { detail } => {
                // El ultimo mensaje es el motivo: si nadie lo recoge, da igual,
                // porque la sesion ya no esta.
                let _ = frames.blocking_send(ScreenOutput::Broken { detail });
                return;
            }
        }
    }
}

/// Que trajo el canal de ordenes.
enum Recogida {
    /// Una orden que aplicar.
    Orden(ScreenCommand),
    /// Nada en el plazo dado.
    Nada,
    /// La sesion soltó el otro extremo.
    Cerrado,
}

/// Espera una orden como maximo `espera`, o mira sin esperar si es cero.
fn recoger(commands: &mpsc::Receiver<ScreenCommand>, espera: Duration) -> Recogida {
    if espera.is_zero() {
        match commands.try_recv() {
            Ok(orden) => Recogida::Orden(orden),
            Err(mpsc::TryRecvError::Empty) => Recogida::Nada,
            Err(mpsc::TryRecvError::Disconnected) => Recogida::Cerrado,
        }
    } else {
        match commands.recv_timeout(espera) {
            Ok(orden) => Recogida::Orden(orden),
            Err(mpsc::RecvTimeoutError::Timeout) => Recogida::Nada,
            Err(mpsc::RecvTimeoutError::Disconnected) => Recogida::Cerrado,
        }
    }
}

/// Aplica una orden y devuelve que contestar, si hay algo que contestar.
fn atender(pump: &mut ScreenPump, command: &ScreenCommand) -> Option<ScreenOutput> {
    match pump.apply(command) {
        Ok(()) => None,
        Err(error) => Some(ScreenOutput::Refused {
            detail: error.to_string(),
        }),
    }
}

/// De donde sale el capturador, creado ya en el hilo que captura.
///
/// Existe por una razon muy concreta: el capturador de Windows guarda un
/// dispositivo Direct3D y objetos COM, que no se pueden mover de un hilo a
/// otro. Si el host lo creara y luego lo pasara al hilo de captura, el
/// programa no compilaria en Windows (y si se forzara, fallaria en ejecucion).
/// Asi que lo que viaja entre hilos es esta fuente, y el capturador nace ya
/// donde se va a usar.
///
/// Es tambien la costura por la que las pruebas meten un capturador falso sin
/// tocar nada del camino real.
pub trait CapturerSource: Send + Sync {
    /// Crea un capturador para una sesion.
    ///
    /// # Errors
    ///
    /// Lo que devuelva la plataforma al preparar la captura.
    fn create(&self) -> Result<Box<dyn ScreenCapturer>, CaptureError>;
}

/// La fuente de verdad: el capturador de esta plataforma.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformCapturers;

impl CapturerSource for PlatformCapturers {
    fn create(&self) -> Result<Box<dyn ScreenCapturer>, CaptureError> {
        nexus_capture::default_capturer()
    }
}

/// Prepara el bombeo y lo mantiene hasta que la sesion termina.
///
/// Es lo que se ejecuta en el hilo de captura: crea el capturador alli (ver
/// [`CapturerSource`]), monta el bombeo y entra en [`run`].
///
/// Si no se puede ni empezar —no hay captura en esta plataforma, el monitor
/// desaparecio, el codificador no arranca— lo dice por el mismo canal y
/// termina. Quedarse callado dejaria al otro extremo esperando una pantalla
/// que no va a llegar nunca.
pub fn serve(
    source: &dyn CapturerSource,
    monitor: u8,
    profile: &QualityProfile,
    commands: &mpsc::Receiver<ScreenCommand>,
    frames: &tokio::sync::mpsc::Sender<ScreenOutput>,
) {
    let pump = match source
        .create()
        .and_then(|capturer| ScreenPump::new(capturer, monitor, profile))
    {
        Ok(pump) => pump,
        Err(error) => {
            let _ = frames.blocking_send(ScreenOutput::Broken {
                detail: format!("no se pudo empezar a capturar la pantalla: {error}"),
            });
            return;
        }
    };

    tracing::info!(
        capturador = pump.capturer.name(),
        codificador = pump.encoder.name(),
        monitor,
        "empieza el envio de la pantalla"
    );
    run(pump, commands, frames);
}

/// Tamano del monitor pedido.
fn size_of(layout: &MonitorLayout, index: u8) -> Result<(u32, u32), CaptureError> {
    layout
        .0
        .iter()
        .find(|m| m.index == index)
        .map(|m| (m.width, m.height))
        .ok_or(CaptureError::UnknownMonitor { index })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_capture::frame::{Frame, PixelFormat};
    use nexus_core::monitor::MonitorInfo;
    use nexus_core::{QualityPreset, QualityProfile};
    use std::sync::{Arc, Mutex};

    const ANCHO: u32 = 320;
    const ALTO: u32 = 240;

    /// Lo que el capturador de prueba debe hacer en cada llamada.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Respuesta {
        /// Entrega una pantalla.
        Pantalla,
        /// Dice que no hay nada nuevo.
        SinCambios,
        /// Falla.
        Fallo,
    }

    /// Capturador de prueba: entrega lo que se le diga, en orden.
    ///
    /// No captura nada. Existe porque capturar de verdad solo funciona en
    /// Windows, y lo que hay que comprobar aqui son las **decisiones** del
    /// bombeo, que no dependen de la plataforma.
    #[derive(Clone)]
    struct Guionizado {
        guion: Arc<Mutex<Vec<Respuesta>>>,
        /// Por omision cuando el guion se acaba.
        resto: Respuesta,
        monitores: MonitorLayout,
        seleccionado: Arc<Mutex<u8>>,
        capturas: Arc<Mutex<u32>>,
    }

    impl Guionizado {
        fn nuevo(guion: Vec<Respuesta>, resto: Respuesta) -> Self {
            Self {
                guion: Arc::new(Mutex::new(guion)),
                resto,
                monitores: MonitorLayout(vec![
                    MonitorInfo {
                        index: 0,
                        name: "Principal".to_owned(),
                        width: ANCHO,
                        height: ALTO,
                        origin_x: 0,
                        origin_y: 0,
                        scale_percent: 100,
                        is_primary: true,
                        refresh_hz: Some(60),
                    },
                    MonitorInfo {
                        index: 1,
                        name: "Secundario".to_owned(),
                        width: 160,
                        height: 120,
                        origin_x: 320,
                        origin_y: 0,
                        scale_percent: 100,
                        is_primary: false,
                        refresh_hz: Some(60),
                    },
                ]),
                seleccionado: Arc::new(Mutex::new(0)),
                capturas: Arc::new(Mutex::new(0)),
            }
        }

        fn capturas(&self) -> u32 {
            *self.capturas.lock().expect("sin envenenar")
        }

        fn seleccionado(&self) -> u8 {
            *self.seleccionado.lock().expect("sin envenenar")
        }
    }

    impl ScreenCapturer for Guionizado {
        fn name(&self) -> &'static str {
            "guionizado"
        }

        fn enumerate_monitors(&self) -> Result<MonitorLayout, CaptureError> {
            Ok(self.monitores.clone())
        }

        fn select_monitor(&mut self, index: u8) -> Result<(), CaptureError> {
            if !self.monitores.0.iter().any(|m| m.index == index) {
                return Err(CaptureError::UnknownMonitor { index });
            }
            *self.seleccionado.lock().expect("sin envenenar") = index;
            Ok(())
        }

        fn capture_frame(&mut self, now: Timestamp) -> Result<Frame, CaptureError> {
            let siguiente = {
                let mut guion = self.guion.lock().expect("sin envenenar");
                if guion.is_empty() {
                    self.resto
                } else {
                    guion.remove(0)
                }
            };

            match siguiente {
                Respuesta::SinCambios => Err(CaptureError::WouldBlock),
                Respuesta::Fallo => Err(CaptureError::System("de prueba".to_owned())),
                Respuesta::Pantalla => {
                    let mut capturas = self.capturas.lock().expect("sin envenenar");
                    *capturas += 1;
                    let n = *capturas;
                    drop(capturas);

                    let indice = self.seleccionado();
                    let m = self
                        .monitores
                        .0
                        .iter()
                        .find(|m| m.index == indice)
                        .expect("el monitor existe");
                    Ok(pantalla(m.width, m.height, n, now))
                }
            }
        }
    }

    /// Una imagen con aspecto de pantalla, distinta en cada captura.
    fn pantalla(width: u32, height: u32, n: u32, now: Timestamp) -> Frame {
        let stride = width as usize * 4;
        let mut data = vec![0_u8; stride * height as usize];
        for y in 0..height {
            for x in 0..width {
                let bloque = (x + n * 8) % 64 < 20 && y > height / 4;
                let color: [u8; 4] = if bloque {
                    [30, 30, 30, 255]
                } else {
                    [240, 240, 235, 255]
                };
                let p = y as usize * stride + x as usize * 4;
                data[p..p + 4].copy_from_slice(&color);
            }
        }
        Frame {
            width,
            height,
            format: PixelFormat::Bgra8,
            stride,
            data,
            captured_at: now,
            monitor_index: 0,
        }
    }

    fn perfil() -> QualityProfile {
        QualityProfile {
            scale_percent: 100,
            ..QualityProfile::from_preset(QualityPreset::Nitido)
        }
    }

    /// Avanza el reloj lo bastante para que el regulador deje capturar.
    fn instante(n: u64) -> Timestamp {
        Timestamp(1_000_000 + n * 1_000)
    }

    fn bombeo(capturer: Guionizado) -> ScreenPump {
        ScreenPump::new(Box::new(capturer), 0, &perfil()).expect("prepara el bombeo")
    }

    impl CapturerSource for Guionizado {
        fn create(&self) -> Result<Box<dyn ScreenCapturer>, CaptureError> {
            Ok(Box::new(self.clone()))
        }
    }

    #[test]
    fn el_primer_fotograma_sale_completo_y_se_puede_decodificar() {
        let mut pump = bombeo(Guionizado::nuevo(vec![], Respuesta::Pantalla));
        match pump.tick(instante(1)) {
            Pumped::Frame(f) => {
                assert!(f.keyframe, "el primero tiene que ser completo");
                assert!(!f.data.is_empty());
                assert_eq!((f.width, f.height), (ANCHO as usize, ALTO as usize));
            }
            otro => panic!("se esperaba un fotograma, llego {otro:?}"),
        }
        assert_eq!(pump.delivered(), 1);
    }

    #[test]
    fn una_pantalla_sin_cambios_no_produce_nada_y_no_es_un_fallo() {
        // No enviar nada es la mejor compresion que existe.
        let capturer = Guionizado::nuevo(vec![], Respuesta::SinCambios);
        let mut pump = bombeo(capturer.clone());
        for i in 0..5 {
            assert_eq!(pump.tick(instante(i)), Pumped::Nothing);
        }
        assert_eq!(capturer.capturas(), 0, "no deberia haber capturado nada");
        assert_eq!(pump.delivered(), 0);
    }

    #[test]
    fn unos_cuantos_fallos_seguidos_no_cortan_la_sesion() {
        // Cambiar la resolucion o bloquear la sesion produce unos pocos y se
        // recupera solo. Cortar por eso seria aparatoso.
        let guion = vec![Respuesta::Fallo; 5];
        let mut pump = bombeo(Guionizado::nuevo(guion, Respuesta::Pantalla));

        for i in 0..5 {
            assert_eq!(
                pump.tick(instante(i)),
                Pumped::Nothing,
                "el fallo {i} no deberia cortar nada"
            );
        }
        assert!(
            matches!(pump.tick(instante(6)), Pumped::Frame(_)),
            "despues tiene que recuperarse solo"
        );
    }

    #[test]
    fn fallar_siempre_se_dice_en_lugar_de_dejar_la_imagen_congelada() {
        // Seguir intentandolo en silencio dejaria al otro extremo mirando una
        // imagen congelada sin saber por que.
        let mut pump = bombeo(Guionizado::nuevo(vec![], Respuesta::Fallo));
        let mut roto = None;
        for i in 0..MAX_FALLOS_SEGUIDOS + 5 {
            if let Pumped::Broken { detail } = pump.tick(instante(u64::from(i))) {
                roto = Some(detail);
                break;
            }
        }
        let detail = roto.expect("tendria que haberse rendido y decirlo");
        assert!(
            detail.contains("veces seguidas"),
            "el motivo tiene que explicarse: {detail}"
        );
    }

    #[test]
    fn un_fallo_suelto_no_cuenta_para_rendirse() {
        // Lo que importa es que sean seguidos: una captura buena en medio
        // vuelve a empezar la cuenta.
        let mut guion = Vec::new();
        for _ in 0..3 {
            guion.extend(vec![Respuesta::Fallo; MAX_FALLOS_SEGUIDOS as usize - 1]);
            guion.push(Respuesta::Pantalla);
        }
        let mut pump = bombeo(Guionizado::nuevo(guion, Respuesta::Pantalla));

        for i in 0..MAX_FALLOS_SEGUIDOS * 3 {
            assert!(
                !matches!(pump.tick(instante(u64::from(i))), Pumped::Broken { .. }),
                "no deberia rendirse mientras se recupere de vez en cuando"
            );
        }
    }

    #[test]
    fn cambiar_de_calidad_cambia_el_tamano_que_se_envia() {
        let mut pump = bombeo(Guionizado::nuevo(vec![], Respuesta::Pantalla));
        let Pumped::Frame(antes) = pump.tick(instante(1)) else {
            panic!("se esperaba un fotograma");
        };
        assert_eq!(antes.width, ANCHO as usize);

        let fluido = QualityProfile::from_preset(QualityPreset::Fluido);
        pump.set_quality(&fluido).expect("cambia la calidad");
        assert_eq!(pump.profile().scale_percent, 60);

        let Pumped::Frame(despues) = pump.tick(instante(2)) else {
            panic!("se esperaba un fotograma");
        };
        assert_eq!(despues.width, 192, "320 al 60 % son 192");
        assert!(
            despues.keyframe,
            "tras cambiar el tamano hace falta uno completo: lo nuevo no se \
             puede decodificar a partir de lo viejo"
        );
    }

    #[test]
    fn cambiar_de_monitor_cambia_lo_que_se_captura() {
        let capturer = Guionizado::nuevo(vec![], Respuesta::Pantalla);
        let mut pump = bombeo(capturer.clone());
        assert_eq!(pump.monitor(), 0);

        pump.select_monitor(1).expect("cambia de monitor");
        assert_eq!(pump.monitor(), 1);
        assert_eq!(capturer.seleccionado(), 1);

        let Pumped::Frame(f) = pump.tick(instante(1)) else {
            panic!("se esperaba un fotograma");
        };
        assert_eq!((f.width, f.height), (160, 120), "el del segundo monitor");
    }

    #[test]
    fn un_monitor_que_no_existe_no_rompe_lo_que_se_estaba_viendo() {
        let capturer = Guionizado::nuevo(vec![], Respuesta::Pantalla);
        let mut pump = bombeo(capturer.clone());

        assert!(matches!(
            pump.select_monitor(9).unwrap_err(),
            CaptureError::UnknownMonitor { index: 9 }
        ));
        assert_eq!(pump.monitor(), 0, "se sigue capturando el de antes");
        assert_eq!(capturer.seleccionado(), 0);

        let Pumped::Frame(f) = pump.tick(instante(1)) else {
            panic!("la sesion tiene que seguir funcionando");
        };
        assert_eq!(f.width, ANCHO as usize);
    }

    #[test]
    fn se_puede_pedir_un_fotograma_completo() {
        // Lo necesita quien acaba de conectarse: sin uno completo no hay nada
        // que decodificar.
        let mut pump = bombeo(Guionizado::nuevo(vec![], Respuesta::Pantalla));
        pump.tick(instante(1));
        pump.tick(instante(2));

        pump.request_keyframe();
        let Pumped::Frame(f) = pump.tick(instante(3)) else {
            panic!("se esperaba un fotograma");
        };
        assert!(f.keyframe, "tras pedirlo tiene que ser completo");
    }

    #[test]
    fn no_se_captura_mas_deprisa_de_lo_que_pide_el_perfil() {
        // Capturar mas deprisa de lo que se envia solo gasta CPU para tirar
        // el resultado.
        let capturer = Guionizado::nuevo(vec![], Respuesta::Pantalla);
        let mut pump = bombeo(capturer.clone());

        // Mismo instante muchas veces: solo la primera puede capturar.
        let ahora = instante(1);
        for _ in 0..10 {
            pump.tick(ahora);
        }
        assert_eq!(
            capturer.capturas(),
            1,
            "en el mismo instante solo cabe una captura"
        );
    }

    #[test]
    fn un_monitor_inexistente_no_abre_bombeo() {
        let capturer = Guionizado::nuevo(vec![], Respuesta::Pantalla);
        assert!(matches!(
            ScreenPump::new(Box::new(capturer), 9, &perfil()).unwrap_err(),
            CaptureError::UnknownMonitor { index: 9 }
        ));
    }

    /// Espera la siguiente salida del bombeo, o falla en lugar de colgarse.
    async fn siguiente(
        recibe: &mut tokio::sync::mpsc::Receiver<ScreenOutput>,
    ) -> Option<ScreenOutput> {
        tokio::time::timeout(Duration::from_secs(5), recibe.recv())
            .await
            .expect("el bombeo se quedo callado")
    }

    /// Pone el bombeo en un hilo propio, como lo hace la sesion de verdad.
    ///
    /// El bombeo se crea **dentro** del hilo, por el mismo camino que recorre
    /// el host: el capturador de Windows no se puede mover de un hilo a otro.
    fn arrancar(
        capturer: Guionizado,
        monitor: u8,
    ) -> (
        mpsc::Sender<ScreenCommand>,
        tokio::sync::mpsc::Receiver<ScreenOutput>,
        std::thread::JoinHandle<()>,
    ) {
        let (ordenes, recibe_ordenes) = mpsc::channel();
        let (envia, recibe) = tokio::sync::mpsc::channel(1);
        let hilo = std::thread::spawn(move || {
            serve(&capturer, monitor, &perfil(), &recibe_ordenes, &envia);
        });
        (ordenes, recibe, hilo)
    }

    /// Espera a que el hilo termine, o falla en lugar de colgarse.
    async fn termina(hilo: std::thread::JoinHandle<()>) {
        for _ in 0..50 {
            if hilo.is_finished() {
                hilo.join().expect("el hilo no entro en panico");
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("el bombeo no termino");
    }

    #[tokio::test]
    async fn el_bucle_entrega_fotogramas_hasta_que_se_suelta_el_mando() {
        let (ordenes, mut recibe, hilo) =
            arrancar(Guionizado::nuevo(vec![], Respuesta::Pantalla), 0);

        for i in 0..3 {
            match siguiente(&mut recibe).await {
                Some(ScreenOutput::Frame { data, duration, .. }) => {
                    assert!(!data.is_empty(), "el fotograma {i} llego vacio");
                    assert!(duration > Duration::ZERO, "un fotograma dura algo");
                }
                otro => panic!("se esperaba un fotograma, llego {otro:?}"),
            }
        }

        // Soltar el mando es como se para el bombeo.
        drop(ordenes);
        while siguiente(&mut recibe).await.is_some() {}
        termina(hilo).await;
    }

    #[tokio::test]
    async fn ningun_fotograma_codificado_se_tira_cuando_la_entrega_va_lenta() {
        // En H.264 cada fotograma normal es la diferencia con los anteriores:
        // tirar uno estropea los que vengan detras. Si la entrega va lenta hay
        // que esperar, no descartar.
        let capturer = Guionizado::nuevo(vec![], Respuesta::Pantalla);
        let (ordenes, mut recibe, hilo) = arrancar(capturer.clone(), 0);

        // Nadie recoge durante un rato.
        tokio::time::sleep(Duration::from_millis(250)).await;
        let capturadas = capturer.capturas();
        drop(ordenes);

        let mut recibidos = 0_u32;
        while let Some(salida) = siguiente(&mut recibe).await {
            if matches!(salida, ScreenOutput::Frame { .. }) {
                recibidos += 1;
            }
        }
        termina(hilo).await;

        assert!(recibidos >= 1, "no llego ningun fotograma");
        // El margen no es por si se pierde alguno: es porque uno puede ir en
        // vuelo y porque el codificador puede decidir que de algun fotograma
        // no hacia falta enviar nada. Tirar fotogramas de verdad se pasa de
        // aqui con mucho: medido, quince capturados y uno entregado.
        assert!(
            capturadas <= recibidos + 3,
            "se capturaron {capturadas} y solo llegaron {recibidos}: se estan \
             tirando fotogramas por el camino"
        );
    }

    #[tokio::test]
    async fn una_orden_imposible_se_contesta_y_el_bombeo_sigue() {
        let capturer = Guionizado::nuevo(vec![], Respuesta::Pantalla);
        let (ordenes, recibe_ordenes) = mpsc::channel();
        // La orden va antes de arrancar para que sea lo primero que se atienda.
        ordenes
            .send(ScreenCommand::Monitor(9))
            .expect("la orden entra en la cola");

        let (envia, mut recibe) = tokio::sync::mpsc::channel(1);
        let fuente = capturer.clone();
        let hilo = std::thread::spawn(move || {
            serve(&fuente, 0, &perfil(), &recibe_ordenes, &envia);
        });

        match siguiente(&mut recibe).await {
            Some(ScreenOutput::Refused { detail }) => {
                assert!(
                    detail.contains('9'),
                    "el motivo tiene que decir cual: {detail}"
                );
            }
            otro => panic!("se esperaba un rechazo, llego {otro:?}"),
        }
        assert!(
            matches!(
                siguiente(&mut recibe).await,
                Some(ScreenOutput::Frame { .. })
            ),
            "la sesion tiene que seguir viendo la pantalla"
        );
        assert_eq!(
            capturer.seleccionado(),
            0,
            "se sigue capturando el de antes"
        );

        drop(ordenes);
        while siguiente(&mut recibe).await.is_some() {}
        termina(hilo).await;
    }

    #[tokio::test]
    async fn pedir_un_fotograma_completo_surte_efecto_en_el_bucle() {
        let (ordenes, mut recibe, hilo) =
            arrancar(Guionizado::nuevo(vec![], Respuesta::Pantalla), 0);

        // El primero siempre es completo; lo que se comprueba es el segundo.
        assert!(matches!(
            siguiente(&mut recibe).await,
            Some(ScreenOutput::Frame { keyframe: true, .. })
        ));
        ordenes
            .send(ScreenCommand::Keyframe)
            .expect("el bombeo escucha");

        let mut completo = false;
        for _ in 0..5 {
            if let Some(ScreenOutput::Frame { keyframe: true, .. }) = siguiente(&mut recibe).await {
                completo = true;
                break;
            }
        }
        assert!(completo, "tras pedirlo tiene que llegar uno completo");

        drop(ordenes);
        while siguiente(&mut recibe).await.is_some() {}
        termina(hilo).await;
    }

    #[tokio::test]
    async fn la_rotura_se_anuncia_antes_de_terminar() {
        // Terminar en silencio dejaria al otro extremo mirando una imagen
        // congelada sin saber por que.
        let (_ordenes, mut recibe, hilo) = arrancar(Guionizado::nuevo(vec![], Respuesta::Fallo), 0);

        match siguiente(&mut recibe).await {
            Some(ScreenOutput::Broken { detail }) => {
                assert!(detail.contains("veces seguidas"), "motivo: {detail}");
            }
            otro => panic!("se esperaba el aviso de rotura, llego {otro:?}"),
        }
        assert!(
            siguiente(&mut recibe).await.is_none(),
            "despues de romperse no hay nada mas"
        );
        termina(hilo).await;
    }

    #[tokio::test]
    async fn si_nadie_recoge_los_fotogramas_el_bombeo_termina() {
        // Que nadie recoja significa que la sesion ya no existe: seguir
        // capturando seria gastar CPU para nada.
        let (_ordenes, recibe, hilo) = arrancar(Guionizado::nuevo(vec![], Respuesta::Pantalla), 0);
        drop(recibe);
        termina(hilo).await;
    }

    #[tokio::test]
    async fn si_no_se_puede_ni_empezar_se_dice_por_el_mismo_canal() {
        // Un host sin captura, o con un monitor que ya no existe, tiene que
        // decirlo en lugar de quedarse callado: el otro extremo estaria
        // esperando una pantalla que no va a llegar nunca.
        let (_ordenes, mut recibe, hilo) =
            arrancar(Guionizado::nuevo(vec![], Respuesta::Pantalla), 9);

        match siguiente(&mut recibe).await {
            Some(ScreenOutput::Broken { detail }) => assert!(
                detail.contains("no se pudo empezar"),
                "el motivo tiene que explicarse: {detail}"
            ),
            otro => panic!("se esperaba el aviso, llego {otro:?}"),
        }
        assert!(siguiente(&mut recibe).await.is_none());
        termina(hilo).await;
    }

    #[test]
    fn la_espera_entre_vueltas_nunca_es_cero() {
        // Una espera de cero convertiria el bucle en una rueda de hamster.
        let mut pump = bombeo(Guionizado::nuevo(vec![], Respuesta::SinCambios));
        for i in 0..5 {
            match pump.step(instante(i)) {
                Step::Wait(espera) => assert!(
                    espera >= ESPERA_MINIMA,
                    "la espera {espera:?} dejaria el bucle girando en vacio"
                ),
                otro => panic!("se esperaba una espera, llego {otro:?}"),
            }
        }
    }
}
