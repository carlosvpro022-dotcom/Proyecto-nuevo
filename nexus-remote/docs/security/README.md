# Seguridad

## Dónde está documentado qué

| Pregunta | Dónde |
| --- | --- |
| Qué protege el sistema y por qué así | [ADR-0004](../architecture/ADR-0004-identidad-criptografica.md) |
| Qué ve y qué no ve el servidor | [ADR-0003](../architecture/ADR-0003-transporte-webrtc.md) |
| Cómo se endurece el despliegue | [guía de despliegue, §10](../deployment/despliegue.md) |
| Estado actual, medida a medida | Pantalla **Seguridad** del panel del host |

Cada medida tiene su módulo con sus pruebas. La pantalla de Seguridad del panel
las lista con el módulo donde vive cada una, para que se pueda ir del nombre al
código.

## Lo que todavía no protege

Está en la misma pantalla del panel, y aquí por escrito:

| Pendiente | Etapa | Riesgo mientras tanto |
| --- | --- | --- |
| Cifrar la clave privada del host con DPAPI | 6 | En Windows, otro proceso del mismo usuario puede leerla del disco |
| Medios cifrados extremo a extremo en marcha | 3 | DTLS-SRTP está elegido pero no hay sesión todavía |
| TLS de la señalización probado de extremo a extremo | 3 | El despliegue está escrito pero no ejecutado |
| Revisión de seguridad del sistema en marcha | 6 | Nada se ha probado contra un sistema funcionando |

## Si encuentra un fallo de seguridad

El proyecto es privado y de un solo usuario. No hay proceso formal de informes:
anótelo como incidencia en el repositorio o corríjalo.

## Principios que se siguen

1. **No se implementa criptografía propia.** Solo se combinan bibliotecas
   mantenidas: `ed25519-dalek`, `argon2`, `sha2`, `subtle`.
2. **Lista blanca, nunca lista negra.** Lo que no se concede, no se puede
   hacer. Un conjunto de permisos vacío deniega absolutamente todo, y hay una
   prueba que lo comprueba operación por operación.
3. **Fallar de forma visible.** Un backend sin implementación devuelve un
   error explícito, nunca un valor vacío que parezca un resultado válido.
4. **Lo que no está probado no se declara terminado.**
   ([ADR-0007](../architecture/ADR-0007-verificacion-multiplataforma.md))
