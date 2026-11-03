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
cargo check --target x86_64-pc-windows-gnu -p nexus-capture -p nexus-input
```

Comprueba tipos, nombres de API y firmas del crate `windows`. No comprueba que
el comportamiento sea el correcto.

### Nivel 3 · Verificado en un motor equivalente

El panel se compila y se carga en Chromium con datos reales del backend
(`scripts/verificar-panel.mjs`). Windows usa WebView2, que **también es
Chromium**, así que esta comprobación es representativa del objetivo real,
más incluso que ejecutarla en el WebKitGTK de Linux.

### Nivel 4 · Solo escrito

Código que no ha pasado por ninguno de los anteriores. Se marca explícitamente
en la documentación del módulo y en la matriz de funciones del panel. **No se
describe como terminado.**

## Consecuencias

- La matriz de `commands::feature_matrix` es parte del producto, no
  documentación aparte: el panel la usa para no mostrar botones que no hacen
  nada, y una prueba comprueba que nada marcado como listo pertenezca a una
  etapa futura.
- Los backends sin implementación devuelven `UnsupportedPlatform`, no un valor
  vacío. Un error explícito es distinguible de un fallo; una lista vacía no.
- Antes de dar por buena la etapa 2 hará falta **un PC con Windows real**. El
  compilado en cruce no sustituye a probar la captura de pantalla.
