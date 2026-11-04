//! Un archivo viajando de verdad entre los dos extremos.
//!
//! Esta es la prueba que demuestra que la transferencia de archivos funciona.
//! No hay dobles en ninguna capa de la que depende el resultado:
//!
//! - El servidor de senalizacion es el del producto, escuchando en un puerto.
//! - El host es el bucle del producto (`HostRuntime`): decide contra su
//!   registro, negocia solo y sirve solo.
//! - La conexion es WebRTC de verdad: ICE encuentra el camino, DTLS cifra y los
//!   marcos van por un canal SCTP, con sus limites de tamano de mensaje reales.
//! - Los bytes salen de un archivo de verdad en el disco y acaban en otro
//!   archivo de verdad, pasando por el formato binario del protocolo.
//!
//! Lo que se comprueba:
//!
//! 1. Que un archivo de varios trozos llega **byte a byte identico**, y que su
//!    resumen SHA-256 coincide con el que anuncio el host.
//! 2. Que una subida aterriza en la carpeta compartida, tambien identica.
//! 3. Que una ruta fuera de las carpetas compartidas se rechaza, aunque el
//!    dispositivo tenga permiso de archivos. Son dos concesiones distintas.
//! 4. Que un archivo de una carpeta de solo lectura no se puede sobrescribir.
//!
//! Lo unico que no es real sigue siendo el relay TURN, que no esta desplegado:
//! aqui los dos extremos estan en la misma maquina e ICE encuentra un camino
//! directo.

mod comun;

use std::path::{Path, PathBuf};
use std::time::Duration;

use comun::{Device, STEP_TIMEOUT, TestServer};
use nexus_core::protocol::Envelope;
use nexus_core::protocol::control::{
    ControlMessage, ControlRequest, ControlResponse, DirEntry, HostEvent, TransferState,
};
use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};
use nexus_core::{DeviceId, Permission, PermissionSet, SessionId, TransferId};
use nexus_file_transfer::{CHUNK_SIZE_BYTES, Frame, SharedRoot};
use nexus_security::DeviceIdentity;
use nexus_security::registry::DeviceRecord;
use nexus_transport::client::SignalingClient;
use nexus_transport::peer::{CONTROL_CHANNEL, FILES_CHANNEL, PeerEvent, PeerRole, PeerSession};
use sha2::{Digest, Sha256};

/// Tope total de la prueba. ICE, DTLS y mover un archivo tardan, pero no tanto.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Las carpetas que el host comparte en la prueba.
struct Carpetas {
    _temp: tempfile::TempDir,
    lectura: PathBuf,
    escritura: PathBuf,
    fuera: PathBuf,
    /// Contenido del archivo que se va a descargar.
    contenido: Vec<u8>,
}

impl Carpetas {
    fn preparar() -> Self {
        let temp = tempfile::tempdir().expect("carpeta temporal");
        let lectura = temp.path().join("documentos");
        let escritura = temp.path().join("intercambio");
        let fuera = temp.path().join("privado");
        for dir in [&lectura, &escritura, &fuera] {
            std::fs::create_dir_all(dir).expect("crea carpeta");
        }

        // Varios trozos y un resto, con un patron que delataria bytes mal
        // colocados o un trozo repetido.
        let contenido: Vec<u8> = (0..CHUNK_SIZE_BYTES * 3 + 1_777)
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(lectura.join("informe.bin"), &contenido).expect("crea archivo");
        std::fs::write(lectura.join("notas.txt"), b"dos lineas\ny otra").expect("crea archivo");
        std::fs::create_dir_all(lectura.join("subcarpeta")).expect("crea carpeta");
        std::fs::write(fuera.join("claves.txt"), b"esto no debe salir de aqui")
            .expect("crea archivo");

        Self {
            _temp: temp,
            lectura,
            escritura,
            fuera,
            contenido,
        }
    }

    fn roots(&self) -> Vec<SharedRoot> {
        vec![
            SharedRoot::read_only("Documentos", &self.lectura),
            SharedRoot::writable("Intercambio", &self.escritura),
        ]
    }
}

