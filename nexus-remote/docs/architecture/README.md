# Decisiones de arquitectura

Cada archivo `ADR-NNNN-*.md` recoge una decisión técnica: qué se decidió, por
qué, qué alternativas se descartaron y qué consecuencias tiene.

El formato es deliberadamente corto. Un ADR no documenta cómo funciona el
código —para eso está el código y sus comentarios—, sino **por qué es así y no
de otra manera**, que es lo que se pierde con el tiempo.

| ADR | Decisión | Estado |
| --- | --- | --- |
| [0001](ADR-0001-monorepositorio.md) | Monorepositorio con workspace de Cargo | Aceptada |
| [0002](ADR-0002-rust-y-tauri.md) | Rust y Tauri para el host de Windows | Aceptada |
| [0003](ADR-0003-transporte-webrtc.md) | WebRTC para medios y control | Aceptada |
| [0004](ADR-0004-identidad-criptografica.md) | Ed25519 por dispositivo y autenticación mutua | Aceptada |
| [0005](ADR-0005-cliente-apple-nativo.md) | Cliente Apple nativo en SwiftUI | Aceptada |
| [0006](ADR-0006-infraestructura-propia.md) | Infraestructura propia con Coturn | Aceptada |
| [0007](ADR-0007-verificacion-multiplataforma.md) | Cómo se verifica código que no se puede ejecutar aquí | Aceptada |
| [0008](ADR-0008-sin-datos-simulados.md) | Nada de datos simulados en la interfaz | Aceptada |
| [0009](ADR-0009-atadura-de-sesion.md) | La sesión se ata a la huella DTLS del otro extremo | Aceptada |
| [0010](ADR-0010-emparejamiento-por-red.md) | El servidor no retransmite códigos de emparejamiento | Aceptada |
| [0011](ADR-0011-directorio-de-identidades.md) | El servidor conoce identidades, no permisos | Aceptada |
| [0012](ADR-0012-transferencia-de-archivos.md) | Los archivos van por su propio canal binario y el destino se publica verificado | Aceptada |
| [0013](ADR-0013-confirmacion-en-el-host.md) | El token de confirmación nace sin validez: solo vale si una persona lo aprueba en el PC | Aceptada |
| [0014](ADR-0014-bombeo-de-pantalla.md) | La pantalla se pide, se bombea en su propio hilo y nunca se tira un fotograma | Aceptada |
