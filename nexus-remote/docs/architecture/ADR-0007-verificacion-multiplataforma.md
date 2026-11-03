# ADR-0007 · Cómo se verifica el código que no se puede ejecutar aquí

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

El desarrollo ocurre en un contenedor Linux. El producto corre en Windows y en
iOS. Hay código que no se puede ejecutar donde se escribe, y el riesgo evidente
es escribir código de Windows que nunca ha pasado por un compilador y
declararlo terminado.

## Decisión

Cuatro niveles de verificación, de más fuerte a más débil, y la regla de no
afirmar nunca que algo funciona por encima del nivel que se haya alcanzado.

### Nivel 1 · Ejecutado y probado aquí

Todo el código sin plataforma: protocolo, criptografía, permisos, máquinas de
estado, restricción de acceso al disco, traducción de coordenadas. Es la mayor
parte del sistema y donde está casi toda la lógica delicada.

Esto es deliberado: **la validación que protege al host vive en código
portable**, y el backend de plataforma queda reducido a traducir órdenes ya
verificadas a llamadas del sistema.

### Nivel 2 · Compilado en cruce para Windows

Para el código `#[cfg(windows)]` se usa el objetivo `x86_64-pc-windows-gnu`
con `mingw-w64`, que ya está instalado:

```bash
rustup target add x86_64-pc-windows-gnu
cargo clippy --target x86_64-pc-windows-gnu -p nexus-capture -p nexus-input \
  --all-targets -- -D warnings
```

Comprueba tipos, nombres de API, firmas del crate `windows` y los lints, con
los avisos tratados como errores. **No comprueba que el comportamiento sea el
correcto.**

Qué ha detectado de verdad este nivel, en la etapa 2: que `IsSupported` está
en `GraphicsCaptureSession` y no en `GraphicsCaptureItem`, que
`MONITORINFOF_PRIMARY` vive en `WindowsAndMessaging` y no en `Gdi`, y que
`CPUAccessFlags` es `u32` mientras la constante es `i32`. Son exactamente los
errores que un compilador encuentra y una lectura atenta no.

### Nivel 3 · Verificado en un motor equivalente

El panel se compila y se carga en Chromium con datos reales del backend
(`scripts/verificar-panel.mjs`). Windows usa WebView2, que **también es
Chromium**, así que esta comprobación es representativa del objetivo real,
más incluso que ejecutarla en el WebKitGTK de Linux.

### Nivel 4 · Solo escrito

Código que no ha pasado por ninguno de los anteriores. Se marca explícitamente
en la documentación del módulo y en la matriz de funciones del panel. **No se
describe como terminado.**

## Cómo se refleja en el producto

Un booleano «funciona / no funciona» no basta para describir esto, así que
`commands::FeatureState` tiene tres valores que corresponden a los niveles:

| Estado | Significa | Nivel |
| --- | --- | --- |
| `working` | Ejecutado y probado donde se escribe | 1 |
| `unverified` | Escrito y compilado, **nunca ejecutado en su entorno real** | 2 |
| `pending` | Todavía no implementado | — |

El panel los muestra con colores distintos y el aviso correspondiente. Cuatro
pruebas lo sostienen:

- `nada_de_la_etapa_2_se_declara_probado` impide marcar como probado código de
  Windows que nadie ha ejecutado en Windows.
- `lo_que_depende_de_un_despliegue_no_se_declara_probado` hace lo mismo con la
  conexión entre redes distintas.
- `lo_que_el_servidor_rechaza_figura_como_pendiente` evita anunciar como
  disponible algo que el servidor rechaza a propósito.
- `el_host_no_anuncia_capacidades_que_no_tiene` impide que el host anuncie audio
  o archivos antes de implementarlos, porque el controlador los ofrecería en su
  interfaz y luego no funcionarían.

### Un matiz que apareció en la etapa 3

El nivel 1 dice «ejecutado y probado», y el código de red **sí** se ejecuta
aquí: el servidor de señalización arranca de verdad, el WebSocket es real y la
conexión WebRTC se establece de verdad. Es tentador concluir que la conexión
remota funciona.

No es lo mismo. En bucle local ICE encuentra camino directo siempre, y nunca se
usa el relay. Lo que está sin comprobar no es el código, es la **topología**:
dos redes distintas, dos NAT, un Coturn en marcha y un certificado válido.

De ahí que la etapa 3 tenga entradas separadas en la matriz: la señalización y
la conexión cifrada como *Probado*, y la conexión entre redes distintas como
*Sin probar*. El nivel de verificación lo marca el entorno en que algo se ha
ejecutado, no la cantidad de código que se ha escrito.

## Consecuencias

- La matriz de funciones es parte del producto, no documentación aparte: el
  panel la usa para no mostrar como terminado lo que no lo está.
- Además de la matriz, que describe el estado del **proyecto**, el comando
  `engine_status` comprueba el **equipo concreto**: intenta enumerar pantallas
  y crear el inyector, y devuelve lo que ocurrió de verdad.
- Los backends sin implementación devuelven `UnsupportedPlatform`, no un valor
  vacío. Un error explícito es distinguible de un fallo; una lista vacía no.
- **Antes de dar por buena la etapa 2 hace falta un PC con Windows real.** El
  compilado en cruce no sustituye a comprobar que los fotogramas llegan, que
  el cursor cae donde debe y que las combinaciones de teclas funcionan.
- **Antes de dar por buena la conexión remota hace falta desplegar el
  servidor.** Que funcione entre dos procesos de la misma máquina no dice nada
  sobre si funcionará desde una red móvil.
