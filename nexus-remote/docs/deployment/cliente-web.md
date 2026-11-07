# El cliente web: estado real

**Fecha:** 2026-11-07

Un navegador puede ser el dispositivo que ve y controla el PC, sin App Store y
sin un Mac para compilar nada. Esta página dice **exactamente** qué funciona ya
y qué no, comprobado ejecutándolo con un navegador de verdad contra el host de
verdad.

La comprobación se ejecuta así, y arranca ella sola el servidor de
señalización, el host y Chromium:

```bash
node scripts/verificar-cliente-web.mjs
```

Deja capturas y el registro de pasos en `target/capturas-cliente-web/`.

## Lo que ya funciona, ejecutado

| Pieza | Comprobado |
| --- | --- |
| **Identidad en el navegador** | El navegador se genera una clave Ed25519 con `crypto.subtle`. La privada **no es extraíble**: ningún script puede leerla, ni la propia página. Vive en IndexedDB y sobrevive a recargar. |
| **Autenticación contra el servidor** | Saludo, reto y firma. La transcripción que se firma coincide byte a byte con la de Rust. |
| **Sesión con el PC** | El host decide contra su registro local y acepta. |
| **Atadura de sesión** | El navegador firma las dos huellas DTLS y el host la verifica: sin eso el host no abre nada. |
| **Negociación WebRTC** | ICE encuentra camino, DTLS cifra (`TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`) y SCTP queda establecido. |
| **Pista de vídeo** | El navegador recibe la pista H.264 que anuncia el host, con el mismo perfil (`42001f`) que decodifica un iPhone por hardware. |

## Lo que todavía no funciona

**No hay imagen.** El host abre sus canales de datos (`control`, `input`,
`files`) y SCTP queda establecido en los dos extremos, pero **el navegador no
recibe el evento `datachannel`**: nunca ve esos canales. Sin canal de control
no se puede pedir la pantalla, y el host no envía un solo fotograma hasta que
se la piden.

Medido en el host: la asociación SCTP pasa a `Established` y la biblioteca dice
*«send data channel message»*. Medido en el navegador: `sctp.state` es
`connected` y no llega ningún canal. Ese es el único eslabón que falta.

## Dos limitaciones que hay que resolver aparte

**Nombres mDNS.** Chrome y Safari esconden la IP local detrás de un nombre
`.local` (por privacidad, y no se puede desactivar en un iPhone). El host no
sabe resolverlos y descarta ese camino, así que **en red local no habría
conexión directa**. Hace falta activar la resolución mDNS en el host. Fuera de
la red local no estorba: ahí los caminos que valen son los que da el servidor
STUN/TURN, que llevan direcciones reales.

Mientras eso no esté, la comprobación se ejecuta con
`NEXUS_SIN_MDNS=1 node scripts/verificar-cliente-web.mjs`, que le pide a
Chromium que no esconda la IP. **Eso es una muleta de la prueba, no del
producto**: un iPhone no tiene ese interruptor.

**Caminos uno a uno (`trickle ICE`).** Un camino que llega antes de la
respuesta ya no tumba la sesión (eso se arregló), pero los caminos que el
navegador envía uno a uno siguen sin llegar al agente ICE del host. Por eso el
cliente espera a tener un camino y lo manda **dentro** de la respuesta, que es
de donde el host sí los lee. Funciona, pero en una red lenta o con TURN añade
espera.

## Lo que no se ha probado

- **Ratón y teclado.** El código está escrito y envía los eventos del
  protocolo, pero no se ha ejecutado contra un host que pueda inyectarlos: en
  Linux no hay inyección. Se probará en Windows.
- **Un iPhone de verdad.** Lo comprobado es Chromium en esta máquina. Safari
  admite todo lo que esto usa, pero no está ejecutado.
