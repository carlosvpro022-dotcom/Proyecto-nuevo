# Autenticación

**No hay un servicio de autenticación aparte, y es deliberado.**

La lista de dispositivos autorizados vive en el **host**, no en el servidor. El
servidor de señalización coordina el encuentro, pero quien decide si un
dispositivo puede abrir sesión es el PC al que se quiere acceder.

La razón es que el servidor es el componente del que menos queremos depender:
está en Internet, lo gestiona un proveedor y podría verse comprometido. Si la
lista de autorizados estuviera ahí, comprometer el servidor sería comprometer
el acceso a todos los PC.

Dónde está cada cosa:

| Pieza | Dónde |
| --- | --- |
| Identidad y firmas | `crates/security/src/identity.rs` |
| Autenticación mutua | `crates/security/src/challenge.rs` |
| Emparejamiento | `crates/security/src/pairing.rs` |
| Lista de autorizados y flujo completo | `crates/security/src/registry.rs` |
| Persistencia de la lista | `apps/windows-host/src-tauri/src/store.rs` |

El servidor de señalización reutilizará `nexus_security::Authenticator` para
comprobar que quien se conecta al WebSocket es un dispositivo conocido, pero la
autorización de la sesión concreta la concede el host.

Detalles en
[ADR-0004](../../docs/architecture/ADR-0004-identidad-criptografica.md).
