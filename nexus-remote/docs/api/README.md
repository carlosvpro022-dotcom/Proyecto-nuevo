# Referencia del protocolo

La referencia la genera el propio código, que es donde está siempre
actualizada:

```bash
cargo doc --workspace --no-deps --open
```

Los dos protocolos están documentados módulo a módulo:

| Protocolo | Dónde | Qué transporta |
| --- | --- | --- |
| **Señalización** | `nexus_core::protocol::signaling` | Encuentro, autenticación y negociación WebRTC, entre dispositivo y servidor propio. Lo ve el servidor. |
| **Control** | `nexus_core::protocol::control` | Ratón, teclado, portapapeles, archivos y administración, entre los dos dispositivos. Cifrado extremo a extremo; el servidor **no** lo ve. |

Ambos viajan dentro de un `nexus_core::protocol::Envelope`, que añade la
versión de protocolo y la marca temporal que necesita la guardia antirreenvío.

Para entender **por qué** el protocolo es así, y no solo cómo es, los ADR son
mejor punto de partida que la referencia:
[ADR-0003](../architecture/ADR-0003-transporte-webrtc.md) y
[ADR-0004](../architecture/ADR-0004-identidad-criptografica.md).
