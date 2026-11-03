//! Script de compilacion de Tauri.
//!
//! Genera el contexto de la aplicacion: incrusta el panel ya compilado de
//! `../dist`, las capacidades de `capabilities/` y los metadatos de
//! `tauri.conf.json`. Debe ejecutarse despues de `npm run build`.

fn main() {
    tauri_build::build();
}
