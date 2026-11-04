//! Errores de transferencia de archivos.

use thiserror::Error;

/// Fallos de acceso y de transferencia.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FileError {
    /// La ruta queda fuera de las raices permitidas.
    #[error("acceso denegado: la ruta queda fuera de las carpetas compartidas")]
    OutsideRoots,

    /// La ruta contiene componentes de recorrido (`..`).
    #[error("acceso denegado: la ruta contiene recorrido de directorios")]
    PathTraversal,

    /// El nombre esta reservado por el sistema o tiene una forma peligrosa.
    #[error("nombre de archivo no permitido: {reason}")]
    ForbiddenName {
        /// Motivo concreto del rechazo.
        reason: &'static str,
    },

    /// La raiz concede acceso, pero no del tipo solicitado.
    #[error("acceso denegado: la carpeta compartida es de solo lectura")]
    ReadOnlyRoot,

    /// La ruta no existe.
    #[error("la ruta no existe")]
    NotFound,

    /// El archivo supera el tamano maximo admitido.
    #[error("archivo demasiado grande: {size_bytes} bytes (maximo {max_bytes})")]
    TooLarge {
        /// Tamano anunciado.
        size_bytes: u64,
        /// Maximo admitido.
        max_bytes: u64,
    },

    /// Se alcanzo el limite de transferencias simultaneas.
    #[error("demasiadas transferencias simultaneas (maximo {max})")]
    TooManyTransfers {
        /// Maximo admitido.
        max: usize,
    },

    /// La transferencia indicada no existe.
    #[error("la transferencia indicada no existe")]
    UnknownTransfer,

    /// La transferencia ya habia terminado.
    #[error("la transferencia ya habia terminado")]
    TransferFinished,

    /// El marco binario recibido no respeta el formato.
    #[error("marco de archivo mal formado: {reason}")]
    MalformedFrame {
        /// Motivo concreto del rechazo.
        reason: &'static str,
    },

    /// Los trozos no llegaron en orden.
    ///
    /// El canal de datos se configura ordenado y fiable, asi que un salto no es
    /// una perdida de la red: es un cliente que no respeta el protocolo.
    #[error("trozo fuera de orden: se esperaba el {expected} y llego el {received}")]
    OutOfOrder {
        /// Numero de secuencia que tocaba.
        expected: u32,
        /// Numero de secuencia recibido.
        received: u32,
    },

    /// Faltan trozos: el contador del marco de fin no cuadra.
    #[error("faltan trozos: se anunciaron {expected} y llegaron {received}")]
    FrameCountMismatch {
        /// Trozos que anuncio quien envia.
        expected: u32,
        /// Trozos recibidos de verdad.
        received: u32,
    },

    /// Llegaron menos bytes de los anunciados.
    #[error("transferencia incompleta: se anunciaron {expected} bytes y llegaron {received}")]
    Incomplete {
        /// Bytes anunciados.
        expected: u64,
        /// Bytes recibidos.
        received: u64,
    },

    /// El contenido no coincide con el resumen que anuncio quien envia.
    ///
    /// Es el unico error que **no** se puede atribuir a un descuido: o los
    /// bytes se corrompieron, o alguien los cambio. En ninguno de los dos casos
    /// el archivo llega a su destino.
    #[error("el contenido recibido no coincide con su resumen SHA-256")]
    IntegrityMismatch,

    /// El destino de una subida ya existe.
    #[error("ya existe un archivo en el destino")]
    AlreadyExists,

    /// Se pidio un archivo donde hay un directorio, o al contrario.
    #[error("la ruta no es {expected}")]
    WrongKind {
        /// Lo que se esperaba encontrar.
        expected: &'static str,
    },

    /// Error de entrada/salida del sistema de archivos.
    #[error("error de entrada/salida: {0}")]
    Io(String),
}

/// Resultado corto del crate.
pub type FileResult<T> = Result<T, FileError>;
