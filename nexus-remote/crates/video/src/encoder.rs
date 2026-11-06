//! Codificador de pantalla a H.264.
//!
//! ## Que entra y que sale
//!
//! Entra un fotograma tal como lo entrega la captura: BGRA, con relleno al
//! final de cada fila. Sale un trozo de flujo H.264 listo para escribir en la
//! pista de video de la sesion.
//!
//! ## Por que H.264 y por que de perfil base
//!
//! Lo elige el otro extremo, no este. Un iPhone y un iPad decodifican H.264
//! de perfil base por hardware, y eso es la diferencia entre una hora de uso y
//! media hora de bateria. Cualquier codec mas moderno y mas eficiente que no
//! tuviera decodificacion por hardware en el dispositivo saldria perdiendo.
//!
//! ## Modo de contenido de pantalla
//!
//! El codificador se configura en modo pantalla y no en modo camara. Son dos
//! problemas distintos: una camara entrega ruido y movimiento continuo, y una
//! pantalla entrega zonas enormes que no cambian en absoluto separadas por
//! bordes muy nitidos. El modo pantalla aprovecha lo primero y no emborrona lo
//! segundo, que es justo lo que hace falta para leer texto.
//!
//! ## Los fotogramas completos
//!
//! Un fotograma completo es caro —puede ocupar veinte veces mas que uno
//! normal— pero es el unico que se puede decodificar sin nada anterior. Hace
//! falta uno al empezar la sesion y cada vez que el otro extremo pierde el
//! hilo. [`Encoder::request_keyframe`] los pide; el codificador tambien los
//! inserta solo de vez en cuando por si un aviso se pierde.

use nexus_capture::frame::Frame;
use nexus_core::QualityProfile;
use openh264::encoder::{
    BitRate, Encoder as OpenEncoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod,
    Profile, RateControlMode, SpsPpsStrategy, UsageType,
};
use openh264::formats::{BgraSliceU8, YUVBuffer};

use crate::error::{VideoError, VideoResult};
use crate::scale;

/// Un fotograma ya codificado, listo para la pista de video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    /// Flujo H.264 en Annex B, con sus codigos de inicio.
    pub data: Vec<u8>,
    /// Si se puede decodificar sin nada anterior.
    pub keyframe: bool,
    /// Ancho con el que se codifico, ya escalado.
    pub width: usize,
    /// Alto con el que se codifico.
    pub height: usize,
}

/// Convierte fotogramas capturados en flujo H.264.
pub trait Encoder: Send {
    /// Nombre estable del codificador, para registros y para el panel.
    fn name(&self) -> &'static str;

    /// Codifica un fotograma.
    ///
    /// Devuelve `None` cuando el codificador decide no emitir nada para este
    /// fotograma, que es lo normal si la pantalla no ha cambiado: no enviar
    /// nada es la mejor compresion posible.
    ///
    /// # Errors
    ///
    /// - [`VideoError::UnsupportedFormat`] si el fotograma es planar.
    /// - [`VideoError::MalformedFrame`] si no cuadra con sus metadatos.
    /// - [`VideoError::Codec`] si la biblioteca rechaza la operacion.
    fn encode(&mut self, frame: &Frame) -> VideoResult<Option<EncodedFrame>>;

    /// Pide que el siguiente fotograma sea completo.
    ///
    /// Se llama al abrir una sesion y cuando el otro extremo avisa de que
    /// perdio el hilo.
    fn request_keyframe(&mut self);
}

/// Codificador por software, el mismo en todas las plataformas.
///
/// Funciona en cualquier equipo sin depender del hardware. En un PC moderno,
/// Windows ofrece ademas codificacion por hardware, que gasta mucha menos CPU;
/// cuando se implemente ira detras de este mismo rasgo y este seguira siendo
/// la alternativa cuando el hardware no este disponible.
pub struct OpenH264Encoder {
    encoder: OpenEncoder,
    scale_percent: u8,
    size: (usize, usize),
    force_next: bool,
}

impl std::fmt::Debug for OpenH264Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenH264Encoder")
            .field("size", &self.size)
            .field("scale_percent", &self.scale_percent)
            .finish_non_exhaustive()
    }
}

