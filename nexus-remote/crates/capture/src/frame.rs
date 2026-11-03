//! Descripcion de un fotograma capturado.

use nexus_core::Timestamp;
use serde::{Deserialize, Serialize};

use crate::error::{CaptureError, CaptureResult};

/// Disposicion de los bytes de cada pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    /// Cuatro bytes por pixel en orden azul, verde, rojo, alfa.
    ///
    /// Es lo que entrega Windows Graphics Capture.
    Bgra8,
    /// Cuatro bytes por pixel en orden rojo, verde, azul, alfa.
    Rgba8,
    /// Luminancia y croma en planos separados, submuestreo 4:2:0.
    ///
    /// Es la entrada que esperan los codificadores de video.
    Nv12,
}

impl PixelFormat {
    /// Bytes por pixel, para los formatos de un solo plano.
    ///
    /// Devuelve `None` para los formatos planares como `Nv12`, cuyo tamano no
    /// se expresa como un numero entero de bytes por pixel.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> Option<usize> {
        match self {
            Self::Bgra8 | Self::Rgba8 => Some(4),
            Self::Nv12 => None,
        }
    }

    /// Tamano en bytes de un fotograma de las dimensiones dadas.
    #[must_use]
    pub const fn frame_size(self, width: u32, height: u32) -> usize {
        let pixels = width as usize * height as usize;
        match self {
            Self::Bgra8 | Self::Rgba8 => pixels * 4,
            // Un plano de luminancia completo mas un plano de croma a la mitad
            // de resolucion en cada eje: 1,5 bytes por pixel.
            Self::Nv12 => pixels + pixels / 2,
        }
    }
}

/// Un fotograma con sus datos y metadatos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Ancho en pixeles.
    pub width: u32,
    /// Alto en pixeles.
    pub height: u32,
    /// Formato de los datos.
    pub format: PixelFormat,
    /// Bytes por fila, que puede ser mayor que `width * bpp` por alineacion.
    pub stride: usize,
    /// Datos del fotograma.
    pub data: Vec<u8>,
    /// Instante de captura.
    pub captured_at: Timestamp,
    /// Monitor del que proviene.
    pub monitor_index: u8,
}

impl Frame {
    /// Comprueba que los metadatos son coherentes con los datos.
    ///
    /// Se valida en la frontera entre el backend de plataforma y el resto del
    /// sistema: un fotograma con `stride` o tamano incoherentes provocaria
    /// lecturas fuera de rango mas adelante.
    ///
    /// # Errors
    ///
    /// [`CaptureError::System`] describiendo la incoherencia.
    pub fn validate(&self) -> CaptureResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(CaptureError::System(format!(
                "dimensiones nulas: {}x{}",
                self.width, self.height
            )));
        }

        if let Some(bpp) = self.format.bytes_per_pixel() {
            let min_stride = self.width as usize * bpp;
            if self.stride < min_stride {
                return Err(CaptureError::System(format!(
                    "stride {} menor que el minimo {min_stride}",
                    self.stride
                )));
            }
            let required = self.stride * self.height as usize;
            if self.data.len() < required {
                return Err(CaptureError::System(format!(
                    "faltan datos: {} bytes para {required} necesarios",
                    self.data.len()
                )));
            }
        } else {
            let required = self.format.frame_size(self.width, self.height);
            if self.data.len() < required {
                return Err(CaptureError::System(format!(
                    "faltan datos: {} bytes para {required} necesarios",
                    self.data.len()
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, format: PixelFormat, stride: usize, len: usize) -> Frame {
        Frame {
            width,
            height,
            format,
            stride,
            data: vec![0; len],
            captured_at: Timestamp(1),
            monitor_index: 0,
        }
    }

    #[test]
    fn packed_formats_report_four_bytes_per_pixel() {
        assert_eq!(PixelFormat::Bgra8.bytes_per_pixel(), Some(4));
        assert_eq!(PixelFormat::Rgba8.bytes_per_pixel(), Some(4));
        assert_eq!(PixelFormat::Nv12.bytes_per_pixel(), None);
    }

    #[test]
    fn frame_sizes_match_the_format() {
        assert_eq!(
            PixelFormat::Bgra8.frame_size(1_920, 1_080),
            1_920 * 1_080 * 4
        );
        assert_eq!(
            PixelFormat::Nv12.frame_size(1_920, 1_080),
            1_920 * 1_080 * 3 / 2
        );
    }

    #[test]
    fn a_coherent_frame_validates() {
        let f = frame(640, 480, PixelFormat::Bgra8, 640 * 4, 640 * 480 * 4);
        f.validate().expect("deberia ser valido");
    }

    #[test]
    fn padded_stride_is_accepted() {
        // Windows suele alinear las filas, dejando stride mayor que el minimo.
        let stride = 640 * 4 + 64;
        let f = frame(640, 480, PixelFormat::Bgra8, stride, stride * 480);
        f.validate().expect("el relleno de fila es normal");
    }

    #[test]
    fn a_stride_below_the_minimum_is_rejected() {
        let f = frame(640, 480, PixelFormat::Bgra8, 100, 640 * 480 * 4);
        assert!(f.validate().is_err());
    }

    #[test]
    fn a_truncated_buffer_is_rejected() {
        let f = frame(640, 480, PixelFormat::Bgra8, 640 * 4, 1_000);
        assert!(f.validate().is_err());
    }

    #[test]
    fn zero_sized_frames_are_rejected() {
        assert!(frame(0, 480, PixelFormat::Bgra8, 0, 0).validate().is_err());
        assert!(
            frame(640, 0, PixelFormat::Bgra8, 2_560, 0)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn planar_frames_are_validated_by_total_size() {
        let required = PixelFormat::Nv12.frame_size(640, 480);
        frame(640, 480, PixelFormat::Nv12, 640, required)
            .validate()
            .expect("tamano suficiente");
        assert!(
            frame(640, 480, PixelFormat::Nv12, 640, required - 1)
                .validate()
                .is_err()
        );
    }
}
