//! El canal de senalizacion, contra un servidor de verdad y por sockets reales.
//!
//! Lo que se comprueba aqui no es la maquina de estados (eso ya lo cubren las
//! pruebas del propio servidor) sino que **el cliente y el servidor se
//! entienden** atravesando un WebSocket real: la codificacion, el orden de los
//! mensajes, la firma Ed25519 y el enrutado entre dos conexiones distintas.

mod comun;

use comun::{Device, STEP_TIMEOUT, TOKEN, TestServer};
use nexus_core::protocol::signaling::{AuthRejection, ClientToServer, ServerToClient};
use nexus_core::{PermissionSet, SessionId};
use nexus_transport::TransportError;

#[tokio::test]
async fn un_host_se_autentica_y_queda_disponible() {
    let server = TestServer::start().await;
    let host = Device::host("PC del salon");

    let (mut client, accepted) = host.connect_ok(&server, Some(TOKEN)).await;

    assert!(
        accepted.ice.supports_remote_networks(),
        "el servidor debe entregar STUN o TURN: sin ellos la conexion solo \
         funcionaria dentro de la misma red"
    );
    assert!(accepted.ice.has_relay(), "y debe incluir relay");

    client
        .register_host(2)
        .await
        .expect("deberia quedar disponible");

    let hosts = server.state.shared().hub.available_hosts();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].device_id, host.id());
    assert_eq!(hosts[0].monitor_count, 2);
}

#[tokio::test]
async fn un_host_sin_testigo_de_alta_no_entra() {
    let server = TestServer::start().await;
    let host = Device::host("PC intruso");

    let error = host
        .connect(&server, None)
        .await
        .expect_err("sin testigo no deberia entrar");

    match error {
        TransportError::AuthRejected { reason, .. } => {
            assert_eq!(reason, AuthRejection::InvalidCredentials);
        }
        other => panic!("se esperaba un rechazo de autenticacion, llego {other:?}"),
    }
    assert!(server.state.shared().auth.registry().is_empty());
}

#[tokio::test]
async fn un_controlador_no_necesita_testigo() {
    let server = TestServer::start().await;
    let controller = Device::controller("iPad Pro");
    let (_client, _accepted) = controller.connect_ok(&server, None).await;
    assert!(server.state.shared().hub.is_connected(controller.id()));
}

