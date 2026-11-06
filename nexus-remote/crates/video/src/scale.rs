//! Reduccion de tamano de un fotograma antes de codificarlo.
//!
//! ## Por que hay que escalar
//!
//! Los perfiles de calidad llevan un porcentaje de escala, y no es un adorno:
//! es lo que hace que una pantalla 4K quepa en una conexion movil. Enviarla a
//! resolucion nativa sin mas gastaria el ancho de banda en pixeles que el iPad
//! ni siquiera puede mostrar.
//!
//! ## Por que se promedia y no se toma un pixel de cada N
//!
//! Lo que se transmite es **texto**: una ventana de codigo, un menu, un
//! documento. Quedarse con un pixel de cada N y tirar el resto convierte las
//! letras finas en puntos sueltos, porque el trazo de una letra mide un pixel
//! y la mitad de las veces cae justo en el pixel descartado.
//!
//! Promediando los pixeles que caen en cada destino, el trazo no desaparece:
//! se vuelve gris. Un gris es mucho mas legible que un hueco, y ademas el
//! codificador lo comprime mejor que un patron de puntos.

use nexus_capture::frame::{Frame, PixelFormat};

use crate::error::{VideoError, VideoResult};

/// Un fotograma BGRA sin relleno entre filas, listo para el codificador.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bgra {
    /// Ancho en pixeles.
    pub width: usize,
    /// Alto en pixeles.
    pub height: usize,
    /// Cuatro bytes por pixel, filas pegadas.
    pub data: Vec<u8>,
}

/// Dimensiones de salida para una escala dada.
///
/// H.264 con submuestreo 4:2:0 exige lados pares: el plano de color va a la
/// mitad de resolucion en cada eje y un lado impar no se puede partir en dos.
/// Se redondea hacia abajo, y nunca por debajo de 2.
#[must_use]
pub fn target_size(width: u32, height: u32, scale_percent: u8) -> (usize, usize) {
    let escala = f64::from(scale_percent.clamp(1, 100)) / 100.0;
    let ancho = ((f64::from(width) * escala).round() as usize).max(2) & !1;
    let alto = ((f64::from(height) * escala).round() as usize).max(2) & !1;
    (ancho.max(2), alto.max(2))
}

