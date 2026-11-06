//! La pista de la pantalla, entre dos extremos de verdad.
//!
//! ## Que prueba esto, y que no
//!
//! **Prueba el camino.** Dos conexiones WebRTC reales negocian una pista de
//! video, y lo que el host escribe en ella llega al otro extremo: negociacion
//! SDP con una seccion de medios, troceado en paquetes RTP, cifrado
//! DTLS-SRTP y entrega. Todo eso es trabajo de la biblioteca y de la
//! configuracion, y hasta ahora nadie lo habia ejecutado en este proyecto.
//!
//! **No prueba la transmision de pantalla.** Faltan las dos puntas: capturar
//! la pantalla, que solo funciona en Windows, y codificar los fotogramas, que
//! todavia no esta hecho. Lo que viaja aqui son bytes con la forma de una
//! unidad H.264, no una imagen.
//!
//! Esa distincion importa y no se difumina. Lo que esta prueba demuestra es
//! que **el camino por el que viajara la pantalla funciona**, que es lo que
//! habia que averiguar antes de construir encima: si la pista no negociara o
//! los paquetes no llegaran, el codificador no serviria de nada.

use std::time::Duration;

use nexus_transport::ice::IceConfiguration;
use nexus_transport::peer::{LinkPhase, NetworkOptions, PeerEvent, PeerRole, PeerSession};
use nexus_video::Encoder as _;
use openh264::formats::YUVSource as _;
use rtc::rtp::packetizer::Depacketizer as _;

/// Tope de la prueba. ICE y DTLS tardan, pero no tanto.
const TIMEOUT: Duration = Duration::from_secs(45);

/// Los dos extremos en la misma maquina, sin servidores.
///
/// Se anuncia `127.0.0.1` como direccion propia: es el mismo montaje que usa
/// quien aloja el servidor en su PC, y aqui hace que los dos se encuentren
/// sin STUN ni TURN.
fn opciones() -> NetworkOptions {
    NetworkOptions {
        udp_port: 0,
        public_ip: Some("127.0.0.1".to_owned()),
    }
}

/// Una unidad H.264 con la forma que espera el troceador.
///
/// Empieza por el codigo de inicio de Annex B y sigue con una cabecera de
/// unidad. **No es video**: son bytes reconocibles con la forma correcta,
/// para poder comprobar que llegan tal cual.
fn unidad(relleno: u8, bytes: usize) -> Vec<u8> {
    let mut datos = vec![0x00, 0x00, 0x00, 0x01, 0x41];
    datos.extend(std::iter::repeat_n(relleno, bytes));
    datos
}

/// Lo que recoge el extremo que recibe.
#[derive(Default)]
struct Recibido {
    pista_abierta: bool,
    /// Cada paquete con la marca de tiempo de su fotograma.
    paquetes: Vec<(u32, Vec<u8>)>,
    /// Marcas de tiempo distintas vistas, en orden de aparicion.
    marcas: Vec<u32>,
}

/// Junta las cargas de todos los paquetes de un mismo fotograma.
///
/// Un fotograma grande viaja troceado: esto es lo que haria el otro extremo
/// antes de pasarselo al decodificador.
fn recomponer(marca: u32, paquetes: &[(u32, Vec<u8>)]) -> Vec<u8> {
    paquetes
        .iter()
        .filter(|(t, _)| *t == marca)
        .flat_map(|(_, p)| p.clone())
        .collect()
}

/// Negocia la sesion y comprueba que el video entra en el acuerdo.
async fn negociar(host: &PeerSession, ipad: &PeerSession) {
    let oferta = host.create_offer().await.expect("crea la oferta");
    assert!(
        oferta.contains("m=video"),
        "la oferta tiene que declarar una seccion de video:\n{oferta}"
    );
    assert!(
        oferta.contains("H264") || oferta.contains("h264"),
        "la oferta tiene que ofrecer H.264:\n{oferta}"
    );

    let respuesta = ipad.accept_offer(&oferta).await.expect("contesta");
    assert!(
        respuesta.contains("m=video"),
        "el otro extremo tiene que aceptar el video:\n{respuesta}"
    );
    host.accept_answer(&respuesta).await.expect("acepta");
}

