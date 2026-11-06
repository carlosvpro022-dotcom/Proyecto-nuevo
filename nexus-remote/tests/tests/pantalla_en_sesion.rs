//! La pantalla del host llegando a una sesion de verdad.
//!
//! ## Que prueba esto, y que no
//!
//! **Prueba la cadena completa de un extremo a otro.** El bucle del producto
//! (`HostRuntime`) atiende una peticion de sesion, negocia WebRTC, comprueba
//! la atadura, arranca su hilo de captura, escala, codifica en H.264, trocea
//! en paquetes RTP y los cifra; al otro lado se recomponen los paquetes y se
//! decodifican hasta volver a tener una imagen. Nada de eso esta simulado.
//!
//! **Lo unico que no es real es el origen de los pixeles.** Capturar la
//! pantalla solo funciona en Windows, asi que aqui el host captura de un
//! capturador sintetico, que entrega color plano del tamano de la pantalla
//! declarada. Esa distincion no se difumina: lo que esta prueba demuestra es
//! que el camino entero funciona y que la sesion recibe **lo que el host
//! esta mostrando**, no que el capturador de Windows funcione.
//!
//! ## Lo que se comprueba
//!
//! - Que la pantalla llega, se recompone y se decodifica como imagen.
//! - Que lo que se ve es lo que el host tenia (color plano, no ruido).
//! - Que cambiar la calidad por el canal de control cambia de verdad lo que
//!   sale por la pista de video, porque el hilo de captura rehace el
//!   codificador.
//! - Y lo que mas importa: que **no sale un solo paquete de pantalla antes de
//!   comprobar quien hay al otro lado**.

mod comun;

use std::time::Duration;

use comun::{Device, HostDeProducto, LadoControlador, STEP_TIMEOUT, TestServer};
use comun::{atender_senalizacion, enviar_candidato};
use nexus_core::protocol::Envelope;
use nexus_core::protocol::control::{ControlMessage, ControlRequest, ControlResponse};
use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};
use nexus_core::{PermissionSet, QualityPreset, QualityProfile, RequestId, SessionId};
use nexus_security::registry::DeviceRecord;
use nexus_transport::client::SignalingClient;
use nexus_transport::peer::{CONTROL_CHANNEL, PeerEvent, PeerRole, PeerSession};
use openh264::formats::YUVSource as _;
use rtc::rtp::packetizer::Depacketizer as _;

/// Tope de la prueba. ICE, DTLS y el codificador tardan, pero no tanto.
const TIMEOUT: Duration = Duration::from_secs(90);

/// Pantalla que el host declara (y que el capturador sintetico entrega).
const PANTALLA: (u32, u32) = (1_920, 1_080);

/// Escala a la que se pide bajar a mitad de sesion.
const ESCALA_PEDIDA: u8 = 50;

/// Cuanto se mira, despues de pedir la pantalla, antes de dar por bueno que no
/// llega nada.
const VENTANA: Duration = Duration::from_secs(3);

