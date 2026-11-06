# NEXUS REMOTE

Alternativa privada a AnyDesk: controlar un PC con Windows desde un iPhone o un
iPad, desde cualquier parte del mundo, con infraestructura propia y acceso
exclusivo a los dispositivos previamente autorizados.

---

## Estado real del proyecto

**Etapas 1 y 3 terminadas. Etapa 2 escrita y compilada, pendiente de probar en
Windows.** Lo que sigue es exacto, y distingue tres cosas: lo que se ha
ejecutado y probado, lo que está escrito pero **nadie ha ejecutado en su
entorno real**, y lo que todavía no existe.

### Funciona y está probado

| Pieza | Qué hace |
| --- | --- |
| **Contratos del protocolo** | Mensajes de señalización y de control, permisos por operación, capacidades negociables, perfiles de calidad, monitores, máquina de estados de sesión. Cada petición del canal de control lleva un identificador que el host devuelve en su respuesta: emparejar por orden de llegada no sirve, porque hay peticiones sin respuesta y eventos que el host empuja por el mismo canal. |
| **Identidad criptográfica** | Ed25519 por dispositivo, persistente entre arranques. Verificación estricta de firmas. |
| **Autenticación mutua** | Reto de un solo uso firmado por ambas partes, con defensa contra reflexión y contra reenvío. |
| **Emparejamiento** | Código temporal de 8 caracteres y QR. Derivado con Argon2id, 5 minutos, 5 intentos. |
| **Lista de autorizados** | SQLite con permisos por dispositivo, revocación, borrado e historial de sesiones. |
| **Límite de intentos** | Bloqueo progresivo por dispositivo. Esperar a que expire no reinicia la escalada. |
| **Restricción de acceso al disco** | Solo las carpetas declaradas. Rechaza recorrido de directorios, enlaces que escapan y nombres reservados de Windows. |
| **Clave del host cifrada en disco** | Una clave guardada antes de que existiera el cifrado se migra al arrancar sin cambiar la identidad del equipo, y si el cifrado falla el host sigue funcionando con la que ya tenía. Eso está probado; la llamada a DPAPI que cifra en Windows está abajo. |
| **Confirmación en el PC para apagar** | Reiniciar y apagar no se hacen porque el dispositivo lo pida: el token que recibe quien lo pide **no vale** hasta que alguien lo apruebe en esta ventana. Un solo uso, caduca en dos minutos y muere con la sesión ([ADR-0013](docs/architecture/ADR-0013-confirmacion-en-el-host.md)). La decisión está probada; la llamada que apaga de verdad, no (abajo). |
| **Límites del portapapeles** | Un texto que no quepa en el canal se rechaza en lugar de recortarse: medio comando o media contraseña con aspecto de estar completos es peor que un error. Leer y escribir el portapapeles de Windows está abajo. |
| **Transferencia de archivos** | Listar, descargar y subir, por un canal binario propio. Un archivo subido se escribe aparte y solo aparece en su ruta cuando su contenido coincide con su `SHA-256`: si la sesión se corta, no queda nada a medias. Comprobado moviendo un archivo de varios trozos por un canal WebRTC real y verificando que llega idéntico ([ADR-0012](docs/architecture/ADR-0012-transferencia-de-archivos.md)). |
| **Servidor de señalización** | `/v1/signaling` completo: autenticación Ed25519 del dispositivo, directorio de identidades con testigo de alta, límite de intentos por dispositivo y por origen, enrutado entre los dos extremos, registro de conexiones. |
| **Transmitir la pantalla en una sesión** | La cadena entera en una sesión del producto: el host negocia WebRTC, comprueba quién hay al otro lado, atiende la petición de pantalla, escala, codifica en H.264 de perfil base —el que un iPhone decodifica por hardware—, trocea en RTP y lo cifra; al otro lado se recompone y se decodifica una imagen. Cambiar la calidad en caliente cambia de verdad lo que llega. Sin firmar la atadura de sesión **no sale un solo paquete** ([ADR-0014](docs/architecture/ADR-0014-bombeo-de-pantalla.md)). **Lo único que no es real en la prueba son los píxeles**: la captura solo corre en Windows, así que vienen de un capturador sintético. |
| **Escalado para el ancho de banda** | Una pantalla 4K no cabe en una conexión móvil. Al reducir se promedian los píxeles en lugar de descartarlos: lo que se transmite es texto, y descartar píxeles borra los trazos finos de las letras. |
| **El vídeo no acapara el PC** | El codificador usa la mitad de los núcleos, nunca más de cuatro: sin decírselo, openh264 coge todos. Y un fotograma codificado nunca se tira, porque en H.264 perder uno estropea los siguientes: si la red se atrasa, la captura espera. |
| **Conexión cifrada entre dispositivos** | WebRTC de verdad: ICE encuentra camino, DTLS cifra y los canales `control` e `input` transportan órdenes. Comprobado de extremo a extremo: un toque enviado por un par llega al otro y se aplica. |
| **El host atiende sesiones** | Se conecta solo, decide cada petición contra su registro local, comprueba quién hay al otro lado y sirve el canal de control. Reintenta con espera creciente si el enlace se cae. **No aplica ni una orden antes de autenticar al otro extremo.** |
| **Historial de sesiones** | Cada sesión queda registrada con su dispositivo, su duración y por qué terminó. |
| **Autenticación mutua atada al canal** | Cada extremo firma las dos huellas DTLS de la sesión. Si el servidor sustituye una para interceptar, la sesión se aborta ([ADR-0009](docs/architecture/ADR-0009-atadura-de-sesion.md)). |
| **Credenciales TURN temporales** | Derivación HMAC-SHA1 compatible con Coturn, verificada contra un vector de prueba público. |
| **Panel del host** | Aplicación Tauri con 8 pantallas. Muestra datos reales del equipo, y comprueba contra el servidor de verdad. |

