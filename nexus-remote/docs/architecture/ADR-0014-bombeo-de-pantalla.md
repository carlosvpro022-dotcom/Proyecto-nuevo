# ADR-0014 · La pantalla se pide, se bombea en su propio hilo y nunca se tira un fotograma

**Estado:** aceptada · **Fecha:** 2026-11-06

## Contexto

Ya existían, por separado y probadas: la captura de Windows, el regulador de
ritmo, el escalado y el codificador H.264. Faltaba lo que ninguna de ellas puede
decidir sola: cuándo capturar, cuándo empezar, qué hacer cuando algo falla y
quién paga el coste.

Ese coste no es menor. Capturar, escalar y codificar es, con mucha diferencia,
lo más caro que hace el host, y corre en el ordenador de alguien que
probablemente lo esté usando. Medido en este proyecto: una pantalla de
1920×1080 al 85 % cuesta unos 58 ms por fotograma, o sea un núcleo entero
ocupado para dar 17 fotogramas por segundo.

## Decisión

### La pantalla se pide, no se empuja

El host no empieza a enviar la pantalla al autenticar la sesión. Espera a que
el dispositivo la pida con `ControlRequest::StartScreen`.

Quien va a mostrar la imagen es el único que sabe cuándo está listo para
mostrarla; hasta entonces, cada fotograma codificado es CPU del PC de alguien
gastada en algo que no se ve. Pedirla otra vez con la pantalla ya en marcha
significa «he perdido el hilo, mándame un fotograma completo», que es lo que
hace falta tras una pérdida larga de paquetes.

Esto **no** añade una segunda puerta de seguridad: la petición llega por el
canal de control, que no se atiende hasta que la atadura de sesión está
comprobada ([ADR-0009]). Es la misma puerta.

### El capturador nace en el hilo que captura

El capturador de Windows guarda un dispositivo Direct3D y objetos COM, que no
se pueden mover de un hilo a otro. Por eso lo que viaja entre hilos no es el
capturador, sino una fuente que lo crea (`CapturerSource`), igual que ya se
hacía con el inyector de entrada. Es además la costura por la que las pruebas
meten un capturador sintético sin tocar nada del camino real.

Es un hilo propio del sistema, no una tarea del grupo de bloqueo: no es un
trabajo que acabe, es uno que dura toda la sesión.

### Un fotograma codificado no se tira nunca

En H.264 cada fotograma normal se describe como la diferencia con los
anteriores, así que perder uno estropea todos los que vengan detrás hasta el
siguiente completo. Cuando la entrega va más lenta que la captura, el hilo
**espera**; esa espera frena el ritmo, que es exactamente lo que hay que hacer.
La cola entre captura y envío es de dos fotogramas: una cola larga sería
retraso acumulado, y en una pantalla remota el retraso se nota más que un
tirón.

### Enviar la pantalla no puede congelar el ratón

El envío por la pista de vídeo vive en su propia tarea, no en el bucle de la
sesión. Si estuviera en el bucle, una espera de red dejaría de atender la
señalización, el teclado y las transferencias.

### El codificador no se queda con la máquina

Sin decírselo, openh264 coge todos los núcleos que encuentra. Se le dan la
mitad, nunca más de cuatro: por arriba reparte el fotograma en más trozos y
comprime algo peor sin ir mucho más rápido. Un escritorio remoto que acapara el
PC entero estropea justo lo que viene a controlar.

### «No ha cambiado» no es un fallo, y fallar siempre sí

El capturador avisa de que la pantalla no ha cambiado: no enviar nada es la
mejor compresión que existe. Un fallo suelto tampoco es grave —ocurre al
cambiar la resolución, al bloquear la sesión o al entrar un juego a pantalla
completa— y se reintenta. Treinta seguidos, a treinta por segundo, es un
segundo sin una sola captura: eso ya no se arregla esperando.

Cuando se llega a ese punto **la sesión no se corta**. El teclado, el ratón, el
portapapeles y las transferencias siguen funcionando. Lo que no puede pasar es
que el otro extremo se quede mirando una imagen congelada sin saber por qué,
así que se avisa con `HostEvent::ScreenStopped`.

## Consecuencias

- Un cliente que no pida la pantalla no la verá. Es explícito y está
  documentado en el protocolo; el precio de la alternativa era codificar para
  nadie.
- El host gasta CPU solo mientras alguien mira.
- Un fallo de captura degrada la sesión en lugar de matarla.

## Lo que está probado y lo que no

Probado ejecutándolo: las decisiones del bombeo (19 pruebas, con un capturador
que falla a demanda), y la cadena completa en una sesión de verdad —el bucle
del producto negocia WebRTC, comprueba la atadura, atiende la petición,
escala, codifica, trocea en RTP y al otro lado se recompone y se decodifica una
imagen, dos veces: antes y después de cambiar la calidad en caliente—. También
que sin firmar la atadura **no sale un solo paquete**, comprobado quitando la
comprobación y viendo llegar 174.

Lo único que no es real en esas pruebas es el origen de los píxeles: la captura
solo corre en Windows. Lo que falta por ejecutar en un PC con Windows está en
[ADR-0007](ADR-0007-verificacion-multiplataforma.md).

[ADR-0009]: ADR-0009-atadura-de-sesion.md
