# ADR-0002 · Rust y Tauri para el host de Windows

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

El host corre de forma permanente en el PC, tiene acceso a la pantalla, al
teclado y a los archivos, y recibe datos de la red antes de que el otro extremo
esté autenticado. Es, con diferencia, la pieza con más superficie de ataque.

Además necesita una interfaz de escritorio que no parezca un proyecto de fin de
semana.

## Decisión

**Rust** para toda la lógica del host, con `#![forbid(unsafe_code)]` en todos
los crates que no necesitan llamar a la API de Windows.

**Tauri 2** para la ventana: el panel se escribe en HTML, CSS y TypeScript, y
se ejecuta en el WebView2 que Windows ya trae instalado.

## Alternativas descartadas

- **C++ con Win32 o Qt.** Es la opción tradicional para captura de pantalla de
  alto rendimiento, y da acceso directo a todas las API. Se descarta porque los
  fallos de memoria en código que procesa paquetes de red son exactamente la
  clase de error que más caro sale en un producto de acceso remoto.
- **Electron.** Empaqueta un Chromium entero: unos 150 MB de instalador frente
  a unos 10 MB de Tauri, y un proceso de Node con acceso al sistema. Para una
  aplicación que va a estar siempre arrancada, el consumo de memoria importa.
- **WinUI 3 o WPF con C#.** Buena interfaz nativa, pero obliga a .NET en el
  equipo y deja la lógica de red y criptografía en un runtime con recolección
  de basura, con pausas poco predecibles en la ruta de vídeo.
- **Tauri con la interfaz en Rust (egui, Dioxus).** Evitaría TypeScript, pero
  conseguir un acabado visual de producto comercial cuesta mucho más.

## Consecuencias

- El panel es HTML, pero **no es una aplicación web**: no hay servidor, no hay
  navegador y el único origen que puede invocar comandos es el propio paquete,
  con una política de seguridad de contenido restrictiva declarada en
  `tauri.conf.json`.
- Las capacidades de Tauri se declaran una a una en
  `src-tauri/capabilities/default.json`. No se usa el conjunto completo.
- La captura de pantalla y la inyección de entrada sí necesitarán `unsafe` para
  llamar a Win32. Ese código quedará aislado en `nexus-capture` y `nexus-input`
  tras una interfaz segura, y esos dos crates levantarán el `forbid` solo en el
  módulo de plataforma.
- WebView2 viene preinstalado desde Windows 10 2004. En instalaciones más
  antiguas el instalador debe incluir el *bootstrapper*, lo que hay que
  verificar en la etapa 6.
