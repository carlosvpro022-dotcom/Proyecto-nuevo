# ADR-0011 · El servidor conoce identidades, no permisos

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

Para verificar la firma de quien se conecta, el servidor de señalización
necesita su clave pública. Eso obliga a decidir dos cosas que es fácil
confundir: **quién es** un dispositivo, y **qué puede hacer**.

Si el servidor guardase también los permisos, un servidor comprometido podría
autorizarse dispositivos a sí mismo y el resto de las defensas no servirían de
nada: ya serían dispositivos legítimos.

Y hay un segundo problema, más silencioso. Si el servidor acepta cualquier
identificador que se le presente con cualquier clave, el primero que llegue se
queda con el identificador de un host que todavía no se ha conectado. A partir
de ahí, cuando el host de verdad intente conectarse, no podrá.

## Decisión

El servidor guarda **una sola cosa**: qué clave pública corresponde a qué
identificador de dispositivo. Nada más. Los permisos los decide el host, cada
vez que llega una petición de sesión, contra su propio registro en SQLite.

Las reglas del directorio (`server/signaling/src/enrollment.rs`):

| Situación | Qué hace el servidor |
|---|---|
| Identificador conocido, clave coincide | lo admite |
| Identificador conocido, **clave distinta** | lo rechaza (intento de apropiación) |
| Identificador conocido, revocado | lo rechaza |
| Identificador nuevo, papel **host** | exige el testigo de alta del servidor |
| Identificador nuevo, papel **controlador** | lo acepta y ata la clave a ese identificador |
| Directorio lleno | lo rechaza |

Un alta que no llega a demostrar la clave privada **se deshace**: si no se
deshiciera, bastaría presentarse en bucle con identificadores nuevos para llenar
el directorio.

### Por qué un host necesita testigo y un controlador no

Son dos preguntas distintas.

Para el **host**, la pregunta es "¿quién puede quedarse con este
identificador?", y la respuesta tiene que venir de alguien que administre el
servidor, porque nadie más puede decidirlo. De ahí el testigo, que es un secreto
de despliegue, se compara en tiempo constante y no se guarda en el host después
de usarlo.

Para el **controlador**, la pregunta que importa no se hace aquí. Aunque un
desconocido ate su clave a un identificador nuevo, no consigue nada: cuando pida
sesión, el host comprobará si esa clave está en su lista de dispositivos
emparejados, y no lo estará. Exigir testigo a cada iPad obligaría a repartir un
secreto del servidor a cada dispositivo para no ganar nada.

### Por qué el rechazo es siempre el mismo hacia fuera

El servidor distingue internamente seis motivos y los anota todos, pero al
cliente le dice siempre "credenciales no válidas". El orden de comprobaciones
heredado de la etapa 1 mira el registro **antes** que la firma, de modo que un
motivo específico permitiría averiguar si un identificador existe, o si está
revocado, sin tener la clave privada.

La única excepción es el límite de intentos, porque el cliente necesita saber
cuánto esperar para no insistir en bucle; y la revocación cuando la clave
presentada **sí** coincide con la registrada, caso en el que ya se ha demostrado
que es el dispositivo de verdad y no se le revela nada que no sepa.

## Consecuencias

A favor:

- Un servidor comprometido no puede conceder permisos ni suplantar dispositivos.
- Nadie puede apropiarse del identificador de un host.
- El flujo de autenticación de la etapa 1 (`Authenticator`, con su límite de
  intentos, su protección contra repetición y su reto de un solo uso) se
  reutiliza tal cual: el directorio solo cambia de dónde salen las claves.

En contra:

- Hay que administrar un secreto más en el despliegue, y el usuario tiene que
  introducirlo una vez en el panel del host.
- El directorio vive en memoria: al reiniciar el servidor, los hosts tendrán que
  presentar el testigo otra vez. Es asumible para un servicio de uso personal y
  evita una base de datos en el servidor, que es superficie de ataque añadida.
  Si llega a molestar, lo que corresponde es persistir el directorio, no quitar
  la comprobación.