impl OpenH264Encoder {
    /// Prepara el codificador para una pantalla y un perfil de calidad.
    ///
    /// Las dimensiones son las de la pantalla **antes** de escalar: la escala
    /// sale del perfil.
    ///
    /// # Errors
    ///
    /// - [`VideoError::BadDimensions`] si la pantalla es de tamano nulo.
    /// - [`VideoError::Codec`] si la biblioteca no arranca.
    pub fn new(width: u32, height: u32, profile: &QualityProfile) -> VideoResult<Self> {
        if width == 0 || height == 0 {
            return Err(VideoError::BadDimensions {
                reason: format!("{width}x{height}"),
            });
        }

        let (ancho, alto) = scale::target_size(width, height, profile.scale_percent);
        let config = EncoderConfig::new()
            // Modo pantalla: zonas quietas enormes y bordes nitidos.
            .usage_type(UsageType::ScreenContentRealTime)
            // Perfil base: el que decodifica un iPhone por hardware.
            .profile(Profile::Baseline)
            .bitrate(BitRate::from_bps(
                profile.target_bitrate_kbps.saturating_mul(1_000),
            ))
            .max_frame_rate(FrameRate::from_hz(f32::from(profile.target_fps)))
            // Ritmo constante: lo que importa es no pasarse del ancho de
            // banda, no que todos los fotogramas tengan la misma calidad.
            .rate_control_mode(RateControlMode::Bitrate)
            // Saltarse fotogramas antes que pasarse del limite. En una
            // pantalla, perder un fotograma se nota menos que la pausa que
            // provoca saturar la conexion.
            .skip_frames(true)
            // Repetir la cabecera en cada fotograma completo: quien se
            // conecte a mitad podra empezar a decodificar sin pedirla.
            .sps_pps_strategy(SpsPpsStrategy::IncreasingId)
            // Un fotograma completo cada pocos segundos, por si un aviso de
            // que hace falta se pierde por el camino.
            // Las dos siguientes no valen para contenido de pantalla y la
            // biblioteca las apaga sola quejandose. Se apagan aqui para que
            // la eleccion sea explicita y el registro no se llene de avisos.
            .adaptive_quantization(false)
            .background_detection(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(
                u32::from(profile.target_fps).saturating_mul(4).max(1),
            ));

        let encoder = OpenEncoder::with_api_config(openh264::OpenH264API::from_source(), config)
            .map_err(|e| VideoError::Codec(e.to_string()))?;

        Ok(Self {
            encoder,
            scale_percent: profile.scale_percent,
            size: (ancho, alto),
            force_next: true,
        })
    }

    /// Tamano con el que se esta codificando, ya escalado.
    #[must_use]
    pub const fn size(&self) -> (usize, usize) {
        self.size
    }
}

