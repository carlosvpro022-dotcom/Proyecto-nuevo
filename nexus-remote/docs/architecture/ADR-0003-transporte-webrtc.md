# ADR-0003 · WebRTC para medios y canal de control

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

El objetivo es controlar el PC desde cualquier parte del mundo. Eso significa
atravesar el NAT del router de casa y, muchas veces, el CGNAT del operador
móvil. Y significa que el vídeo tiene que llegar con poca latencia por una red
que pierde paquetes.

## Decisión

**WebRTC**, con:

- **ICE + STUN** para intentar conexión directa entre los dos dispositivos.
- **TURN propio (Coturn)** como respaldo cuando la directa no es posible.
- **DTLS-SRTP** para el cifrado de medios, que WebRTC impone y no es opcional.
- **Canal de datos SCTP** para el control (ratón, teclado, portapapeles,
  archivos), cifrado por el mismo DTLS.

El servidor de señalización solo coordina el encuentro. No ve el contenido.

## Alternativas descartadas

- **Protocolo propio sobre TCP o QUIC.** Habría que implementar desde cero el
  atravesamiento de NAT, el control de congestión adaptado a vídeo y la
  negociación de cifrado. Es mucho trabajo para reinventar lo que WebRTC ya
  resuelve, y cada pieza es una oportunidad de equivocarse en seguridad.
- **RDP o VNC con un túnel.** RDP es excelente pero está atado a Windows y a
  su modelo de licencias y sesiones; VNC no tiene atravesamiento de NAT ni
  cifrado modernos. Ninguno da un cliente iOS nativo decente.
- **SSH con reenvío de puertos.** No resuelve el NAT del lado del móvil sin un
  servidor intermedio con IP pública, que es justo lo que queremos evitar que
  vea el tráfico.

## Consecuencias

- **El servidor TURN consume ancho de banda real** cuando se usa: todo el vídeo
  pasa por él. Hay que dimensionarlo y vigilarlo; está documentado en
  `docs/deployment/despliegue.md`.
- El TURN ve pasar paquetes cifrados pero **no puede descifrarlos**: las claves
  DTLS se negocian entre los dos extremos.
- H.264 es el codec preferido porque es el único con decodificación por
  hardware garantizada en iPhone y iPad. La negociación de códecs está en
  `nexus_core::capabilities`, con H.264 primero por orden de preferencia.
- La señalización necesita TLS por su cuenta: WebRTC cifra los medios, no el
  canal donde se negocian. De ahí que el host rechace cualquier URL que no
  empiece por `wss://`.
