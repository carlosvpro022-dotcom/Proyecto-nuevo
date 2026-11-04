# ADR-0012 · La transferencia de archivos va por su propio canal binario y el destino se publica solo cuando el contenido está verificado

**Estado:** aceptada · **Fecha:** 2026-11-04

## Contexto

El canal de control de la sesión lleva JSON por un canal de datos de WebRTC
ordenado y fiable. Las órdenes que viajan por ahí son pequeñas: un movimiento de
ratón, una pulsación, una petición de listado. Un archivo no se parece a eso en
nada: son cientos de megabytes que tardan minutos.

Meter los archivos en ese mismo canal tiene dos consecuencias que no se
arreglan afinando nada.

La primera es el tamaño: JSON no lleva bytes crudos, así que habría que
codificarlos en base64 y pagar un 33 % de más en cada byte de cada archivo.

La segunda es peor. Un canal ordenado y fiable entrega los mensajes en el orden
en que se enviaron. Una descarga de dos gigabytes delante de un movimiento de
ratón significa un ratón congelado hasta que la descarga termine. No es una
cuestión de prioridades del programa: es el orden del propio canal.

Y hay una tercera cuestión, que es de corrección y no de rendimiento: **qué
queda en el disco cuando una transferencia se corta**. Una sesión que se corta a
mitad de una subida es lo normal en un móvil, no la excepción.

## Decisión

### Un canal aparte, en binario

Los archivos viajan por un canal de datos propio (`files`), con un formato
binario de cabecera fija:

```text
posición  bytes  campo
0         1      versión del formato
1         1      tipo de marco (datos, fin, aborto)
2         16     identificador de la transferencia
18        4      número de secuencia (big endian)
22        ...    carga
```

La cabecera es de tamaño fijo y **no declara longitudes**. El canal de datos
conserva los límites de cada mensaje, así que el tamaño del marco es el del
mensaje: no existe el error clásico de creerse una longitud que no se
corresponde con los bytes recibidos.

El trozo es de **63 KiB**, que no es un número redondo por un motivo concreto:
un mensaje del canal de datos no puede pasar de 65 536 bytes, que es el valor
por omisión que fija el [RFC 8841] para SCTP y el que aplican tanto esta
biblioteca como la pila WebRTC de los navegadores y de iOS. El trozo tiene que
dejar sitio a la cabecera dentro de ese tope. Hay una comprobación en tiempo de
compilación que lo impide romper, porque el fallo, sin ella, solo aparecería al
mover un archivo grande en hardware real.

### El destino se publica cuando el contenido está verificado, no antes

Quien recibe **no escribe en la ruta de destino**. Escribe en un archivo
temporal de la misma carpeta y solo le pone su nombre definitivo cuando se
cumplen tres cosas: que llegaron todos los trozos que anunció quien envía, que
la cuenta de bytes coincide con lo anunciado, y que el `SHA-256` del contenido
escrito coincide con el anunciado.

El motivo es lo que pasa si no se hace. Escribiendo directamente en el destino,
una transferencia cortada a mitad deja ahí un archivo truncado con el nombre
correcto, indistinguible de uno completo. Y si en esa ruta ya había un archivo,
se habría destruido para dejar un resto.

El temporal va en la misma carpeta para que publicarlo sea un cambio de nombre
dentro del mismo sistema de archivos, y no una copia. Si nadie cierra la
transferencia, el temporal se borra al soltar el escritor: una sesión que se cae
no deja la carpeta compartida llenándose de restos.

El resumen protege de la corrupción, no del engaño: lo calcula quien envía, así
que un remitente malicioso puede mandar otro contenido con su resumen correcto.
De eso protege el cifrado del canal y la atadura de sesión del [ADR-0009], no
esta capa.

### Una subida no está completa cuando llegan sus bytes

La contabilidad de una **descarga** se cierra sola cuando sale el último byte:
el host ya no tiene nada más que hacer. La de una **subida**, no. Entre el
último byte y el final hay una comprobación de integridad, y darla por completa
antes sería anunciar como terminado algo que todavía puede rechazarse. Quien
verifica es quien la cierra.

### Compartir una carpeta y tener permiso son dos concesiones distintas

Un dispositivo con permiso de archivos no ve nada hasta que el dueño del PC
declare una carpeta compartida, y solo ve lo que haya dentro de ella. La lista
vacía es el valor por omisión.

Una ruta que queda fuera de las carpetas compartidas y una que no existe dan el
**mismo** rechazo, con el mismo código y el mismo texto. Distinguirlas
permitiría averiguar qué hay en el disco del host preguntando una ruta tras
otra.

### Sobrescribir se rechaza, por ahora

Una subida cuyo destino ya existe se rechaza y se dice por qué. Sobrescribir un
archivo de alguien es una acción que debería pedir confirmación en el host, y
esa confirmación todavía no existe. Hasta que exista, rechazar es la respuesta
honesta; permitirlo en silencio no lo sería.

### El listado se pagina

Un listado viaja por el canal de control, que tiene un tope de 64 KiB por
mensaje. Una carpeta de descargas con tres mil archivos no cabe. Truncar
significaría que la persona ve doscientos de tres mil sin forma de llegar al
resto: un límite que se notaría el primer día. Así que se pagina, con el total
en cada respuesta.

Paginar obliga a un orden estable entre páginas, y eso obliga a leer la carpeta
entera y ordenarla en cada petición, porque `read_dir` no garantiza ningún
orden. Es lo que cuesta hacerlo bien: sin ordenar completo, un archivo podría
salir en dos páginas, o en ninguna.

## Consecuencias

Lo que se gana:

- El ratón no espera detrás de un archivo, y los bytes no pagan el 33 % de
  base64.
- En la carpeta compartida solo aparecen archivos completos y verificados.
  Nunca uno a medias con nombre de bueno.
- La memoria que usa una transferencia no depende del tamaño del archivo. Para
  que eso fuera cierto hubo que acotar el buffer de envío del canal de datos,
  que la biblioteca trae **sin límite** por omisión: con el valor de serie, un
  archivo de varios gigas se habría acumulado entero en la memoria del host.

Lo que cuesta:

- Un canal de datos más por sesión.
- Dos formatos que mantener a la vez: JSON en el control y binario en los
  archivos. El formato binario lleva su versión en el primer byte para que un
  cambio futuro se pueda rechazar en lugar de malinterpretarse.
- Listar una carpeta enorme recorre la carpeta entera en cada página.

## Lo que queda pendiente

- **Sobrescribir con confirmación en el host.** Hoy se rechaza.
- **Reanudar una transferencia cortada.** Hoy se empieza de nuevo.
- **Carpetas enteras.** Hoy se transfiere archivo por archivo.

[RFC 8841]: https://datatracker.ietf.org/doc/html/rfc8841#section-6.1-4
[ADR-0009]: ADR-0009-atadura-de-sesion.md