fn ruta(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Lo que la prueba recoge del otro extremo.
#[derive(Default)]
struct Recogido {
    listado: Option<(u64, Vec<DirEntry>)>,
    aceptadas: Vec<(TransferId, Option<u64>)>,
    rechazos: Vec<(String, String)>,
    /// Bytes de la descarga, en el orden en que llegaron.
    descargado: Vec<u8>,
    /// Resumen y numero de trozos que anuncio el host al cerrar.
    cierre: Option<(u32, [u8; 32])>,
    /// Transferencias que el host declaro terminadas, con su estado.
    finales: Vec<(TransferId, TransferState)>,
}

/// El guion que la prueba ejecuta, en orden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Paso {
    /// Esperando a que se abran los canales.
    Arrancando,
    /// Pidiendo el listado de la carpeta compartida.
    Listando,
    /// Pidiendo una ruta que no deberia poder verse.
    Fisgando,
    /// Descargando el archivo.
    Descargando,
    /// Subiendo una copia a la carpeta de escritura.
    Subiendo,
    /// Intentando sobrescribir en la carpeta de solo lectura.
    Forzando,
    /// Terminado.
    Hecho,
}

/// Un controlador que ejercita las operaciones de archivos.
struct Controlador {
    device: Device,
    sig: SignalingClient,
    ice: nexus_transport::IceConfiguration,
}

impl Controlador {
    async fn conectar(server: &TestServer, device: Device) -> Self {
        let (sig, aceptado) = device.connect_ok(server, None).await;
        Self {
            device,
            sig,
            ice: aceptado.ice,
        }
    }

    async fn pedir(&mut self, host: DeviceId, permisos: PermissionSet) -> ServerToClient {
        self.sig
            .send(ClientToServer::RequestSession {
                host_id: host,
                requested_permissions: permisos,
            })
            .await
            .expect("pide sesion");
        self.sig
            .recv_timeout(STEP_TIMEOUT)
            .await
            .expect("el host deberia contestar")
    }