/// Recompone los fotogramas que llegan troceados en paquetes RTP.
///
/// Es lo que tendra que hacer el cliente de iPhone antes de decodificar: un
/// fotograma no cabe en un paquete y viaja partido, con la marca del ultimo
/// trozo indicando donde acaba.
#[derive(Default)]
struct Recomponedor {
    depacketizador: rtc::rtp::codec::h264::H264Packet,
    actual: Vec<u8>,
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

/// Un dispositivo emparejado con acceso desatendido.
fn emparejado(device: &Device) -> DeviceRecord {
    DeviceRecord::new(
        device.info.clone(),
        device.identity.public_key(),
        PermissionSet::standard(),
        true,
        nexus_core::Timestamp::now(),
    )
}

/// Pide sesion y devuelve el identificador que conceda el host.
async fn pedir_sesion(sig: &mut SignalingClient, host: nexus_core::DeviceId) -> SessionId {
    sig.send(ClientToServer::RequestSession {
        host_id: host,
        requested_permissions: PermissionSet::standard(),
    })
    .await
    .expect("pide sesion");

    match sig
        .recv_timeout(STEP_TIMEOUT)
        .await
        .expect("el host contesta")
    {
        ServerToClient::SessionAccepted { session_id, .. } => session_id,
        otro => panic!("la sesion deberia aceptarse, llego {otro:?}"),
    }
}

/// El perfil con el que se pide bajar la escala a mitad de sesion.
fn perfil_reducido() -> QualityProfile {
    QualityProfile {
        scale_percent: ESCALA_PEDIDA,
        ..QualityProfile::from_preset(QualityPreset::Equilibrado)
    }
}

/// Decodifica un fotograma y devuelve su tamano y lo plano que es.
///
/// Lo segundo importa: la pantalla sintetica es de color plano, asi que si lo
/// que llega tiene mucho contraste no es la pantalla del host, es ruido.
fn medir(decoder: &mut openh264::decoder::Decoder, flujo: &[u8]) -> Option<((usize, usize), u8)> {
    let imagen = decoder
        .decode(flujo)
        .expect("el flujo tiene que ser valido")?;
    let (w, h) = imagen.dimensions();
    let mut rgb = vec![0_u8; w * h * 3];
    imagen.write_rgb8(&mut rgb);
    let max = rgb.iter().copied().max().unwrap_or(0);
    let min = rgb.iter().copied().min().unwrap_or(0);
    Some(((w, h), max - min))
}

/// Comprueba un tamano con holgura de un macrobloque.
///
/// H.264 codifica en bloques de 16 pixeles y el tamano exacto que declara el
/// decodificador depende del recorte; lo que se comprueba es la resolucion,
/// no el pixel.
fn parecido(real: (usize, usize), esperado: (usize, usize)) -> bool {
    real.0.abs_diff(esperado.0) <= 16 && real.1.abs_diff(esperado.1) <= 16
}

/// Lo que se vio al otro lado.
#[derive(Default)]
struct Visto {
    /// Tamano de cada fotograma que se pudo decodificar.
    tamanos: Vec<(usize, usize)>,
    /// Contraste de cada uno: la pantalla de prueba es de color plano.
    contrastes: Vec<u8>,
}

/// Lleva el lado del controlador hasta ver la pantalla a las dos escalas.
///
/// Pide la pantalla en cuanto hay canal de control, y cuando ya ha visto unos
/// cuantos fotogramas pide bajar la escala, para que una sola sesion demuestre
/// las dos cosas: que la pantalla llega y que obedece.
async fn ver_la_pantalla(
    sig: &mut SignalingClient,
    peer: &mut PeerSession,
    lado: &LadoControlador<'_>,
    grande: (usize, usize),
    pequena: (usize, usize),
) -> Visto {
    let mut recomponedor = Recomponedor::default();
    let mut decoder = openh264::decoder::Decoder::new().expect("decodificador");
    let mut visto = Visto::default();
    let mut pedido = false;
    let fin = tokio::time::Instant::now() + TIMEOUT;

    loop {
        tokio::select! {
            () = tokio::time::sleep_until(fin) => {
                panic!(
                    "la pantalla no llego a tiempo; tamanos vistos: {:?}, \
                     fotogramas recompuestos: {}",
                    visto.tamanos,
                    recomponedor.completos.len()
                );
            }
            mensaje = sig.recv() => {
                let mensaje = mensaje.expect("el canal con el servidor sigue abierto");
                assert!(
                    atender_senalizacion(sig, peer, mensaje, lado).await,
                    "el host se fue antes de enviar la pantalla"
                );
            }
            evento = peer.next_event() => {
                match evento.expect("la conexion sigue viva") {
                    PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                        enviar_candidato(sig, lado.session_id, candidate, sdp_mid).await;
                    }
                    // Aqui es donde se pide la pantalla: con el canal de
                    // control abierto, este extremo ya esta listo para verla.
                    PeerEvent::ChannelOpen { label } if label == CONTROL_CHANNEL => {
                        mandar(peer, ControlRequest::StartScreen).await;
                    }
                    PeerEvent::ScreenPacket { payload, last, .. } => {
                        recomponedor.paquete(&payload, last);
                        let Some(flujo) = recomponedor.completos.pop() else { continue };
                        if let Some((tamano, contraste)) = medir(&mut decoder, &flujo) {
                            visto.tamanos.push(tamano);
                            visto.contrastes.push(contraste);
                        }

                        let vistos_grandes =
                            visto.tamanos.iter().filter(|t| parecido(**t, grande)).count();
                        if !pedido && vistos_grandes >= 2 {
                            mandar(
                                peer,
                                ControlRequest::SetQuality { profile: perfil_reducido() },
                            )
                            .await;
                            pedido = true;
                        }
                        if pedido && visto.tamanos.iter().any(|t| parecido(*t, pequena)) {
                            return visto;
                        }
                    }
                    PeerEvent::Message { label, data } if label == CONTROL_CHANNEL => {
                        comprobar_que_no_rechazo(&data);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Envia una peticion por el canal de control.
async fn mandar(peer: &PeerSession, request: ControlRequest) {
    let sobre = Envelope::new(ControlMessage::request(RequestId::FIRST, request));
    peer.send(
        CONTROL_CHANNEL,
        sobre.encode().expect("codifica").as_bytes(),
    )
    .await
    .expect("el canal de control esta abierto");
}

/// Falla si el host rechazo algo: sin esto, un rechazo se veria como silencio.
fn comprobar_que_no_rechazo(data: &[u8]) {
    let texto = std::str::from_utf8(data).expect("el control es texto");
    let sobre: Envelope<ControlMessage> = Envelope::decode(texto).expect("el control se entiende");
    if let ControlMessage::Response {
        response: ControlResponse::Denied { code, message },
        ..
    } = sobre.payload
    {
        panic!("el host rechazo una peticion: {code} ({message})");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn la_pantalla_llega_a_la_sesion_y_obedece_al_cambio_de_calidad() {
    let server = TestServer::start().await;
    let ipad = Device::controller("iPad de Carlos");
    let host = HostDeProducto::arrancar_con_pantalla(
        &server,
        vec![emparejado(&ipad)],
        HostDeProducto::layout(),
    )
    .await;

    let (mut sig, aceptado) = ipad.connect_ok(&server, None).await;
    let session_id = pedir_sesion(&mut sig, host.id()).await;
    let mut peer = PeerSession::open(&aceptado.ice, PeerRole::Answerer)
        .await
        .expect("abre la conexion");
    let lado = LadoControlador {
        session_id,
        yo: ipad.id(),
        host: host.id(),
        firmante: &ipad.identity,
    };

    let equilibrado = QualityProfile::from_preset(QualityPreset::Equilibrado);
    let grande = nexus_video::target_size(PANTALLA.0, PANTALLA.1, equilibrado.scale_percent);
    let pequena = nexus_video::target_size(PANTALLA.0, PANTALLA.1, ESCALA_PEDIDA);

    let visto = ver_la_pantalla(&mut sig, &mut peer, &lado, grande, pequena).await;

    assert!(
        visto.tamanos.iter().any(|t| parecido(*t, grande)),
        "tenia que llegar la pantalla a la escala de partida {grande:?}; \
         llegaron {:?}",
        visto.tamanos
    );
    assert!(
        visto.tamanos.iter().any(|t| parecido(*t, pequena)),
        "tras pedir el {ESCALA_PEDIDA} % tenia que llegar a {pequena:?}; \
         llegaron {:?}",
        visto.tamanos
    );
    assert!(
        visto.contrastes.iter().all(|contraste| *contraste <= 40),
        "la pantalla del host es de color plano: lo que llego tiene contrastes \
         de {:?}, que no es lo que se estaba mostrando",
        visto.contrastes
    );

    let _ = peer.close().await;
    host.parar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sin_firmar_la_atadura_no_sale_un_solo_paquete_de_pantalla() {
    // Esto es lo que separa un escritorio remoto de una camara abierta. El
    // dispositivo esta emparejado, la conexion cifrada se establece y **pide la
    // pantalla** por el canal de control. Lo que no hace es firmar la atadura
    // de sesion, que es lo que demuestra que quien habla por este canal
    // cifrado es quien dice ser. Mientras eso no pase, la pantalla no sale del
    // PC.
    let server = TestServer::start().await;
    let ipad = Device::controller("iPad de Carlos");
    // Una pantalla pequena: aqui se mide la puerta, no la imagen.
    let host = HostDeProducto::arrancar_con_pantalla(
        &server,
        vec![emparejado(&ipad)],
        HostDeProducto::pantalla_pequena(),
    )
    .await;

    let (mut sig, aceptado) = ipad.connect_ok(&server, None).await;
    let session_id = pedir_sesion(&mut sig, host.id()).await;
    let mut peer = PeerSession::open(&aceptado.ice, PeerRole::Answerer)
        .await
        .expect("abre la conexion");

    let mut pidio = false;
    let mut paquetes = 0_u32;
    // Un plazo absoluto y no un `sleep` por vuelta: un `sleep` creado en cada
    // vuelta del bucle vuelve a empezar con cada evento, y mientras lleguen
    // eventos no se cumple nunca.
    let mut hasta = tokio::time::Instant::now() + TIMEOUT;

    loop {
        tokio::select! {
            () = tokio::time::sleep_until(hasta) => break,
            mensaje = sig.recv() => {
                match mensaje.expect("el canal con el servidor sigue abierto") {
                    // La oferta se contesta, para que la conexion se establezca
                    // de verdad. Lo que **no** se envia es la atadura firmada.
                    ServerToClient::Offer { sdp, .. } => {
                        let answer = peer.accept_offer(&sdp).await.expect("contesta");
                        sig.send(ClientToServer::Answer { session_id, sdp: answer })
                            .await
                            .expect("envia la respuesta");
                    }
                    ServerToClient::IceCandidate { candidate, sdp_mid, .. } => {
                        peer.add_remote_candidate(candidate, sdp_mid)
                            .await
                            .expect("anade camino");
                    }
                    ServerToClient::PeerGone { .. } => break,
                    _ => {}
                }
            }
            evento = peer.next_event() => {
                match evento.expect("la conexion sigue viva") {
                    PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                        enviar_candidato(&mut sig, session_id, candidate, sdp_mid).await;
                    }
                    PeerEvent::ScreenPacket { .. } => paquetes += 1,
                    // Que el canal se abra prueba que el camino cifrado esta
                    // montado: ya habria por donde enviar la pantalla.
                    PeerEvent::ChannelOpen { label } if label == CONTROL_CHANNEL => {
                        mandar(&peer, ControlRequest::StartScreen).await;
                        pidio = true;
                        // Desde que se pide, se mira durante un rato corto.
                        hasta = tokio::time::Instant::now() + VENTANA;
                    }
                    _ => {}
                }
            }
        }
    }

    assert!(
        pidio,
        "la conexion cifrada tenia que establecerse y la pantalla tenia que \
         pedirse para que esta prueba signifique algo"
    );
    assert_eq!(
        paquetes, 0,
        "llegaron {paquetes} paquetes de pantalla sin haber firmado la atadura"
    );

    let _ = peer.close().await;
    host.parar().await;
}