### Escrito, pero sin ejecutar en su entorno real

Son dos cosas distintas, y conviene no confundirlas.

> Para convertir esto en verificado, en el PC con Windows:
>
> ```powershell
> cargo run -p nexus-windows-host --example diagnostico
> ```
>
> Escribe un informe de lo que este equipo puede hacer de verdad, sin tocar
> nada y sin necesitar que la interfaz llegue a abrirse. La lista completa
> está en
> [`docs/deployment/probar-en-windows.md`](docs/deployment/probar-en-windows.md).

**El motor de Windows (etapa 2).** El compilador verificó tipos, nombres de API
y firmas mediante compilación en cruce a `x86_64-pc-windows-gnu`, con clippy
pedantic y los avisos como errores. **Nadie lo ha ejecutado en un PC con
Windows**, así que el comportamiento está sin comprobar.

| Pieza | Cómo está implementada |
| --- | --- |
| **Detección de monitores** | `EnumDisplayMonitors`, `GetMonitorInfoW`, `GetDpiForMonitor` y `EnumDisplaySettingsW`. Devuelve resolución, posición, escala, refresco y cuál es la principal. |
| **Captura de pantalla** | Windows Graphics Capture sobre Direct3D 11: cola de fotogramas sin hilo propio, copia a textura de preparación y lectura BGRA. Rehace la cola al cambiar la resolución y recupera el fotograma a la cola pase lo que pase. |
| **Control de ratón y teclado** | `SendInput` con coordenadas absolutas sobre el escritorio virtual, tabla de ~110 teclas con su marca de extendida, texto por `KEYEVENTF_UNICODE` (pares subrogados incluidos) y liberación de todo al cerrar. |
| **Portapapeles de Windows** | `OpenClipboard`/`GetClipboardData`/`SetClipboardData` con `CF_UNICODETEXT`. Resueltas las tres trampas de esa API: cerrarlo siempre (desde un `Drop`), que `GlobalUnlock` devuelva falso cuando acierta, y que quien escribe deje de ser el dueño de la memoria. |
| **Cifrado de la clave del host** | `CryptProtectData`/`CryptUnprotectData` con ámbito de usuario y `CRYPTPROTECT_UI_FORBIDDEN`, porque el host puede arrancar sin nadie delante. El bloque que devuelve DPAPI se libera desde un `Drop`. |
| **Bloquear, reiniciar y apagar** | `LockWorkStation` y `ExitWindowsEx`, activando antes `SeShutdownPrivilege`. Se comprueba `GetLastError` porque `AdjustTokenPrivileges` devuelve éxito aunque no haya podido activarlo. No se fuerza el cierre de programas con trabajo sin guardar. |

