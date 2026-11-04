# ADR-0013 · El token de confirmación nace sin validez

**Estado:** aceptada · **Fecha:** 2026-11-04

## Contexto

Reiniciar y apagar son las dos operaciones más destructivas del protocolo. Un
apagado equivocado deja el PC inalcanzable hasta que alguien vaya físicamente a
encenderlo, y si había trabajo sin guardar, se pierde.

El protocolo ya preveía una confirmación: una petición de apagado sin token se
contesta con `ConfirmationRequired { token, prompt }`, y quien pidió la
operación la reintenta adjuntando ese token.

**Implementado de la forma obvia, eso no confirma nada.** Quien pide recibe el
token al instante y lo reenvía acto seguido; nadie ha mirado nada en el equipo.
El campo se llama `prompt` —«texto que el host mostrará a la persona»— pero si
el token vale desde que se emite, da igual que se muestre o no: la operación se
ejecuta igual. Sería un paso de más que da sensación de seguridad sin añadir
ninguna, que es peor que no tener el paso.

Esto no lo cubre nada de lo que ya había:

- El **permiso** dice que ese dispositivo *puede* apagar. No dice que *ahora*
  deba hacerlo, ni que no sea un error, ni que no haya alguien usando el PC.
- La **atadura de sesión** ([ADR-0009]) garantiza que quien habla es el
  dispositivo autorizado y no un intermediario. Un dispositivo autorizado con
  el dedo torpe sigue apagando el equipo.

## Decisión

**El token nace sin validez.** Se emite, sí, pero canjearlo no hace nada hasta
que una persona delante del PC lo apruebe en el panel. Hasta entonces el host
contesta `awaiting_confirmation`, que es la verdad.

Hay **dos identificadores**, y no se pueden intercambiar:

| | Quién lo recibe | Para qué sirve |
| --- | --- | --- |
| **Token** (secreto) | Solo quien pidió la operación | Demostrar «soy el mismo que lo pidió» |
| **Identificador** (público) | Solo el panel del host | Aprobar o rechazar |

Quien pide nunca ve el identificador, así que no puede aprobarse a sí mismo. El
panel nunca necesita el token, así que no puede filtrarlo de vuelta.

El token se compara en **tiempo constante**, con el mismo `secret_matches` que
usa el servidor de señalización: es un secreto, y comparar con `==` permitiría
adivinarlo byte a byte midiendo lo que tarda.

Además:

- **Un solo uso.** Un canje correcto consume la confirmación. El mismo token no
  puede apagar el equipo dos veces.
- **Caduca a los dos minutos.** Cubre el tiempo de acercarse al PC y mirar, sin
  dejar viva una aprobación de hace media hora.
- **Va atada a la operación.** La confirmación de un reinicio no sirve para un
  apagado.
- **Muere con la sesión.** Si la sesión se cae y vuelve, hay que volver a
  preguntar.
- **Solo hay una a la vez.** Permitir varias dejaría a quien pide llenar el
  panel de avisos hasta que alguien aprobara uno por cansancio, que es justo lo
  que una confirmación debe evitar. Reintentar la misma operación devuelve la
  misma confirmación, no una nueva.

**Bloquear la sesión no pide confirmación.** Es reversible y no pierde nada;
pedirla ahí solo enseñaría a la gente a aprobar sin leer, que es como se
desactivan las confirmaciones de verdad.

### Dónde vive la puerta

Las dos mitades ocurren en sitios distintos del programa: la pide la sesión
remota y la aprueba una persona en la ventana del panel. La puerta
(`SharedConfirmations`) se comparte entre las dos, y el aviso se dibuja **encima
de cualquier pantalla** del panel: una operación que espera decisión no puede
quedarse escondida en una pestaña que nadie mire.

## Consecuencias

Lo que se gana: apagar el PC desde fuera exige que alguien esté delante y diga
que sí. Es, de hecho, una limitación deliberada —no se puede apagar un PC vacío
en remoto— y es la correcta mientras no exista una forma de distinguir «el dueño
quiere apagarlo» de «alguien con el dispositivo del dueño quiere apagarlo».

Lo que cuesta: el caso de uso «apagar el PC de casa desde el trabajo» **no
funciona**, porque no hay nadie en casa para aprobarlo. Si llega a hacer falta,
la salida no es quitar la confirmación: es una aprobación previa explícita del
dueño en el panel («permitir apagados sin preguntar durante las próximas N
horas»), que deja la decisión donde debe estar.

## Cómo se comprueba

La prueba que sostiene todo esto es
`con_el_token_en_la_mano_pero_sin_aprobar_no_se_apaga_nada`. Se verificó que
tiene dientes sustituyendo la puerta por el diseño ingenuo —token válido desde
que se emite—: fallan nueve pruebas, esa entre ellas.

El backend que llama a Windows es lo único de este diseño que no se ejecuta
aquí: una prueba automática de «apagar el equipo» apagaría el equipo donde
corre. Por eso lo que sí se prueba, y a fondo, es todo lo que decide si se llega
hasta él.

[ADR-0009]: ADR-0009-atadura-de-sesion.md
