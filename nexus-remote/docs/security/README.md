# Seguridad

## Dónde está documentado qué

| Pregunta | Dónde |
| --- | --- |
| Qué protege el sistema y por qué así | [ADR-0004](../architecture/ADR-0004-identidad-criptografica.md) |
| Qué ve y qué no ve el servidor | [ADR-0003](../architecture/ADR-0003-transporte-webrtc.md) |
| A qué parte del disco puede llegar un dispositivo | [ADR-0012](../architecture/ADR-0012-transferencia-de-archivos.md) |
| Por qué apagar el PC exige que alguien esté delante | [ADR-0013](../architecture/ADR-0013-confirmacion-en-el-host.md) |
| Cómo se endurece el despliegue | [guía de despliegue, §10](../deployment/despliegue.md) |
| Estado actual, medida a medida | Pantalla **Seguridad** del panel del host |

Cada medida tiene su módulo con sus pruebas. La pantalla de Seguridad del panel
las lista con el módulo donde vive cada una, para que se pueda ir del nombre al
código.

## Lo que todavía no protege

Está en la misma pantalla del panel, y aquí por escrito:

| Pendiente | Etapa | Riesgo mientras tanto |
| --- | --- | --- |
| Medios cifrados extremo a extremo en marcha | 3 | DTLS-SRTP está elegido pero no hay sesión todavía |
| TLS de la señalización probado de extremo a extremo | 3 | El despliegue está escrito pero no ejecutado |
| Revisión de seguridad del sistema en marcha | 6 | Nada se ha probado contra un sistema funcionando |

### Sobre la clave privada del host

Ya **se cifra**. En Windows con DPAPI, que ata el descifrado a la cuenta: el
archivo copiado a otro equipo o leído desde otra cuenta no sirve. Una clave
guardada antes de que existiera el cifrado se migra al arrancar, sin cambiar la
identidad del equipo.

Lo que DPAPI **no** evita: un programa que corra como esa misma cuenta puede
pedirle que descifre, igual que hace el host. Sube el listón —ya no basta con
copiar el archivo— pero no lo hace imposible. Evitarlo exigiría pedir una
contraseña en cada arranque, y eso es incompatible con el acceso desatendido,
que es el motivo de existir de este producto.

Fuera de Windows no hay almacén del sistema y la clave queda en claro. El panel
lo dice, en lugar de dejar la limitación escondida en un comentario.

### Sobre el acceso al disco

Dos cosas que conviene tener claras de la transferencia de archivos:

- **Un dispositivo con permiso de archivos no ve nada** hasta que el dueño del
  PC declare una carpeta compartida, y solo ve lo que haya dentro. Son dos
  concesiones separadas a propósito.
- **El resumen `SHA-256` protege de la corrupción, no del engaño.** Lo calcula
  quien envía, así que un remitente malicioso puede mandar otro contenido con
  su resumen correcto. De eso protege el cifrado del canal y la atadura de
  sesión ([ADR-0009](../architecture/ADR-0009-atadura-de-sesion.md)), que es lo
  que garantiza que quien envía es el dispositivo autorizado y no un
  intermediario.

Y una decisión consciente: el host **contesta a cada marco de archivo que
rechaza**, diciendo por qué. Un dispositivo autenticado podría inundar el canal
de archivos con basura y provocar una respuesta por cada marco. No se acota
porque ese mismo dispositivo ya podría inundar el canal de control
directamente: no abre una puerta que no estuviera abierta, y a cambio un
cliente legítimo con un error recibe el motivo en lugar de silencio.

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
