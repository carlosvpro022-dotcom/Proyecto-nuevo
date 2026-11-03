# ADR-0001 · Monorepositorio con workspace de Cargo

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

NEXUS REMOTE tiene al menos cuatro piezas que deben entenderse entre sí: el
host de Windows, el cliente de Apple, el servidor de señalización y el servidor
TURN. El punto de contacto entre todas es el protocolo: si el host cambia el
nombre de un campo y el cliente no se entera, la sesión falla en ejecución, no
en compilación.

## Decisión

Un único repositorio con un workspace de Cargo. Los contratos viven en
`crates/core`, que compilan a la vez el host y el servidor de señalización. Un
cambio incompatible en el protocolo rompe la compilación de todo lo que
depende de él, en el mismo commit.

Estructura:

```
crates/     código compartido, sin plataforma, sin entrada/salida
apps/       las aplicaciones de cada plataforma
server/     los servicios desplegables
infrastructure/  cómo se despliegan
tests/      pruebas que cruzan varios crates
```

## Alternativas descartadas

- **Un repositorio por componente.** Obliga a versionar y publicar el crate de
  protocolo para cada cambio, y a coordinar cuatro repositorios para una
  modificación que es conceptualmente una sola.
- **Un único crate grande.** No permite que el servidor de señalización se
  compile sin arrastrar Tauri, SQLite y las dependencias de Windows. La imagen
  Docker del servidor pasaría de decenas de megabytes a cientos.

## Consecuencias

- El `Dockerfile` del servidor excluye explícitamente `apps/windows-host` y
  `tests` del workspace para no compilar Tauri dentro del contenedor.
- El cliente Apple **no** está en el workspace de Cargo: es un proyecto Xcode.
  Para que comparta los contratos habrá que generar su equivalente en Swift o
  exponer `nexus-core` por FFI, lo que se decide en el ADR-0005.
- La política de lints (`clippy::pedantic`) se declara una sola vez en el
  `Cargo.toml` raíz y la heredan todos los crates.