#[tokio::test]
async fn nadie_puede_apropiarse_del_identificador_de_otro() {
    let server = TestServer::start().await;
    let legitimo = Device::controller("iPad Pro");
    let (_client, _) = legitimo.connect_ok(&server, None).await;

    // Mismo identificador, otra clave.
    let mut impostor = Device::controller("iPad Pro");
    impostor.info.id = legitimo.id();

    let error = impostor
        .connect(&server, None)
        .await
        .expect_err("con otra clave no deberia entrar");
    assert!(
        matches!(error, TransportError::AuthRejected { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn el_recorrido_de_sesion_llega_de_un_extremo_al_otro() {
    let server = TestServer::start().await;
    let host = Device::host("PC del salon");
    let controller = Device::controller("iPad Pro");

    let (mut host_client, _) = host.connect_ok(&server, Some(TOKEN)).await;
    host_client.register_host(1).await.expect("disponible");
    let (mut controller_client, _) = controller.connect_ok(&server, None).await;

    controller_client
        .send(ClientToServer::RequestSession {
            host_id: host.id(),
            requested_permissions: PermissionSet::full(),
        })
        .await
        .expect("envia la peticion");

    let session_id = match host_client
        .recv_timeout(STEP_TIMEOUT)
        .await
        .expect("el host recibe la peticion")
    {
        ServerToClient::SessionRequested {
            session_id,
            controller: quien,
            controller_capabilities,
            ..
        } => {
            assert_eq!(quien.id, controller.id());
            assert_eq!(
                controller_capabilities, controller.capabilities,
                "el host necesita las capacidades del controlador"
            );
            session_id
        }
        other => panic!("se esperaba SessionRequested, llego {other:?}"),
    };

    // El host concede menos de lo que se le pidio.
    host_client
        .send(ClientToServer::AcceptSession {
            session_id,
            negotiated_capabilities: host.capabilities.clone(),
            effective_permissions: PermissionSet::view_only(),
        })
        .await
        .expect("acepta");

    match controller_client
        .recv_timeout(STEP_TIMEOUT)
        .await
        .expect("el controlador recibe la aceptacion")
    {
        ServerToClient::SessionAccepted {
            effective_permissions,
            ..
        } => {
            assert_eq!(
                effective_permissions,
                PermissionSet::view_only(),
                "manda lo que concede el host"
            );
        }
        other => panic!("se esperaba SessionAccepted, llego {other:?}"),
    }
}

#[tokio::test]
async fn un_host_apagado_no_se_distingue_de_uno_inexistente() {
    let server = TestServer::start().await;
    let controller = Device::controller("iPad Pro");
    let (mut client, _) = controller.connect_ok(&server, None).await;

    client
        .send(ClientToServer::RequestSession {
            host_id: nexus_core::DeviceId::new(),
            requested_permissions: PermissionSet::standard(),
        })
        .await
        .expect("envia");

    match client.recv_timeout(STEP_TIMEOUT).await.expect("contesta") {
        ServerToClient::Error { code, .. } => assert_eq!(code, "host_unavailable"),
        other => panic!("se esperaba un error, llego {other:?}"),
    }
}

#[tokio::test]
async fn el_servidor_no_retransmite_codigos_de_emparejamiento() {
    // Es una limitacion conocida y declarada, no un fallo silencioso.
    let server = TestServer::start().await;
    let controller = Device::controller("iPad Pro");
    let (mut client, _) = controller.connect_ok(&server, None).await;

    client
        .send(ClientToServer::RedeemPairingCode {
            host_id: nexus_core::DeviceId::new(),
            pairing_id: nexus_core::PairingId::new(),
            code: "ABCD2345".to_owned(),
            public_key_hex: controller.identity.public_key().to_hex(),
        })
        .await
        .expect("envia");

    match client.recv_timeout(STEP_TIMEOUT).await.expect("contesta") {
        ServerToClient::Error { code, .. } => assert_eq!(code, "pairing_relay_unavailable"),
        other => panic!("se esperaba un rechazo explicito, llego {other:?}"),
    }
}

#[tokio::test]
async fn una_sesion_ajena_no_se_puede_tocar() {
    let server = TestServer::start().await;
    let host = Device::host("PC del salon");
    let controller = Device::controller("iPad Pro");
    let intruso = Device::controller("iPhone ajeno");

    let (mut host_client, _) = host.connect_ok(&server, Some(TOKEN)).await;
    host_client.register_host(1).await.expect("disponible");
    let (mut controller_client, _) = controller.connect_ok(&server, None).await;
    let (mut intruso_client, _) = intruso.connect_ok(&server, None).await;

    controller_client
        .send(ClientToServer::RequestSession {
            host_id: host.id(),
            requested_permissions: PermissionSet::standard(),
        })
        .await
        .expect("envia");
    let session_id = match host_client
        .recv_timeout(STEP_TIMEOUT)
        .await
        .expect("recibe")
    {
        ServerToClient::SessionRequested { session_id, .. } => session_id,
        other => panic!("se esperaba SessionRequested, llego {other:?}"),
    };

    intruso_client
        .send(ClientToServer::Offer {
            session_id,
            sdp: "v=0 intruso".to_owned(),
        })
        .await
        .expect("envia");

    match intruso_client
        .recv_timeout(STEP_TIMEOUT)
        .await
        .expect("contesta")
    {
        ServerToClient::Error { code, .. } => assert_eq!(code, "not_a_member"),
        other => panic!("se esperaba not_a_member, llego {other:?}"),
    }
}

#[tokio::test]
async fn una_sesion_inventada_se_rechaza() {
    let server = TestServer::start().await;
    let controller = Device::controller("iPad Pro");
    let (mut client, _) = controller.connect_ok(&server, None).await;

    client
        .send(ClientToServer::Offer {
            session_id: SessionId::new(),
            sdp: "v=0".to_owned(),
        })
        .await
        .expect("envia");

    match client.recv_timeout(STEP_TIMEOUT).await.expect("contesta") {
        ServerToClient::Error { code, .. } => assert_eq!(code, "unknown_session"),
        other => panic!("se esperaba unknown_session, llego {other:?}"),
    }
}

#[tokio::test]
async fn reconectar_desplaza_la_conexion_anterior() {
    // Es lo que pasa cuando el movil cambia de red: la conexion anterior aun no
    // se ha dado por muerta y el dispositivo no puede quedarse fuera.
    let server = TestServer::start().await;
    let controller = Device::controller("iPad Pro");

    let (mut primera, _) = controller.connect_ok(&server, None).await;
    let (_segunda, _) = controller.connect_ok(&server, None).await;

    match primera.recv_timeout(STEP_TIMEOUT).await {
        Ok(ServerToClient::Error { code, .. }) => assert_eq!(code, "connection_replaced"),
        other => panic!("la conexion anterior deberia recibir el aviso, llego {other:?}"),
    }
    assert_eq!(server.state.shared().hub.peer_count(), 1);
}

#[tokio::test]
async fn al_reconectar_la_conexion_nueva_sobrevive_a_la_antigua() {
    // Lo que pasa cuando el movil cambia de red: reconecta antes de que la
    // conexion anterior se de por muerta. Cuando esa muere, no debe llevarse
    // por delante a la nueva.
    let server = TestServer::start().await;
    let controller = Device::controller("iPad Pro");

    let (primera, _) = controller.connect_ok(&server, None).await;
    let (mut segunda, _) = controller.connect_ok(&server, None).await;

    // La primera se cierra del todo.
    primera.close().await.expect("cierra");

    // Dale tiempo al servidor a limpiarla.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert!(
        server.state.shared().hub.is_connected(controller.id()),
        "la conexion nueva debe seguir registrada en el servidor"
    );
    assert_eq!(server.state.shared().hub.peer_count(), 1);

    // Y sigue sirviendo: el servidor le contesta.
    segunda.send(ClientToServer::Ping).await.expect("envia");
    assert!(matches!(
        segunda.recv_timeout(STEP_TIMEOUT).await.expect("contesta"),
        ServerToClient::Pong
    ));
}

#[tokio::test]
async fn el_servidor_contesta_al_latido() {
    let server = TestServer::start().await;
    let controller = Device::controller("iPad Pro");
    let (mut client, _) = controller.connect_ok(&server, None).await;

    client.send(ClientToServer::Ping).await.expect("envia");
    assert!(matches!(
        client.recv_timeout(STEP_TIMEOUT).await.expect("contesta"),
        ServerToClient::Pong
    ));
}

#[tokio::test]
async fn el_registro_del_servidor_anota_la_conexion() {
    let server = TestServer::start().await;
    let host = Device::host("PC del salon");
    let (mut client, _) = host.connect_ok(&server, Some(TOKEN)).await;
    client.register_host(1).await.expect("disponible");

    let lines = server.audit_lines();
    assert!(
        lines.iter().any(|l| l.contains("127.0.0.1")),
        "debe constar la direccion de origen: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("alta aceptada")),
        "{lines:?}"
    );
    assert!(
        lines.iter().all(|l| !l.contains(TOKEN)),
        "el testigo no debe aparecer nunca: {lines:?}"
    );
}

#[tokio::test]
async fn una_url_sin_cifrar_contra_otra_maquina_se_rechaza_antes_de_conectar() {
    let host = Device::host("PC del salon");
    let error = nexus_transport::SignalingClient::connect(
        nexus_transport::ConnectRequest {
            url: "ws://nexus.example/v1/signaling",
            device: &host.info,
            capabilities: &host.capabilities,
            enrollment_token: Some(TOKEN),
        },
        &host.identity,
    )
    .await
    .expect_err("no deberia intentar conectar en claro");

    assert!(matches!(error, TransportError::InsecureUrl(_)), "{error:?}");
}

#[tokio::test]
async fn la_comprobacion_del_host_informa_de_lo_que_encuentra() {
    // Es lo que el usuario pulsara en el panel de su PC para saber si su
    // servidor funciona, antes de tener un iPad al otro lado.
    let server = TestServer::start().await;
    let host = Device::host("PC del salon");

    let probe = nexus_windows_host::signaling::probe(
        &server.url,
        &host.info,
        &host.capabilities,
        &host.identity,
        Some(TOKEN),
        2,
    )
    .await;

    assert!(probe.connected, "deberia conectar: {}", probe.detail);
    assert_eq!(probe.ice_servers, 2, "STUN y TURN");
    assert!(probe.has_relay, "{}", probe.detail);
    assert!(probe.supports_remote_networks);

    // Y queda disponible de verdad en el servidor, no solo autenticado.
    assert_eq!(server.state.shared().hub.available_hosts().len(), 1);
}

#[tokio::test]
async fn la_comprobacion_explica_que_falta_el_testigo_de_alta() {
    let server = TestServer::start().await;
    let host = Device::host("PC del salon");

    let probe = nexus_windows_host::signaling::probe(
        &server.url,
        &host.info,
        &host.capabilities,
        &host.identity,
        None,
        1,
    )
    .await;

    assert!(!probe.connected);
    assert!(
        probe.detail.contains("testigo de alta"),
        "sin esta pista el usuario no puede adivinar la causa: {}",
        probe.detail
    );
}

#[tokio::test]
async fn la_comprobacion_explica_un_servidor_que_no_responde() {
    let host = Device::host("PC del salon");

    // Puerto cerrado en la propia maquina: falla al instante y sin DNS.
    let probe = nexus_windows_host::signaling::probe(
        "ws://127.0.0.1:1/v1/signaling",
        &host.info,
        &host.capabilities,
        &host.identity,
        Some(TOKEN),
        1,
    )
    .await;

    assert!(!probe.connected);
    assert!(
        probe.detail.contains("No se pudo hablar con el servidor"),
        "{}",
        probe.detail
    );
}

#[tokio::test]
async fn la_comprobacion_avisa_cuando_no_hay_relay() {
    // Un servidor sin TURN parece funcionar y luego falla desde una red movil.
    // El aviso tiene que salir en la comprobacion, no en el primer intento real.
    let server = TestServer::start().await;
    let host = Device::host("PC del salon");

    let probe = nexus_windows_host::signaling::probe(
        &server.url,
        &host.info,
        &host.capabilities,
        &host.identity,
        Some(TOKEN),
        1,
    )
    .await;

    // Este servidor si tiene relay configurado; lo que se comprueba es que el
    // dato llega al panel para poder avisar cuando no lo haya.
    assert!(probe.has_relay);
    assert!(probe.detail.contains("relay"), "{}", probe.detail);
}

#[tokio::test]
async fn un_marco_enorme_se_corta_sin_llegar_a_almacenarse() {
    // El tope de 64 KiB tiene que estar en la capa WebSocket, no solo al
    // decodificar. Por omision axum admite mensajes de 64 MB: sin este tope, un
    // cliente sin autenticar podria hacer que el servidor reservase esa memoria
    // por conexion antes de que nadie mirase el contenido.
    //
    // Por eso esta prueba usa un cliente WebSocket crudo y no el del proyecto:
    // el nuestro se niega a codificar un mensaje asi, que es justo lo que hay
    // que saltarse para comprobar al servidor.
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let server = TestServer::start().await;
    let (mut socket, _) = tokio_tungstenite::connect_async(&server.url)
        .await
        .expect("el canal acepta la conexion");

    let enorme = "x".repeat(1024 * 1024);
    // El envio puede fallar ya aqui, si el servidor corta mientras se escribe.
    let enviado = socket.send(Message::Text(enorme.into())).await;

    // Lo que no puede pasar es que el servidor conteste un error de protocolo,
    // porque eso significaria que almaceno el megabyte entero para mirarlo.
    let respuesta = tokio::time::timeout(STEP_TIMEOUT, socket.next()).await;
    let contesto_error = matches!(
        &respuesta,
        Ok(Some(Ok(Message::Text(texto)))) if texto.contains("bad_request")
    );

    assert!(
        !contesto_error,
        "el servidor no debe procesar un marco de 1 MiB: tiene que cortarlo antes"
    );
    let se_corto = enviado.is_err()
        || matches!(
            respuesta,
            Err(_) | Ok(None | Some(Err(_) | Ok(Message::Close(_))))
        );
    assert!(se_corto, "la conexion deberia cerrarse: {respuesta:?}");

    // Y el servidor sigue atendiendo a los demas.
    let controller = Device::controller("iPad Pro");
    let (_cliente, _) = controller.connect_ok(&server, None).await;
}

#[tokio::test]
async fn un_marco_binario_cierra_la_conexion() {
    // El protocolo es texto. Un marco binario no es un mensaje que no se
    // entienda: es un cliente que no habla este protocolo.
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let server = TestServer::start().await;
    let (mut socket, _) = tokio_tungstenite::connect_async(&server.url)
        .await
        .expect("el canal acepta la conexion");

    socket
        .send(Message::Binary(vec![0, 1, 2].into()))
        .await
        .expect("envia");

    match tokio::time::timeout(STEP_TIMEOUT, socket.next()).await {
        Ok(None | Some(Err(_) | Ok(Message::Close(_)))) => {}
        otro => panic!("se esperaba un cierre, llego {otro:?}"),
    }
}