    /// Lleva la sesion de principio a fin ejecutando el guion de archivos.
    async fn transferir(
        &mut self,
        session_id: SessionId,
        host: DeviceId,
        firmante: &DeviceIdentity,
        carpetas: &Carpetas,
    ) -> Recogido {
        let ice = self.ice.clone();
        let mut peer = PeerSession::open(&ice, PeerRole::Answerer)
            .await
            .expect("abre la conexion");

        let lado = comun::LadoControlador {
            session_id,
            yo: self.device.id(),
            host,
            firmante,
        };
        let mut recogido = Recogido::default();
        let mut paso = Paso::Arrancando;
        let mut abiertos: Vec<String> = Vec::new();

        loop {
            tokio::select! {
                mensaje = self.sig.recv() => {
                    let mensaje = mensaje.expect("el canal sigue abierto");
                    if !comun::atender_senalizacion(&mut self.sig, &peer, mensaje, &lado).await {
                        break;
                    }
                }
                evento = peer.next_event() => {
                    match evento.expect("la conexion sigue viva") {
                        PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                            comun::enviar_candidato(
                                &mut self.sig,
                                session_id,
                                candidate,
                                sdp_mid,
                            )
                            .await;
                        }
                        PeerEvent::ChannelOpen { label } => {
                            abiertos.push(label);
                            // Hacen falta los dos: el de control para las
                            // ordenes y el de archivos para los bytes.
                            if paso == Paso::Arrancando
                                && abiertos.iter().any(|l| l == CONTROL_CHANNEL)
                                && abiertos.iter().any(|l| l == FILES_CHANNEL)
                            {
                                paso = Paso::Listando;
                                pedir(
                                    &peer,
                                    ControlRequest::ListDir {
                                        path: ruta(&carpetas.lectura),
                                        offset: 0,
                                    },
                                )
                                .await;
                            }
                        }
                        PeerEvent::Message { label, data } if label == CONTROL_CHANNEL => {
                            paso = atender_control(
                                &peer,
                                &data,
                                paso,
                                &mut recogido,
                                carpetas,
                            )
                            .await;
                            if paso == Paso::Hecho {
                                break;
                            }
                        }
                        PeerEvent::Message { label, data } if label == FILES_CHANNEL => {
                            if let Some(siguiente) =
                                atender_archivo(&peer, &data, paso, &mut recogido, carpetas).await
                            {
                                paso = siguiente;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Deja al host terminar de escribir antes de cerrar.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let _ = peer.close().await;
        recogido
    }
}

/// Envia una peticion por el canal de control.
async fn pedir(peer: &PeerSession, request: ControlRequest) {
    let raw = Envelope::new(ControlMessage::Request(request))
        .encode()
        .expect("codifica");
    peer.send(CONTROL_CHANNEL, raw.as_bytes())
        .await
        .expect("envia la peticion");
}

/// Atiende lo que llega por el canal de control y avanza el guion.
async fn atender_control(
    peer: &PeerSession,
    data: &[u8],
    paso: Paso,
    recogido: &mut Recogido,
    carpetas: &Carpetas,
) -> Paso {
    let texto = std::str::from_utf8(data).expect("el canal de control es texto");
    let sobre: Envelope<ControlMessage> =
        Envelope::decode(texto).expect("la respuesta cumple el protocolo");

    match sobre.payload {
        ControlMessage::Response(ControlResponse::DirListing {
            entries,
            total_entries,
            ..
        }) => {
            recogido.listado = Some((total_entries, entries));
            // Siguiente: una ruta que no deberia poder verse.
            pedir(
                peer,
                ControlRequest::ListDir {
                    path: ruta(&carpetas.fuera),
                    offset: 0,
                },
            )
            .await;
            Paso::Fisgando
        }
        ControlMessage::Response(ControlResponse::Denied { code, message }) => {
            recogido.rechazos.push((code, message));
            match paso {
                Paso::Fisgando => {
                    pedir(
                        peer,
                        ControlRequest::DownloadStart {
                            path: ruta(&carpetas.lectura.join("informe.bin")),
                        },
                    )
                    .await;
                    Paso::Descargando
                }
                Paso::Forzando => Paso::Hecho,
                otro => otro,
            }
        }
        ControlMessage::Response(ControlResponse::TransferAccepted {
            transfer_id,
            size_bytes,
        }) => {
            recogido.aceptadas.push((transfer_id, size_bytes));
            if paso == Paso::Subiendo {
                // La subida ya esta aceptada: ahora van los bytes.
                enviar_archivo(peer, transfer_id, &carpetas.contenido).await;
            }
            paso
        }
        ControlMessage::Event(HostEvent::TransferProgress {
            transfer_id, state, ..
        }) => {
            if state.is_final() {
                recogido.finales.push((transfer_id, state));
                if paso == Paso::Subiendo && state == TransferState::Completed {
                    // Ultimo paso: intentar escribir donde no se puede.
                    pedir(
                        peer,
                        ControlRequest::UploadStart {
                            path: ruta(&carpetas.lectura.join("intruso.txt")),
                            size_bytes: 4,
                        },
                    )
                    .await;
                    return Paso::Forzando;
                }
            }
            paso
        }
        _ => paso,
    }
}

/// Atiende un marco del canal de archivos.
///
/// Devuelve el paso siguiente cuando la descarga termina.
async fn atender_archivo(
    peer: &PeerSession,
    data: &[u8],
    paso: Paso,
    recogido: &mut Recogido,
    carpetas: &Carpetas,
) -> Option<Paso> {
    let marco = Frame::parse(data).expect("el host envia marcos validos");
    match marco {
        Frame::Data { payload, .. } => {
            recogido.descargado.extend_from_slice(&payload);
            None
        }
        Frame::End { frames, digest, .. } => {
            recogido.cierre = Some((frames, digest));
            if paso == Paso::Descargando {
                // Lo descargado se vuelve a subir: asi la subida lleva un
                // contenido que de verdad viajo, no uno fabricado aqui.
                pedir(
                    peer,
                    ControlRequest::UploadStart {
                        path: ruta(&carpetas.escritura.join("copia.bin")),
                        size_bytes: recogido.descargado.len() as u64,
                    },
                )
                .await;
                return Some(Paso::Subiendo);
            }
            None
        }
        Frame::Abort { .. } => None,
    }
}

/// Envia un archivo entero por el canal de archivos.
async fn enviar_archivo(peer: &PeerSession, transfer_id: TransferId, contenido: &[u8]) {
    let mut sequence = 0_u32;
    for trozo in contenido.chunks(CHUNK_SIZE_BYTES) {
        let marco = Frame::Data {
            transfer_id,
            sequence,
            payload: trozo.to_vec(),
        };
        peer.send(FILES_CHANNEL, &marco.encode())
            .await
            .expect("envia el trozo");
        sequence += 1;
    }
    let digest: [u8; 32] = Sha256::digest(contenido).into();
    let fin = Frame::End {
        transfer_id,
        frames: sequence,
        digest,
    };
    peer.send(FILES_CHANNEL, &fin.encode())
        .await
        .expect("envia el cierre");
}

/// Permisos de archivos: leer y escribir, nada mas.
fn permisos_de_archivos() -> PermissionSet {
    PermissionSet::from_iter([
        Permission::ViewScreen,
        Permission::FileRead,
        Permission::FileWrite,
    ])
}

fn emparejado(device: &Device, permisos: PermissionSet) -> DeviceRecord {
    DeviceRecord::new(
        device.info.clone(),
        device.identity.public_key(),
        permisos,
        true,
        nexus_core::Timestamp::now(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn un_archivo_va_y_vuelve_por_el_canal_cifrado() {
    let prueba = async {
        let carpetas = Carpetas::preparar();
        let server = TestServer::start().await;

        let ipad = Device::controller("iPad de prueba");
        let host = comun::HostDeProducto::arrancar_con_carpetas(
            &server,
            vec![emparejado(&ipad, permisos_de_archivos())],
            carpetas.roots(),
        )
        .await;

        let mut control = Controlador::conectar(&server, ipad).await;
        let session_id = match control.pedir(host.id(), permisos_de_archivos()).await {
            ServerToClient::SessionAccepted { session_id, .. } => session_id,
            otro => panic!("el host deberia aceptar la sesion, llego {otro:?}"),
        };

        let firmante = control.device.duplicate_identity();
        let recogido = control
            .transferir(session_id, host.id(), &firmante, &carpetas)
            .await;

        // --- 1. El listado ---
        let (total, entradas) = recogido.listado.expect("deberia llegar el listado");
        assert_eq!(total, 3, "dos archivos y una subcarpeta");
        assert_eq!(
            entradas.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["subcarpeta", "informe.bin", "notas.txt"],
            "las carpetas van primero y luego los nombres ordenados"
        );
        let informe = entradas
            .iter()
            .find(|e| e.name == "informe.bin")
            .expect("esta en el listado");
        assert_eq!(informe.size_bytes, carpetas.contenido.len() as u64);
        assert!(!informe.is_dir);

        // --- 2. Lo que no se puede ver ---
        assert!(
            recogido
                .rechazos
                .iter()
                .any(|(code, _)| code == "access_denied"),
            "la carpeta de fuera deberia rechazarse: {:?}",
            recogido.rechazos
        );

        // --- 3. La descarga, byte a byte ---
        assert_eq!(
            recogido.descargado.len(),
            carpetas.contenido.len(),
            "deberia llegar el archivo completo"
        );
        assert_eq!(
            recogido.descargado, carpetas.contenido,
            "el archivo tiene que llegar identico"
        );

        let (trozos, digest) = recogido.cierre.expect("deberia llegar el marco de fin");
        let esperados = carpetas.contenido.len().div_ceil(CHUNK_SIZE_BYTES) as u32;
        assert_eq!(trozos, esperados, "el host declara los trozos que mando");
        let calculado: [u8; 32] = Sha256::digest(&recogido.descargado).into();
        assert_eq!(
            digest, calculado,
            "el resumen que anuncia el host tiene que cuadrar con lo recibido"
        );

        // --- 4. La subida aterriza en la carpeta compartida ---
        let copia = carpetas.escritura.join("copia.bin");
        assert!(copia.exists(), "la copia deberia estar en Intercambio");
        assert_eq!(
            std::fs::read(&copia).expect("lee la copia"),
            carpetas.contenido,
            "lo que subio tiene que estar identico en el disco del host"
        );
        assert!(
            recogido
                .finales
                .iter()
                .any(|(_, state)| *state == TransferState::Completed),
            "el host deberia declarar la subida completada: {:?}",
            recogido.finales
        );

        // --- 5. No se puede escribir en una carpeta de solo lectura ---
        assert!(
            recogido
                .rechazos
                .iter()
                .any(|(code, _)| code == "read_only"),
            "escribir en Documentos deberia rechazarse: {:?}",
            recogido.rechazos
        );
        assert!(
            !carpetas.lectura.join("intruso.txt").exists(),
            "no debe crearse nada en la carpeta de solo lectura"
        );

        // --- 6. Ni restos de temporales ---
        let restos: Vec<_> = std::fs::read_dir(&carpetas.escritura)
            .expect("lee")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("nexus-part"))
            .collect();
        assert!(
            restos.is_empty(),
            "no deberian quedar archivos temporales: {restos:?}"
        );

        host.parar().await;
    };

    tokio::time::timeout(TIMEOUT, prueba)
        .await
        .expect("la prueba no deberia agotar el tiempo");
}

/// El guion de la prueba hostil: que hace el controlador cuando quiere colar
/// algo que no deberia pasar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ataque {
    /// Enviar basura por el canal de archivos antes de nada.
    Basura,
    /// Subir un contenido que no coincide con el resumen anunciado.
    ResumenFalso,
}

/// Arranca el ataque en cuanto los dos canales estan abiertos.
async fn lanzar_ataque(peer: &PeerSession, ataque: Ataque, destino: &Path, size_bytes: usize) {
    if ataque == Ataque::Basura {
        // Basura por el canal binario, antes de abrir ninguna transferencia.
        for mugre in [
            vec![0_u8; 3],
            vec![0xFF_u8; 64],
            vec![1_u8, 9, 0, 0, 0, 0, 0, 0],
        ] {
            peer.send(FILES_CHANNEL, &mugre)
                .await
                .expect("envia basura");
        }
    }
    pedir(
        peer,
        ControlRequest::UploadStart {
            path: ruta(destino),
            size_bytes: size_bytes as u64,
        },
    )
    .await;
}

/// Al aceptarse la subida, envia **otro** contenido con el resumen del que se
/// anuncio. Devuelve el rechazo si lo que llego era uno.
async fn sustituir_contenido(
    peer: &PeerSession,
    data: &[u8],
    anunciado: &[u8],
) -> Option<(String, String)> {
    let texto = std::str::from_utf8(data).expect("texto");
    let sobre: Envelope<ControlMessage> = Envelope::decode(texto).expect("cumple el protocolo");

    match sobre.payload {
        ControlMessage::Response(ControlResponse::TransferAccepted { transfer_id, .. }) => {
            let falso = b"CONTENIDO SUSTITUIDO".to_vec();
            let marco = Frame::Data {
                transfer_id,
                sequence: 0,
                payload: falso[..anunciado.len()].to_vec(),
            };
            peer.send(FILES_CHANNEL, &marco.encode())
                .await
                .expect("envia el trozo");

            let digest: [u8; 32] = Sha256::digest(anunciado).into();
            let fin = Frame::End {
                transfer_id,
                frames: 1,
                digest,
            };
            peer.send(FILES_CHANNEL, &fin.encode())
                .await
                .expect("envia el cierre");
            None
        }
        ControlMessage::Response(ControlResponse::Denied { code, message }) => {
            Some((code, message))
        }
        _ => None,
    }
}

/// Un controlador que ejercita lo que el host tiene que rechazar.
struct Atacante {
    device: Device,
    sig: SignalingClient,
    ice: nexus_transport::IceConfiguration,
}

impl Atacante {
    async fn conectar(server: &TestServer, device: Device) -> Self {
        let (sig, aceptado) = device.connect_ok(server, None).await;
        Self {
            device,
            sig,
            ice: aceptado.ice,
        }
    }

    async fn pedir(&mut self, host: DeviceId, permisos: PermissionSet) -> ServerToClient {
        self.sig
            .send(ClientToServer::RequestSession {
                host_id: host,
                requested_permissions: permisos,
            })
            .await
            .expect("pide sesion");
        self.sig
            .recv_timeout(STEP_TIMEOUT)
            .await
            .expect("el host deberia contestar")
    }

    /// Lleva la sesion y ejecuta el ataque. Devuelve los rechazos recibidos.
    async fn atacar(
        &mut self,
        session_id: SessionId,
        host: DeviceId,
        firmante: &DeviceIdentity,
        carpetas: &Carpetas,
        ataque: Ataque,
    ) -> Vec<(String, String)> {
        let ice = self.ice.clone();
        let mut peer = PeerSession::open(&ice, PeerRole::Answerer)
            .await
            .expect("abre la conexion");

        let lado = comun::LadoControlador {
            session_id,
            yo: self.device.id(),
            host,
            firmante,
        };
        let mut rechazos = Vec::new();
        let mut abiertos: Vec<String> = Vec::new();
        let mut lanzado = false;
        let destino = carpetas.escritura.join("colado.bin");
        let anunciado = b"contenido autentico".to_vec();

        loop {
            tokio::select! {
                mensaje = self.sig.recv() => {
                    let mensaje = mensaje.expect("el canal sigue abierto");
                    if !comun::atender_senalizacion(&mut self.sig, &peer, mensaje, &lado).await {
                        break;
                    }
                }
                evento = peer.next_event() => {
                    match evento.expect("la conexion sigue viva") {
                        PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                            comun::enviar_candidato(
                                &mut self.sig,
                                session_id,
                                candidate,
                                sdp_mid,
                            )
                            .await;
                        }
                        PeerEvent::ChannelOpen { label } => {
                            abiertos.push(label);
                            if !lanzado
                                && abiertos.iter().any(|l| l == CONTROL_CHANNEL)
                                && abiertos.iter().any(|l| l == FILES_CHANNEL)
                            {
                                lanzado = true;
                                lanzar_ataque(&peer, ataque, &destino, anunciado.len()).await;
                            }
                        }
                        PeerEvent::Message { label, data } if label == CONTROL_CHANNEL => {
                            if let Some(rechazo) =
                                sustituir_contenido(&peer, &data, &anunciado).await
                            {
                                let cortar = rechazo.0 == "integrity_mismatch";
                                rechazos.push(rechazo);
                                if cortar {
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
        let _ = peer.close().await;
        rechazos
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn un_contenido_sustituido_no_llega_a_la_carpeta_compartida() {
    let prueba = async {
        let carpetas = Carpetas::preparar();
        let server = TestServer::start().await;

        let ipad = Device::controller("iPad tramposo");
        let host = comun::HostDeProducto::arrancar_con_carpetas(
            &server,
            vec![emparejado(&ipad, permisos_de_archivos())],
            carpetas.roots(),
        )
        .await;

        let mut atacante = Atacante::conectar(&server, ipad).await;
        let session_id = match atacante.pedir(host.id(), permisos_de_archivos()).await {
            ServerToClient::SessionAccepted { session_id, .. } => session_id,
            otro => panic!("el host deberia aceptar la sesion, llego {otro:?}"),
        };

        let firmante = atacante.device.duplicate_identity();
        let rechazos = atacante
            .atacar(
                session_id,
                host.id(),
                &firmante,
                &carpetas,
                Ataque::ResumenFalso,
            )
            .await;

        assert!(
            rechazos
                .iter()
                .any(|(code, _)| code == "integrity_mismatch"),
            "el host tenia que rechazar el contenido sustituido: {rechazos:?}"
        );
        assert!(
            !carpetas.escritura.join("colado.bin").exists(),
            "el archivo no debe existir: su contenido no coincidia con su resumen"
        );
        let restos: Vec<_> = std::fs::read_dir(&carpetas.escritura)
            .expect("lee")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            restos.is_empty(),
            "la carpeta compartida tiene que quedar como estaba: {restos:?}"
        );

        host.parar().await;
    };

    tokio::time::timeout(TIMEOUT, prueba)
        .await
        .expect("la prueba no deberia agotar el tiempo");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn la_basura_por_el_canal_de_archivos_no_tumba_la_sesion() {
    let prueba = async {
        let carpetas = Carpetas::preparar();
        let server = TestServer::start().await;

        let ipad = Device::controller("iPad ruidoso");
        let host = comun::HostDeProducto::arrancar_con_carpetas(
            &server,
            vec![emparejado(&ipad, permisos_de_archivos())],
            carpetas.roots(),
        )
        .await;

        let mut atacante = Atacante::conectar(&server, ipad).await;
        let session_id = match atacante.pedir(host.id(), permisos_de_archivos()).await {
            ServerToClient::SessionAccepted { session_id, .. } => session_id,
            otro => panic!("el host deberia aceptar la sesion, llego {otro:?}"),
        };

        let firmante = atacante.device.duplicate_identity();
        // Basura primero y despues una subida: lo que se comprueba es que la
        // sesion sobrevive a la basura y sigue atendiendo, no que la basura se
        // acepte.
        let rechazos = atacante
            .atacar(session_id, host.id(), &firmante, &carpetas, Ataque::Basura)
            .await;

        assert!(
            rechazos
                .iter()
                .any(|(code, _)| code == "bad_transfer" || code == "unknown_transfer"),
            "la basura deberia rechazarse con un motivo claro: {rechazos:?}"
        );
        assert!(
            rechazos
                .iter()
                .any(|(code, _)| code == "integrity_mismatch"),
            "y despues de la basura la sesion sigue viva y verificando: {rechazos:?}"
        );

        host.parar().await;
    };

    tokio::time::timeout(TIMEOUT, prueba)
        .await
        .expect("la prueba no deberia agotar el tiempo");
}
