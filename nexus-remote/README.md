# NEXUS REMOTE

Alternativa privada a AnyDesk: controlar un PC con Windows desde un iPhone o un
iPad, desde cualquier parte del mundo, con infraestructura propia y acceso
exclusivo a los dispositivos previamente autorizados.

---

## Estado real del proyecto

**Etapa 1 terminada. Etapa 2 escrita y compilada, pendiente de probar en
Windows.** Lo que sigue es exacto, y distingue tres cosas: lo que se ha
ejecutado y probado, lo que está escrito y compila para Windows pero **nadie
ha ejecutado allí**, y lo que todavía no existe.

### Funciona y está probado

| Pieza | Qué hace |
| --- | --- |
| **Contratos del protocolo** | Mensajes de señalización y de control, permisos por operación, capacidades negociables, perfiles de calidad, monitores, máquina de estados de sesión. |
| **Identidad criptográfica** | Ed25519 por dispositivo, persistente entre arranques. Verificación estricta de firmas. |
| **Autenticación mutua** | Reto de un solo uso firmado por ambas partes, con defensa contra reflexión y contra reenvío. |
| **Emparejamiento** | Código temporal de 8 caracteres y QR. Derivado con Argon2id, 5 minutos, 5 intentos. |
| **Lista de autorizados** | SQLite con permisos por dispositivo, revocación, borrado e historial de sesiones. |
| **Límite de intentos** | Bloqueo progresivo por dispositivo. Esperar a que expire no reinicia la escalada. |
| **Restricción de acceso al disco** | Solo las carpetas declaradas. Rechaza recorrido de directorios, enlaces que escapan y nombres reservados de Windows. |
| **Credenciales TURN temporales** | Derivación HMAC-SHA1 compatible con Coturn, verificada contra un vector de prueba público. |
| **Servidor de señalización** | Arranca, valida su configuración, responde `/health` y `/v1/info`, y para de forma ordenada. |
| **Panel del host** | Aplicación Tauri con 8 pantallas. Muestra datos reales del equipo. |

### Escrito y compila para Windows, pero sin ejecutar allí

Esto es la etapa 2. El compilador verificó tipos, nombres de API y firmas
mediante compilación en cruce a `x86_64-pc-windows-gnu`, con clippy pedantic y
los avisos como errores. **Nadie lo ha ejecutado en un PC con Windows**, así
que el comportamiento está sin comprobar.

| Pieza | Cómo está implementada |
| --- | --- |
| **Detección de monitores** | `EnumDisplayMonitors`, `GetMonitorInfoW`, `GetDpiForMonitor` y `EnumDisplaySettingsW`. Devuelve resolución, posición, escala, refresco y cuál es la principal. |
| **Captura de pantalla** | Windows Graphics Capture sobre Direct3D 11: cola de fotogramas sin hilo propio, copia a textura de preparación y lectura BGRA. Rehace la cola al cambiar la resolución y recupera el fotograma a la cola pase lo que pase. |
| **Control de ratón y teclado** | `SendInput` con coordenadas absolutas sobre el escritorio virtual, tabla de ~110 teclas con su marca de extendida, texto por `KEYEVENTF_UNICODE` (pares subrogados incluidos) y liberación de todo al cerrar. |

Lo que **sí** está probado de esa etapa es la parte portable, que es donde
están los errores difíciles: la tabla de teclas, la aritmética de coordenadas
(verificada píxel a píxel sobre siete configuraciones de pantalla) y la
conversión de los datos crudos del sistema.

### Todavía no existe

| Pieza | Etapa | Qué falta |
| --- | --- | --- |
| Canal de señalización | 3 | El WebSocket `/v1/signaling`. La maquinaria de autenticación que necesita ya existe. |
| Sesión WebRTC | 3 | Negociación ICE y canales de medios y datos. |
| Cliente de iPhone y iPad | 4 | El proyecto entero. **Hace falta un Mac con Xcode.** |
| Audio, archivos, portapapeles, administración | 5 | Las llamadas al sistema. Protocolo, permisos y contabilidad están listos. |
| Cifrado de la clave del host con DPAPI | 6 | Hoy la clave privada está en un archivo sin cifrar. |

El panel refleja esta misma distinción con tres estados —*Probado*, *Sin
probar*, *Pendiente*—, y una prueba impide marcar como probado código de
Windows que nadie ha ejecutado en Windows. Además del estado del proyecto, el
panel comprueba **este equipo concreto**: intenta enumerar las pantallas y
crear el inyector, y enseña lo que ocurrió.

**No hay datos de ejemplo en ninguna parte**
([ADR-0008](docs/architecture/ADR-0008-sin-datos-simulados.md)).

### Verificación

```
475 pruebas automatizadas, todas en verde
  · nexus-security        102    criptografía, emparejamiento, antirreenvío
  · nexus-core             98    protocolo, permisos, estados
  · nexus-windows-host     62    almacén, identidad, servicio del host, comandos
  · nexus-input            44    tabla de teclas, coordenadas absolutas
  · nexus-capture          41    fotogramas, ritmo, enumeración de monitores
  · nexus-file-transfer    37    restricción de acceso, transferencias
  · nexus-transport        28    códec, ICE, enlace
  · nexus-signaling        27    configuración, TURN, rutas
  · integración            22    recorridos completos entre crates
  · nexus-audio            14    formato, volumen

cargo fmt            sin diferencias
clippy::pedantic     sin un solo aviso, con -D warnings, en Linux y en Windows
panel                8 pantallas verificadas en Chromium con datos reales del backend
cruce a Windows      clippy --target x86_64-pc-windows-gnu limpio en los 7 crates
servidor             arranca, responde /health y /v1/info, y para con SIGTERM
```

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

Después, **etapa 3 — Comunicación**: el WebSocket `/v1/signaling` y WebRTC.
Luego cliente Apple (4), funciones avanzadas (5) y producción (6). Ninguna
avanza hasta que la anterior funcione.

### Lo que hace falta y no se puede resolver desde aquí

| Qué | Para qué | Cuándo |
| --- | --- | --- |
| Un PC con Windows 10 u 11 | Probar captura y control de entrada | Etapa 2 |
| Un servidor con IP pública y un dominio | Señalización y TURN | Etapa 3 |
| Un Mac con Xcode | Compilar el cliente de iPhone y iPad | Etapa 4 |
| Cuenta de desarrollador de Apple | Instalar en los dispositivos | Etapa 4 |

---

## Licencia

Propietaria. Uso privado.
