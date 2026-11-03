# Cliente de iPhone y iPad

**Pendiente — etapa 4.**

Aquí irá el proyecto Xcode del cliente nativo: Swift y SwiftUI, con
`WebRTC.framework` para el transporte y `VideoToolbox` para decodificar el
vídeo por hardware.

La decisión y sus alternativas están en
[ADR-0005](../../docs/architecture/ADR-0005-cliente-apple-nativo.md).

## Por qué está vacío

**Hace falta un Mac con Xcode.** No se puede crear, compilar ni probar un
proyecto de iOS desde Linux ni desde Windows. Crear aquí una carpeta de
archivos Swift que nunca han pasado por un compilador daría la impresión de
que hay avance donde no lo hay.

## Qué hará falta cuando llegue el momento

| Requisito | Detalle |
| --- | --- |
| Mac con Xcode 16 o superior | Para compilar y firmar |
| Cuenta de desarrollador de Apple | Gratuita vale, pero la firma caduca cada 7 días |
| Los dispositivos físicos | El simulador no da acceso a la cámara para el QR |

## Qué está ya resuelto y le espera

- El protocolo completo, en `crates/core`, con sus mensajes y sus reglas.
- El flujo de emparejamiento del lado del host, funcionando y probado.
- La invitación que viaja en el QR, con su formato JSON ya definido.
- La forma exacta de la transcripción que el cliente tendrá que firmar, en
  `crates/security/src/challenge.rs`, campo por campo y con su razón.

Antes de empezar la etapa 4 hay que decidir cómo compartir los contratos entre
Rust y Swift. Las dos opciones y la preferencia inicial están en el ADR-0005.
