//! # nexus-input
//!
//! Traduccion de eventos de entrada y contrato de inyeccion en el sistema.
//!
//! El crate se divide en dos mitades con responsabilidades muy distintas, y
//! con **grados de verificacion distintos**.
//!
//! ## Portable, ejecutado y probado aqui
//!
//! - [`resolve`]: valida el evento del protocolo y lo convierte en una orden
//!   absoluta.
//! - [`keymap`]: traduce los nombres de tecla del protocolo a codigos
//!   virtuales de Windows, con su marca de tecla extendida.
//! - [`absolute`]: lleva las coordenadas del escritorio virtual al rango que
//!   espera `SendInput`, con una prueba que recorre pixel a pixel varias
//!   resoluciones reales.
//!
//! ## De plataforma, compilado para Windows pero no ejecutado alli
//!
//! - [`platform::windows`]: las llamadas a `SendInput`. Verificacion de nivel
//!   2 del ADR-0007: el compilador comprobo tipos y firmas en cruce, **nadie
//!   ha comprobado el comportamiento en un PC con Windows**.
//!
//! La separacion es deliberada: toda la validacion que protege al host ocurre
//! en la mitad portable, que se prueba de forma exhaustiva en cualquier
//! maquina, y el backend de plataforma queda reducido a traducir ordenes ya
//! verificadas a llamadas del sistema. Asi lo que no se puede probar aqui es
//! lo menos posible, y lo mas simple posible.

// El codigo portable de este crate no usa `unsafe`. El backend de Windows si
// lo necesita para llamar a `SendInput`, y lo permite de forma acotada en su
// propio modulo (`platform::windows`), con una justificacion escrita en cada
// llamada. `forbid` no se puede levantar por modulo, de ahi `deny`.
#![deny(unsafe_code)]

pub mod absolute;
pub mod error;
pub mod inject;
pub mod keymap;
pub mod platform;
pub mod resolve;

pub use absolute::VirtualDesktop;
pub use error::{InputError, InputResult};
pub use inject::{InputInjector, RecordingInjector};
pub use keymap::{KeyMapping, VirtualKey};
pub use platform::default_injector;
pub use resolve::{ResolvedInput, resolve};