Lo que **sí** está probado de esa etapa es la parte portable, que es donde
están los errores difíciles: la tabla de teclas, la aritmética de coordenadas
(verificada píxel a píxel sobre siete configuraciones de pantalla) y la
conversión de los datos crudos del sistema.

**La conexión entre redes distintas (etapa 3).** El código de red sí se ejecuta
aquí, y la conexión WebRTC se establece de verdad. Pero en bucle local los dos
extremos están en la misma máquina, ICE encuentra camino directo siempre y el
relay TURN nunca se usa. Lo que falta por comprobar no es el código, es la
**topología**: dos redes, dos NAT, un Coturn en marcha y un certificado válido.
Hace falta desplegar el servidor.

### Todavía no existe

| Pieza | Etapa | Qué falta |
| --- | --- | --- |
| Emparejar desde fuera de la red local | 3 | El servidor lo rechaza a propósito: el código tiene unos 40 bits y viajaría en claro por él. Falta enviar un verificador Argon2id y confirmar la huella en el host ([ADR-0010](docs/architecture/ADR-0010-emparejamiento-por-red.md)). Mientras, se empareja en la red local. |
| Cliente de iPhone y iPad | 4 | El proyecto entero. **Hace falta un Mac con Xcode.** |
| Ver la pantalla del PC de verdad | 5 | **Falta ejecutarlo en Windows y tener quien la mire.** La cadena completa ya funciona dentro de una sesión del producto, con una salvedad: los píxeles vienen de un capturador sintético, porque la captura real solo corre en Windows. Lo que falta es arrancar el host en un PC —el `diagnostico` lo comprueba en un comando— y el cliente de iPhone que muestre la imagen. |
| Audio del sistema | 5 | La captura WASAPI en modo bucle. El formato y el control de volumen están listos. |

El panel refleja esta misma distinción con tres estados —*Probado*, *Sin
probar*, *Pendiente*—, y cuatro pruebas lo sostienen: impiden marcar como
probado código de Windows que nadie ha ejecutado en Windows, declarar probada
la conexión entre redes distintas sin haberla desplegado, anunciar como
disponible algo que el servidor rechaza, y anunciar capacidades que el host no
tiene implementadas.

Además del estado del proyecto, el panel comprueba **este equipo concreto**:
intenta enumerar las pantallas, crear el inyector y —si se le pide— conectarse
de verdad al servidor configurado, y enseña lo que ocurrió.

**No hay datos de ejemplo en ninguna parte**
([ADR-0008](docs/architecture/ADR-0008-sin-datos-simulados.md)).

### Verificación

```
712 pruebas automatizadas, todas en verde
  · nexus-security        131    criptografía, emparejamiento, antirreenvío, atadura de sesión
  · nexus-signaling       129    protocolo del canal, directorio de identidades, enrutado, registro
  · nexus-windows-host    115    almacén, admisión de sesiones, bucle del host, comandos
  · nexus-core            108    protocolo, permisos, estados, limpieza de texto
  · nexus-input            44    tabla de teclas, coordenadas absolutas
  · nexus-transport        44    códec, ICE, cliente, traducción de configuración
  · nexus-capture          41    fotogramas, ritmo, enumeración de monitores
  · nexus-file-transfer    37    restricción de acceso, transferencias
  · integración            49    recorridos completos, con el host del producto atendiendo
  · nexus-audio            14    formato, volumen

cargo fmt            sin diferencias
clippy::pedantic     sin un solo aviso, con -D warnings, en Linux y en Windows
panel                8 pantallas verificadas en Chromium con datos reales del backend
cruce a Windows      clippy --target x86_64-pc-windows-gnu limpio en los 7 crates
sesión completa      servidor real en un puerto local, ICE, DTLS, canal de datos y
                     un evento de entrada que llega al final de la cadena
host atendiendo      el bucle del producto decide, autentica y sirve a un
                     controlador real; y no toca el equipo si la firma no cuadra
```

