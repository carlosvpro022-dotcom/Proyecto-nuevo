# ADR-0006 · Infraestructura propia con Coturn

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

El requisito es no depender de servidores comerciales de escritorio remoto. Eso
descarta usar la infraestructura de AnyDesk, TeamViewer o similares, pero deja
abierta la pregunta de qué hace falta montar realmente.

## Decisión

Tres piezas en una sola máquina modesta:

| Servicio | Papel | Expuesto |
| --- | --- | --- |
| **Caddy** | Proxy inverso, TLS automático | 80, 443 |
| **nexus-signaling** | Encuentro y negociación | solo red interna |
| **Coturn** | STUN y TURN | 3478, 5349, UDP 49160-49200 |

Puntos concretos:

- **STUN propio, no `stun.l.google.com`.** Un servidor STUN ve qué direcciones
  IP intentan conectarse entre sí y cuándo. Es poca información, pero es
  información sobre el uso del sistema, y el requisito era no depender de
  terceros.
- **Credenciales TURN temporales** derivadas por HMAC-SHA1 de un secreto
  compartido, con caducidad. Coturn recalcula el mismo HMAC, así que no hace
  falta ninguna comunicación entre los dos servicios ni una lista de usuarios.
- **El servidor de señalización no termina TLS.** Lo hace Caddy. Así el
  proceso que escribimos nosotros no gestiona certificados ni renovaciones, y
  no se expone directamente a Internet.
- **Coturn con la red del host.** Con la red puenteada de Docker anunciaría la
  IP del contenedor, que no sirve desde Internet.

## Alternativas descartadas

- **Solo STUN, sin TURN.** Funciona en muchos casos, pero falla justo cuando
  más falta hace: CGNAT de operador móvil y NAT simétrico de redes
  corporativas. Sin TURN, el sistema no cumple el objetivo de funcionar desde
  cualquier parte.
- **Nginx o Traefik en lugar de Caddy.** Funcionan igual de bien. Caddy se
  elige porque obtiene y renueva los certificados sin configuración adicional,
  que es un error menos en un despliegue que se mantiene solo.
- **Un servidor TURN gestionado (Twilio, Xirsys).** Es cómodo y barato, pero
  vuelve a introducir un tercero en la ruta del tráfico.

## Consecuencias

- **El TURN cuesta ancho de banda.** Una sesión de escritorio a 5 Mbps que pase
  por el relay consume 5 Mbps de subida y 5 de bajada en el servidor. Conviene
  vigilarlo.
- Hay que abrir puertos en el cortafuegos, incluido un rango UDP. Está
  documentado en `docs/deployment/despliegue.md`.
- Coturn necesita los mismos certificados que Caddy para servir TURNS en el
  5349. El despliegue los comparte por volumen, y la rotación tras una
  renovación hay que verificarla.
- **Sin validar todavía:** este despliegue no se ha ejecutado. El `compose.yaml`
  pasa `docker compose config`, pero no se ha levantado ningún contenedor
  porque el entorno de desarrollo no tiene demonio Docker.