/// Pasa candidatos, envia los fotogramas al conectar y recoge lo que llega.
async fn intercambiar(
    host: &mut PeerSession,
    ipad: &mut PeerSession,
    pantalla: &nexus_transport::peer::ScreenSender,
    fotogramas: &[Vec<u8>],
) -> Recibido {
    let mut recibido = Recibido::default();
    let mut enviados = false;

    loop {
        tokio::select! {
            evento = host.next_event() => {
                match evento.expect("la conexion del host sigue viva") {
                    PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                        ipad.add_remote_candidate(candidate, sdp_mid)
                            .await
                            .expect("el dispositivo anade el camino");
                    }
                    PeerEvent::Phase(LinkPhase::Connected) if !enviados => {
                        enviados = true;
                        for datos in fotogramas {
                            pantalla
                                .send_frame(datos, Duration::from_millis(33))
                                .await
                                .expect("escribe el fotograma en la pista");
                        }
                    }
                    _ => {}
                }
            }
            evento = ipad.next_event() => {
                match evento.expect("la conexion del dispositivo sigue viva") {
                    PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                        host.add_remote_candidate(candidate, sdp_mid)
                            .await
                            .expect("el host anade el camino");
                    }
                    PeerEvent::TrackOpen { .. } => recibido.pista_abierta = true,
                    PeerEvent::ScreenPacket { payload, timestamp, .. } => {
                        if !recibido.marcas.contains(&timestamp) {
                            recibido.marcas.push(timestamp);
                        }
                        recibido.paquetes.push((timestamp, payload));
                        if recibido.marcas.len() == fotogramas.len() {
                            // Un respiro para los trozos que falten del
                            // ultimo fotograma.
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            return recibido;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn un_fotograma_escrito_en_la_pista_llega_al_otro_extremo() {
    let prueba = async {
        let ice = IceConfiguration::new(vec![]);
        let red = opciones();

        let mut host = PeerSession::open_with(&ice, PeerRole::Offerer, &red)
            .await
            .expect("el host abre la conexion");
        let mut ipad = PeerSession::open_with(&ice, PeerRole::Answerer, &red)
            .await
            .expect("el dispositivo abre la conexion");

        // Solo quien ofrece —el host— tiene pista de salida.
        let pantalla = host
            .screen()
            .expect("el host deberia tener pista de pantalla");
        assert!(
            ipad.screen().is_none(),
            "quien recibe no envia pantalla: no deberia tener pista de salida"
        );

        negociar(&host, &ipad).await;

        let fotogramas = [unidad(0xAA, 200), unidad(0xBB, 300), unidad(0xCC, 150)];
        let recibido = intercambiar(&mut host, &mut ipad, &pantalla, &fotogramas).await;

        // --- Lo que tiene que haber pasado ---
        assert!(
            recibido.pista_abierta,
            "el dispositivo tiene que haber visto abrirse la pista"
        );
        assert_eq!(
            recibido.marcas.len(),
            fotogramas.len(),
            "tienen que llegar los tres fotogramas, cada uno con su marca de tiempo"
        );

        for (i, esperado) in fotogramas.iter().enumerate() {
            let recompuesto = recomponer(recibido.marcas[i], &recibido.paquetes);
            // El codigo de inicio de Annex B no viaja: RTP ya delimita cada
            // unidad, asi que se compara lo que va detras.
            assert_eq!(
                recompuesto,
                esperado[4..],
                "el fotograma {i} tiene que llegar tal cual"
            );
        }

        let _ = host.close().await;
        let _ = ipad.close().await;
    };

    tokio::time::timeout(TIMEOUT, prueba)
        .await
        .expect("la prueba no deberia agotar el tiempo");
}

// ---------------------------------------------------------------------------
// La cadena completa: codificar, enviar, recibir, decodificar.
// ---------------------------------------------------------------------------

/// Una imagen con aspecto de pantalla: barra arriba y bloques como texto.
fn captura(desplazamiento: u32) -> nexus_capture::frame::Frame {
    const ANCHO: u32 = 320;
    const ALTO: u32 = 240;
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
    nexus_capture::frame::Frame {
        width: ANCHO,
        height: ALTO,
        format: nexus_capture::frame::PixelFormat::Bgra8,
        stride,
        data,
        captured_at: nexus_core::Timestamp(0),
        monitor_index: 0,
    }
}

/// Pasa la captura a RGB, para comparar con lo que vuelva decodificado.
fn como_rgb(frame: &nexus_capture::frame::Frame) -> Vec<u8> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut rgb = vec![0_u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let p = y * frame.stride + x * 4;
            let d = (y * w + x) * 3;
            rgb[d] = frame.data[p + 2];
            rgb[d + 1] = frame.data[p + 1];
            rgb[d + 2] = frame.data[p];
        }
    }
    rgb
}

/// Diferencia media por canal entre dos imagenes RGB del mismo tamano.
fn diferencia(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "las dos imagenes tienen que medir igual");
    let suma: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    suma as f64 / a.len() as f64
}

/// Recompone los fotogramas que llegan troceados en paquetes RTP.
///
/// Es lo que tendra que hacer el cliente de iPhone antes de decodificar: un
/// fotograma grande no cabe en un paquete y viaja partido, con la marca del
/// ultimo trozo indicando donde acaba.
#[derive(Default)]
struct Recomponedor {
    depacketizador: rtc::rtp::codec::h264::H264Packet,
    actual: Vec<u8>,
    /// Fotogramas completos, en orden.
    completos: Vec<Vec<u8>>,
}

impl Recomponedor {
    fn paquete(&mut self, carga: &[u8], ultimo: bool) {
        if let Ok(trozo) = self
            .depacketizador
            .depacketize(&bytes::Bytes::copy_from_slice(carga))
        {
            self.actual.extend_from_slice(&trozo);
        }
        if ultimo && !self.actual.is_empty() {
            self.completos.push(std::mem::take(&mut self.actual));
        }
    }
}

/// Decodifica lo que llego y lo compara con las pantallas originales.
///
/// H.264 pierde informacion a proposito, asi que no puede salir identico: lo
/// que se comprueba es que lo que vuelve **se ve como** lo que entro.
fn comprobar_que_se_ven(flujos: &[Vec<u8>], capturas: &[nexus_capture::frame::Frame]) {
    let mut decoder = openh264::decoder::Decoder::new().expect("decodificador");
    let mut vistos = 0;

    for (i, flujo) in flujos.iter().enumerate() {
        let Some(imagen) = decoder
            .decode(flujo)
            .expect("el flujo tiene que ser valido")
        else {
            // El decodificador puede necesitar mas de un fotograma antes de
            // entregar el primero; lo que no puede es fallar.
            continue;
        };

        let (w, h) = imagen.dimensions();
        let mut rgb = vec![0_u8; w * h * 3];
        imagen.write_rgb8(&mut rgb);

        let error = diferencia(&como_rgb(&capturas[i]), &rgb);
        assert!(
            error < 12.0,
            "el fotograma {i} tiene que verse como la pantalla original; \
             diferencia media por canal: {error:.1}"
        );
        vistos += 1;
    }

    assert_eq!(
        vistos,
        capturas.len(),
        "los tres fotogramas tienen que poder verse al otro lado"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn una_pantalla_codificada_llega_y_se_puede_ver_al_otro_lado() {
    let prueba = async {
        let ice = IceConfiguration::new(vec![]);
        let red = opciones();

        let mut host = PeerSession::open_with(&ice, PeerRole::Offerer, &red)
            .await
            .expect("el host abre la conexion");
        let mut ipad = PeerSession::open_with(&ice, PeerRole::Answerer, &red)
            .await
            .expect("el dispositivo abre la conexion");
        let pantalla = host.screen().expect("el host envia la pantalla");

        negociar(&host, &ipad).await;

        // El host codifica tres pantallas con el codificador de verdad.
        let perfil = nexus_core::QualityProfile {
            scale_percent: 100,
            ..nexus_core::QualityProfile::from_preset(nexus_core::QualityPreset::Nitido)
        };
        let capturas = [captura(0), captura(24), captura(48)];
        let mut encoder =
            nexus_video::OpenH264Encoder::new(320, 240, &perfil).expect("prepara el codificador");
        let codificados: Vec<Vec<u8>> = capturas
            .iter()
            .map(|c| {
                encoder
                    .encode(c)
                    .expect("codifica")
                    .expect("ninguna de las tres se salta")
                    .data
            })
            .collect();

        // --- Caminos, envio y recogida ---
        let mut recomponedor = Recomponedor::default();
        let mut enviados = false;

        loop {
            tokio::select! {
                evento = host.next_event() => {
                    match evento.expect("la conexion del host sigue viva") {
                        PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                            ipad.add_remote_candidate(candidate, sdp_mid)
                                .await
                                .expect("anade el camino");
                        }
                        PeerEvent::Phase(LinkPhase::Connected) if !enviados => {
                            enviados = true;
                            for datos in &codificados {
                                pantalla
                                    .send_frame(datos, Duration::from_millis(33))
                                    .await
                                    .expect("envia el fotograma codificado");
                            }
                        }
                        _ => {}
                    }
                }
                evento = ipad.next_event() => {
                    match evento.expect("la conexion del dispositivo sigue viva") {
                        PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                            host.add_remote_candidate(candidate, sdp_mid)
                                .await
                                .expect("anade el camino");
                        }
                        PeerEvent::ScreenPacket { payload, last, .. } => {
                            recomponedor.paquete(&payload, last);
                            if recomponedor.completos.len() == capturas.len() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // --- Lo que tiene que haber pasado ---
        assert_eq!(
            recomponedor.completos.len(),
            capturas.len(),
            "tienen que llegar los tres fotogramas"
        );

        comprobar_que_se_ven(&recomponedor.completos, &capturas);

        let _ = host.close().await;
        let _ = ipad.close().await;
    };

    tokio::time::timeout(TIMEOUT, prueba)
        .await
        .expect("la prueba no deberia agotar el tiempo");
}