La última línea es la prueba que sostiene la etapa 3. No hay dobles de prueba
en ninguna capa: el servidor es el de verdad, montado con la misma función que
usa el binario; las claves son Ed25519 reales; la conexión es WebRTC real; y el
manejador que recibe la orden al final es el del producto, no una copia escrita
para la prueba.

---

## Estructura

```
nexus-remote/
├── crates/                 código compartido, sin plataforma, sin E/S
│   ├── core/               protocolo, permisos, estados      ← el contrato
│   ├── security/           identidad, emparejamiento, defensas
│   ├── transport/          códec, ICE, enlace de señalización
│   ├── capture/            contrato de captura + control de ritmo
│   ├── input/              resolución de eventos + contrato de inyección
│   ├── audio/              formato y volumen + contrato de captura
│   └── file-transfer/      restricción de acceso + transferencias
├── apps/
│   ├── windows-host/       aplicación Tauri (Rust + React)
│   └── ios-client/         pendiente, etapa 4
├── server/
│   ├── signaling/          servidor propio (axum)
│   ├── relay/              pendiente: solo se usa Coturn
│   └── auth/               pendiente: la autenticación vive en el host
├── infrastructure/
│   ├── docker/             Compose, Dockerfile, proxy inverso
│   ├── turn/               configuración de Coturn
│   └── deployment/         pendiente
├── docs/
│   ├── architecture/       decisiones y por qué (8 ADR)
│   └── deployment/         guía de despliegue
├── scripts/                preparar entorno, verificar, servidor local
└── tests/                  pruebas que cruzan varios crates
```

---

## Empezar

### Requisitos