impl Encoder for OpenH264Encoder {
    fn name(&self) -> &'static str {
        "openh264"
    }

    fn encode(&mut self, frame: &Frame) -> VideoResult<Option<EncodedFrame>> {
        let bgra = scale::prepare(frame, self.scale_percent)?;
        if (bgra.width, bgra.height) != self.size {
            // La pantalla cambio de resolucion a mitad de sesion. Rehacer el
            // codificador es trabajo de quien lo usa, que tambien tiene que
            // avisar al otro extremo: aqui se dice y no se adivina.
            return Err(VideoError::BadDimensions {
                reason: format!(
                    "el fotograma mide {}x{} y el codificador se preparo para {}x{}",
                    bgra.width, bgra.height, self.size.0, self.size.1
                ),
            });
        }

        if self.force_next {
            self.encoder.force_intra_frame();
            self.force_next = false;
        }

        let yuv =
            YUVBuffer::from_rgb_source(BgraSliceU8::new(&bgra.data, (bgra.width, bgra.height)));
        let salida = self
            .encoder
            .encode(&yuv)
            .map_err(|e| VideoError::Codec(e.to_string()))?;

        // `Skip` es el codificador diciendo que no hacia falta enviar nada.
        if salida.frame_type() == FrameType::Skip {
            return Ok(None);
        }

        let data = salida.to_vec();
        if data.is_empty() {
            return Ok(None);
        }

        Ok(Some(EncodedFrame {
            data,
            keyframe: matches!(salida.frame_type(), FrameType::IDR | FrameType::I),
            width: bgra.width,
            height: bgra.height,
        }))
    }

    fn request_keyframe(&mut self) {
        self.force_next = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_capture::frame::PixelFormat;
    use nexus_core::{QualityPreset, Timestamp};
    use openh264::decoder::Decoder;
    use openh264::formats::YUVSource;

    const ANCHO: u32 = 320;
    const ALTO: u32 = 240;

    /// Perfil que no escala, para poder comparar pixel con pixel.
    fn perfil_sin_escala() -> QualityProfile {
        QualityProfile {
            scale_percent: 100,
            ..QualityProfile::from_preset(QualityPreset::Nitido)
        }
    }

    /// Una imagen con aspecto de pantalla: fondo claro, barra y bloques
    /// oscuros, que es lo que de verdad va a codificar este programa.
    fn pantalla(desplazamiento: u32) -> Frame {
        let stride = ANCHO as usize * 4;
        let mut data = vec![0_u8; stride * ALTO as usize];
        for y in 0..ALTO {
            for x in 0..ANCHO {
                let barra = y < 24;
                let bloque = (x + desplazamiento) % 64 < 20 && y > 60 && y < 180;
                let color: [u8; 4] = if barra {
                    [60, 50, 45, 255]
                } else if bloque {
                    [30, 30, 30, 255]
                } else {
                    [245, 245, 240, 255]
                };
                let p = y as usize * stride + x as usize * 4;
                data[p..p + 4].copy_from_slice(&color);
            }
        }
        Frame {
            width: ANCHO,
            height: ALTO,
            format: PixelFormat::Bgra8,
            stride,
            data,
            captured_at: Timestamp(0),
            monitor_index: 0,
        }
    }

    /// Decodifica un flujo y devuelve la imagen en RGB.
    fn decodificar(decoder: &mut Decoder, datos: &[u8]) -> Option<Vec<u8>> {
        let imagen = decoder
            .decode(datos)
            .expect("el flujo tiene que ser valido")?;
        let (w, h) = imagen.dimensions();
        let mut rgb = vec![0_u8; w * h * 3];
        imagen.write_rgb8(&mut rgb);
        Some(rgb)
    }

    /// Diferencia media por canal entre dos imagenes RGB del mismo tamano.
    fn diferencia(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        let suma: u64 = a
            .iter()
            .zip(b)
            .map(|(x, y)| u64::from(x.abs_diff(*y)))
            .sum();
        suma as f64 / a.len() as f64
    }

    /// El fotograma de entrada, en RGB, para comparar con lo decodificado.
    fn como_rgb(frame: &Frame) -> Vec<u8> {
        let mut rgb = vec![0_u8; ANCHO as usize * ALTO as usize * 3];
        for y in 0..ALTO as usize {
            for x in 0..ANCHO as usize {
                let p = y * frame.stride + x * 4;
                let d = (y * ANCHO as usize + x) * 3;
                // BGRA de origen, RGB de destino.
                rgb[d] = frame.data[p + 2];
                rgb[d + 1] = frame.data[p + 1];
                rgb[d + 2] = frame.data[p];
            }
        }
        rgb
    }

    #[test]
    fn lo_codificado_se_decodifica_y_se_parece_a_lo_que_entro() {
        // La prueba que da sentido al crate: no basta con que el codificador
        // devuelva bytes, tienen que ser video de verdad.
        let mut encoder = OpenH264Encoder::new(ANCHO, ALTO, &perfil_sin_escala()).expect("prepara");
        let entrada = pantalla(0);

        let salida = encoder
            .encode(&entrada)
            .expect("codifica")
            .expect("el primer fotograma nunca se salta");
        assert!(salida.keyframe, "el primero tiene que ser completo");
        assert!(!salida.data.is_empty());
        assert_eq!(
            (salida.width, salida.height),
            (ANCHO as usize, ALTO as usize)
        );

        let mut decoder = Decoder::new().expect("decodificador");
        let vuelta = decodificar(&mut decoder, &salida.data).expect("tiene que decodificarse");

        let error = diferencia(&como_rgb(&entrada), &vuelta);
        assert!(
            error < 12.0,
            "la imagen decodificada tiene que parecerse a la original; \
             diferencia media por canal: {error:.1}"
        );
    }

    #[test]
    fn la_comparacion_distingue_una_imagen_distinta() {
        // Si la comparacion diera por buena cualquier cosa, la prueba de
        // arriba no valdria nada.
        let entrada = pantalla(0);
        let otra = pantalla(32);
        let error = diferencia(&como_rgb(&entrada), &como_rgb(&otra));
        assert!(
            error > 12.0,
            "dos pantallas distintas tienen que diferenciarse: {error:.1}"
        );
    }

    #[test]
    fn una_pantalla_quieta_cuesta_mucho_menos_tras_el_primer_fotograma() {
        // Es la razon de ser de un codificador de video: lo que no cambia no
        // se vuelve a enviar.
        let mut encoder = OpenH264Encoder::new(ANCHO, ALTO, &perfil_sin_escala()).expect("prepara");
        let quieta = pantalla(0);

        let primero = encoder
            .encode(&quieta)
            .expect("codifica")
            .expect("el primero sale");
        let mut despues = 0_usize;
        let mut enviados = 0_usize;
        for _ in 0..5 {
            if let Some(f) = encoder.encode(&quieta).expect("codifica") {
                despues += f.data.len();
                enviados += 1;
            }
        }

        // Lo que importa es el coste **por fotograma**: uno sin cambios tiene
        // que salir mucho mas barato que el completo del principio. Si el
        // codificador decidiera no enviar ninguno, mejor todavia.
        let media = despues.checked_div(enviados).unwrap_or(0);
        assert!(
            media * 4 < primero.data.len(),
            "un fotograma sin cambios ({media} bytes de media, {enviados} enviados) \
             tendria que costar mucho menos que el completo del principio ({} bytes)",
            primero.data.len()
        );
    }

    #[test]
    fn se_puede_pedir_un_fotograma_completo_cuando_hace_falta() {
        // Lo necesita quien se conecta a mitad de sesion: sin uno completo no
        // puede empezar a decodificar nada.
        let mut encoder = OpenH264Encoder::new(ANCHO, ALTO, &perfil_sin_escala()).expect("prepara");
        encoder.encode(&pantalla(0)).expect("codifica");
        let intermedio = encoder.encode(&pantalla(4)).expect("codifica");
        assert!(
            intermedio.is_none_or(|f| !f.keyframe),
            "sin pedirlo no deberia mandar otro completo tan pronto"
        );

        encoder.request_keyframe();
        let pedido = encoder
            .encode(&pantalla(8))
            .expect("codifica")
            .expect("un completo nunca se salta");
        assert!(pedido.keyframe, "tras pedirlo, tiene que ser completo");
    }

    #[test]
    fn el_perfil_de_calidad_decide_el_tamano() {
        let fluido = QualityProfile::from_preset(QualityPreset::Fluido);
        let encoder = OpenH264Encoder::new(1920, 1080, &fluido).expect("prepara");
        let (ancho, alto) = encoder.size();
        assert_eq!(fluido.scale_percent, 60);
        assert_eq!((ancho, alto), (1152, 648));
        assert_eq!(encoder.name(), "openh264");
    }

    #[test]
    fn un_cambio_de_resolucion_se_dice_en_lugar_de_codificar_cualquier_cosa() {
        let mut encoder = OpenH264Encoder::new(ANCHO, ALTO, &perfil_sin_escala()).expect("prepara");
        let mut otra = pantalla(0);
        otra.width = 160;
        otra.stride = 160 * 4;
        otra.data.truncate(160 * 4 * ALTO as usize);

        assert!(matches!(
            encoder.encode(&otra).unwrap_err(),
            VideoError::BadDimensions { .. }
        ));
    }

    #[test]
    fn una_pantalla_de_tamano_nulo_no_abre_codificador() {
        assert!(matches!(
            OpenH264Encoder::new(0, 1080, &perfil_sin_escala()).unwrap_err(),
            VideoError::BadDimensions { .. }
        ));
    }
}
