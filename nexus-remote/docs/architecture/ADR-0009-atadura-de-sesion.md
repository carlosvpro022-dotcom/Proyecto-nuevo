# ADR-0009 · La sesión se ata a la huella DTLS del otro extremo

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

El servidor de señalización reenvía las descripciones SDP entre los dos
dispositivos. Una descripción SDP contiene la **huella del certificado DTLS**
del emisor, y esa huella es lo que cada extremo usa para comprobar con quién
está hablando por el canal cifrado.

De ahí sale un ataque que el diseño anterior no cubría. Un servidor de
señalización comprometido puede sustituir las dos huellas por las suyas, abrir
dos sesiones DTLS —una con cada extremo— y leer todo lo que pasa: la pantalla,
las pulsaciones de teclado, los archivos. El cifrado seguiría activo y ninguno
de los dos extremos notaría nada.

Esto **no lo evita** ninguna de las medidas de la etapa 1:

- El TLS del WebSocket protege el tramo contra terceros, no contra el propio
  servidor, que es el extremo del TLS.
- La autenticación del dispositivo ante el servidor demuestra quién se conecta
  al servidor, no con quién acaba hablando.
- El `AuthTranscript` de la etapa 1 firma nonces e identificadores, pero nada
  que ate esa firma al canal cifrado concreto de la sesión.

Y es un ataque que importa de verdad en este producto: el servidor es una
máquina expuesta a Internet, y el proyecto parte de que el contenido de la
sesión no debe estar a su alcance.

## Decisión

Después de intercambiar oferta y respuesta, cuando cada extremo ya conoce las
dos huellas DTLS, **cada uno firma las dos** con su clave Ed25519 y comprueba la
firma del otro contra la huella que negoció de verdad.

La transcripción firmada, en `nexus_security::binding`, lleva:

```text
"NEXUS-REMOTE/v1/session-binding" | 0x00 | papel | versión
  | id de sesión | dispositivo local | dispositivo remoto
  | huella local | huella remota
```

Si el servidor sustituye una huella, la firma del otro extremo cubre una huella
distinta de la observada y la sesión se aborta con `FingerprintMismatch`. El
servidor no puede forjar la firma: no tiene las claves privadas, que no salen de
los dispositivos.

### Por qué no hace falta un nonce adicional

La biblioteca WebRTC genera un certificado DTLS nuevo en cada conexión, de modo
que el par de huellas ya es distinto en cada sesión. Reproducir una firma
antigua obligaría al atacante a presentar el certificado antiguo, y para
completar el DTLS necesitaría su clave privada.

### Por qué el papel va dentro

Sin el byte de papel, la transcripción del host y la del controlador serían
iguales y el servidor podría devolver a un extremo su propia firma haciéndole
creer que el otro contestó. Con el papel dentro, una firma no vale reflejada.

### Por qué el dominio es distinto del de autenticación

Para que una firma hecha para autenticarse ante el servidor no pueda colarse
como atadura de sesión, ni al revés. Hay una prueba que lo comprueba firmando
una transcripción de autenticación e intentando usarla como atadura.

### De dónde saca cada extremo la clave del otro

Este es el punto donde todo lo anterior se sostiene o se cae:

- El **host** la tiene en su registro de dispositivos emparejados (SQLite).
- El **controlador** la recibe en la invitación de emparejamiento, que viaja
  fuera de banda (un QR en la pantalla del host).

Por eso la invitación lleva ahora la clave pública completa del host y no sólo
su huella corta, y por eso el mensaje `PairingCompleted` del servidor **no**
lleva la clave del host: si el controlador la aceptase del servidor, estaría
confiando en el servidor justo para lo que esta decisión pretende no confiar.

## Consecuencias

A favor:

- Un servidor de señalización comprometido puede negar el servicio, pero no
  leer una sesión ni suplantar a un dispositivo.
- El relay TURN sigue siendo lo que debe: un reenviador de paquetes cifrados.
- Se comprueba con pruebas que ejecutan el ataque: sustitución de huella, firma
  del atacante con su propia clave, reflexión de una firma, transferencia a otra
  sesión y a otro par de dispositivos.

En contra:

- Una vuelta más de mensajes antes de dar la sesión por buena.
- El cliente de Apple tendrá que implementar la misma transcripción byte a byte.
  Está documentada arriba y el formato es de longitud fija a propósito, para que
  no haya ambigüedad al reimplementarla en Swift.
- Si el certificado DTLS se reutilizase entre sesiones (hoy no ocurre), haría
  falta añadir nonces. Queda anotado aquí para que no se descubra por sorpresa.

## Lo que esta decisión no cubre

No sustituye al emparejamiento. Garantiza que el canal cifrado va contra el
dispositivo que posee la clave esperada; quién decide que una clave es de
confianza sigue siendo el host, en su registro.