/// Convierte un fotograma capturado en BGRA compacto del tamano pedido.
///
/// Acepta `Bgra8` y `Rgba8`, intercambiando los canales cuando hace falta, y
/// respeta el `stride` del original, que en Windows suele traer relleno al
/// final de cada fila.
///
/// # Errors
///
/// - [`VideoError::UnsupportedFormat`] con un formato planar como `Nv12`.
/// - [`VideoError::MalformedFrame`] si los datos no llegan a lo que anuncian.
/// - [`VideoError::BadDimensions`] si el fotograma esta vacio.
pub fn prepare(frame: &Frame, scale_percent: u8) -> VideoResult<Bgra> {
    let origen_ancho = frame.width as usize;
    let origen_alto = frame.height as usize;
    if origen_ancho == 0 || origen_alto == 0 {
        return Err(VideoError::BadDimensions {
            reason: format!("{origen_ancho}x{origen_alto}"),
        });
    }

    let intercambiar = match frame.format {
        PixelFormat::Bgra8 => false,
        PixelFormat::Rgba8 => true,
        PixelFormat::Nv12 => {
            return Err(VideoError::UnsupportedFormat { format: "nv12" });
        }
    };

    let necesario = frame.stride * origen_alto;
    if frame.stride < origen_ancho * 4 || frame.data.len() < necesario {
        return Err(VideoError::MalformedFrame(format!(
            "stride {} y {} bytes para {origen_ancho}x{origen_alto}",
            frame.stride,
            frame.data.len()
        )));
    }

    let (ancho, alto) = target_size(frame.width, frame.height, scale_percent);
    let mut data = vec![0_u8; ancho * alto * 4];

    for y in 0..alto {
        // Franja de filas de origen que cae en esta fila de destino.
        let y0 = y * origen_alto / alto;
        let y1 = (((y + 1) * origen_alto).div_ceil(alto))
            .min(origen_alto)
            .max(y0 + 1);

        for x in 0..ancho {
            let x0 = x * origen_ancho / ancho;
            let x1 = (((x + 1) * origen_ancho).div_ceil(ancho))
                .min(origen_ancho)
                .max(x0 + 1);

            let (mut b, mut g, mut r) = (0_u32, 0_u32, 0_u32);
            let mut cuenta = 0_u32;
            for sy in y0..y1 {
                let fila = sy * frame.stride;
                for sx in x0..x1 {
                    let p = fila + sx * 4;
                    b += u32::from(frame.data[p]);
                    g += u32::from(frame.data[p + 1]);
                    r += u32::from(frame.data[p + 2]);
                    cuenta += 1;
                }
            }

            let destino = (y * ancho + x) * 4;
            let (c0, c2) = if intercambiar { (r, b) } else { (b, r) };
            data[destino] = (c0 / cuenta) as u8;
            data[destino + 1] = (g / cuenta) as u8;
            data[destino + 2] = (c2 / cuenta) as u8;
            data[destino + 3] = 255;
        }
    }

    Ok(Bgra {
        width: ancho,
        height: alto,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::Timestamp;

    /// Fotograma BGRA con el color que diga la funcion, por pixel.
    fn frame(
        width: u32,
        height: u32,
        stride_extra: usize,
        color: impl Fn(u32, u32) -> [u8; 4],
    ) -> Frame {
        let stride = width as usize * 4 + stride_extra;
        let mut data = vec![0_u8; stride * height as usize];
        for y in 0..height {
            for x in 0..width {
                let p = y as usize * stride + x as usize * 4;
                data[p..p + 4].copy_from_slice(&color(x, y));
            }
        }
        Frame {
            width,
            height,
            format: PixelFormat::Bgra8,
            stride,
            data,
            captured_at: Timestamp(0),
            monitor_index: 0,
        }
    }

    #[test]
    fn sin_escala_el_fotograma_sale_igual() {
        let f = frame(4, 4, 0, |x, y| [x as u8 * 10, y as u8 * 10, 200, 255]);
        let salida = prepare(&f, 100).expect("prepara");
        assert_eq!((salida.width, salida.height), (4, 4));
        // El alfa se fija a opaco; el resto tiene que coincidir.
        for i in 0..16 {
            assert_eq!(&salida.data[i * 4..i * 4 + 3], &f.data[i * 4..i * 4 + 3]);
        }
    }

    #[test]
    fn el_relleno_de_fila_no_se_cuela_en_la_imagen() {
        // En Windows cada fila suele traer bytes de relleno al final. Si se
        // ignorara el stride, la imagen saldria inclinada.
        let f = frame(4, 4, 64, |x, _| [x as u8 * 60, 0, 0, 255]);
        let salida = prepare(&f, 100).expect("prepara");
        for y in 0..4 {
            for x in 0..4_usize {
                let p = (y * 4 + x) * 4;
                assert_eq!(salida.data[p], x as u8 * 60, "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_la_mitad_se_promedian_los_cuatro_pixeles() {
        // Cuatro pixeles de un cuadrado 2x2 con valores 0, 10, 20 y 30
        // tienen que dar 15, no uno de los cuatro.
        let f = frame(2, 2, 0, |x, y| {
            let v = (y * 2 + x) as u8 * 10;
            [v, v, v, 255]
        });
        let salida = prepare(&f, 50).expect("prepara");
        assert_eq!((salida.width, salida.height), (2, 2), "el minimo son dos");

        // Con una imagen 4x4 a la mitad, cada destino promedia cuatro.
        let g = frame(4, 4, 0, |x, y| {
            let v = if (x + y) % 2 == 0 { 0 } else { 200 };
            [v, v, v, 255]
        });
        let media = prepare(&g, 50).expect("prepara");
        assert_eq!((media.width, media.height), (2, 2));
        for i in 0..4 {
            assert_eq!(
                media.data[i * 4],
                100,
                "un damero a la mitad tiene que dar gris, no negro ni blanco"
            );
        }
    }

    #[test]
    fn un_trazo_fino_no_desaparece_al_reducir() {
        // La razon de promediar: una linea de un pixel sobre fondo negro.
        // Tomando un pixel de cada dos desapareceria la mitad de las veces.
        let f = frame(8, 2, 0, |x, _| {
            if x == 3 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        });
        let salida = prepare(&f, 50).expect("prepara");
        let visible = salida.data.chunks_exact(4).any(|p| p[0] > 0 && p[0] < 255);
        assert!(
            visible,
            "el trazo tiene que quedar como gris, no desaparecer: {:?}",
            salida
                .data
                .chunks_exact(4)
                .map(|p| p[0])
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn los_lados_siempre_salen_pares() {
        // H.264 en 4:2:0 no admite lados impares.
        for (w, h, escala) in [
            (1920, 1080, 60),
            (1366, 768, 85),
            (3, 3, 100),
            (101, 77, 33),
        ] {
            let (ancho, alto) = target_size(w, h, escala);
            assert_eq!(ancho % 2, 0, "{w}x{h} al {escala} %");
            assert_eq!(alto % 2, 0, "{w}x{h} al {escala} %");
            assert!(ancho >= 2 && alto >= 2);
        }
    }

    #[test]
    fn rgba_se_convierte_a_bgra() {
        let mut f = frame(2, 2, 0, |_, _| [10, 20, 30, 255]);
        f.format = PixelFormat::Rgba8;
        let salida = prepare(&f, 100).expect("prepara");
        // Entraba R=10, G=20, B=30; tiene que salir B=30, G=20, R=10.
        assert_eq!(&salida.data[0..3], &[30, 20, 10]);
    }

    #[test]
    fn un_formato_planar_se_rechaza_en_lugar_de_leerse_mal() {
        let mut f = frame(4, 4, 0, |_, _| [0, 0, 0, 255]);
        f.format = PixelFormat::Nv12;
        assert_eq!(
            prepare(&f, 100).unwrap_err(),
            VideoError::UnsupportedFormat { format: "nv12" }
        );
    }

    #[test]
    fn un_fotograma_que_miente_sobre_su_tamano_se_rechaza() {
        let mut f = frame(4, 4, 0, |_, _| [0, 0, 0, 255]);
        f.data.truncate(10);
        assert!(matches!(
            prepare(&f, 100).unwrap_err(),
            VideoError::MalformedFrame(_)
        ));
    }

    #[test]
    fn un_fotograma_vacio_se_rechaza() {
        let mut f = frame(4, 4, 0, |_, _| [0, 0, 0, 255]);
        f.width = 0;
        assert!(matches!(
            prepare(&f, 100).unwrap_err(),
            VideoError::BadDimensions { .. }
        ));
    }
}
