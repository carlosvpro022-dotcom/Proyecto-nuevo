# Retransmisión (relay)

**No se va a implementar un servidor propio: se usa Coturn.**

Esta carpeta existe porque el relay es una pieza del sistema, pero el software
que la cubre no lo escribimos nosotros. Coturn es la implementación de
referencia de STUN y TURN, está mantenida y auditada, y escribir una propia
sería reinventar un protocolo delicado sin ganar nada.

Lo que sí es nuestro:

- La configuración de Coturn, en
  [`infrastructure/turn/turnserver.conf.template`](../../infrastructure/turn/turnserver.conf.template),
  con las restricciones que impiden usar el relay para alcanzar la red interna
  del servidor.
- La derivación de credenciales temporales, en
  [`server/signaling/src/turn.rs`](../signaling/src/turn.rs), compatible con el
  mecanismo `use-auth-secret` de Coturn y verificada contra un vector de prueba
  público de HMAC-SHA1.

El razonamiento completo está en
[ADR-0006](../../docs/architecture/ADR-0006-infraestructura-propia.md).