- **Rust** 1.90 o superior ([rustup.rs](https://rustup.rs))
- **Node** 20 o superior
- En Linux, las dependencias de Tauri. El script las comprueba e instala.

```bash
cd nexus-remote
./scripts/preparar-entorno.sh            # diagnostica
./scripts/preparar-entorno.sh --instalar # instala lo que falte
```

### Compilar y probar

```bash
cargo test --workspace                   # todas las pruebas de Rust
./scripts/verificar.sh                   # lo mismo que la CI
./scripts/verificar.sh --rapido          # sin panel ni cruce
```

### El panel del host

```bash
cd apps/windows-host
npm install
npm run tauri dev
```

Se abre la ventana, genera la identidad del host en el primer arranque y
muestra los datos reales del equipo.

> En Linux funciona para desarrollo, pero la captura de pantalla y el control
> de entrada devolverán `UnsupportedPlatform`: son específicos de Windows.

### El servidor de señalización en local

```bash
./scripts/servidor-local.sh
curl http://127.0.0.1:8443/v1/info
```

### Desplegar el servidor de verdad

Sigue [`docs/deployment/despliegue.md`](docs/deployment/despliegue.md). En
resumen: una máquina con IP pública, un dominio, y

```bash
cd infrastructure/docker
cp .env.example .env     # rellénelo; el secreto con: openssl rand -base64 48
docker compose up -d
```

---

## Cómo está pensado

Tres ideas gobiernan el diseño. Cada una tiene su ADR.

**1 · El contrato vive en un sitio.** `crates/core` lo compilan a la vez el
host y el servidor. Un cambio incompatible rompe la compilación en el mismo
commit, no en producción
([ADR-0001](docs/architecture/ADR-0001-monorepositorio.md)).

**2 · La validación vive en código portable.** Toda la comprobación que protege
al host —coordenadas acotadas, nombres de tecla válidos, permisos, rutas
permitidas— está en crates sin plataforma, probados de forma exhaustiva en
cualquier máquina. El backend de Windows queda reducido a traducir órdenes ya
verificadas a llamadas del sistema
([ADR-0007](docs/architecture/ADR-0007-verificacion-multiplataforma.md)).

**3 · El servidor no ve nada.** Coordina el encuentro y entrega credenciales
TURN temporales, pero el vídeo y las órdenes de control van cifrados extremo a
extremo con DTLS entre los dos dispositivos
([ADR-0003](docs/architecture/ADR-0003-transporte-webrtc.md)).

### Seguridad

No se implementa ninguna primitiva criptográfica propia: `ed25519-dalek`,
`argon2`, `sha2`, `subtle`. Lo que sí es propio —y está documentado y
probado— es cómo se combinan:

- La transcripción que se firma incluye el **papel del firmante**, lo que
  impide que un atacante devuelva al host su propia firma.
- Las claves de orden pequeño se rechazan **al registrar el dispositivo**, no
  solo al verificar.
- El código de emparejamiento se deriva junto al **identificador del
  emparejamiento**, así que uno observado no sirve en otro.
- Un intento rechazado **no consume memoria** de la guardia antirreenvío.
- Rotar identificadores para esquivar el límite de intentos **no da intentos
  gratis**.

Lo que **no** protege todavía está en la pantalla de Seguridad del panel y en
[ADR-0004](docs/architecture/ADR-0004-identidad-criptografica.md).

---

## Lo siguiente

**Probar la etapa 2 en Windows.** El motor está escrito; falta ejecutarlo. En
un PC con Windows 10 u 11:

```bash
cd nexus-remote/apps/windows-host
npm install && npm run tauri dev
```

Y comprobar, en la pantalla *Escritorio remoto*, que aparecen las pantallas
reales con su resolución, posición y escala, y que el motor se declara
disponible. Lo que más probable es que necesite ajuste: la lectura de los
fotogramas (paso de fila y formato) y la marca de tecla extendida en teclados
no españoles.

**Desplegar el servidor.** La etapa 3 está terminada y probada, pero contra
sockets locales. Hay dos caminos:

- **En el mismo PC, gratis.** Si su operador le da una IP pública, el servidor
  cabe en el propio equipo: dos ejecutables, sin Docker ni Linux.
  [Guía paso a paso](docs/deployment/en-el-mismo-pc.md), con la comprobación
  previa de si su conexión lo permite.
- **En una máquina alquilada**, unos 3-6 € al mes, que además permite usar el
  relay TURN de verdad. [Guía completa](docs/deployment/despliegue.md).

En ambos casos, después: panel del host, *Configuración* → **Probar conexión**.
Esa comprobación se conecta de verdad y dice qué encontró.

Luego cliente Apple (4), funciones avanzadas (5) y producción (6). Ninguna
avanza hasta que la anterior funcione.

### Comprobar un servidor ya desplegado

```bash
cargo run -p nexus-windows-host --example comprobar_servidor -- \
  wss://su-dominio/v1/signaling "$NEXUS_HOST_ENROLLMENT_TOKEN"
```

Se conecta de verdad, se autentica y se declara disponible. Devuelve 0 solo si
además hay relay TURN, así que vale para un script de comprobación.

### Probar el canal de señalización sin servidor

Las dos cosas se pueden ver funcionando aquí mismo, sin desplegar nada:

```bash
cd nexus-remote
cargo test -p nexus-integration-tests --test senalizacion
cargo test -p nexus-integration-tests --test sesion_extremo_a_extremo -- --nocapture
```

La segunda levanta un servidor de señalización real en un puerto libre, conecta
dos dispositivos con claves Ed25519 propias, negocia WebRTC, verifica la atadura
a las huellas DTLS y entrega un evento de entrada al manejador del host.

### Lo que hace falta y no se puede resolver desde aquí

| Qué | Para qué | Cuándo |
| --- | --- | --- |
| Un PC con Windows 10 u 11 | Probar captura y control de entrada | Etapa 2 |
| Una IP pública en casa, o un servidor alquilado | Conectarse desde fuera de casa. Con IP pública no hace falta pagar nada: [guía para alojarlo en el mismo PC](docs/deployment/en-el-mismo-pc.md) | Etapa 3 |
| Un Mac con Xcode | Compilar el cliente de iPhone y iPad | Etapa 4 |
| Cuenta de desarrollador de Apple | Instalar en los dispositivos | Etapa 4 |

---

## Licencia

Propietaria. Uso privado.
