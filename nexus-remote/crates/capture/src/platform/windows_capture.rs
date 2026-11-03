//! Captura de pantalla en Windows con Windows Graphics Capture.
//!
//! # Estado de verificacion
//!
//! **Compilado para Windows, no ejecutado en Windows.** Es verificacion de
//! nivel 2 del [ADR-0007]: tipos, nombres de API y firmas comprobados por el
//! compilador en cruce, comportamiento **sin comprobar**. Este modulo no debe
//! darse por bueno hasta ejecutarlo en un PC con Windows.
//!
//! [ADR-0007]: ../../../../docs/architecture/ADR-0007-verificacion-multiplataforma.md
//!
//! # Por que Windows Graphics Capture y no las alternativas
//!
//! - **`BitBlt` sobre el contexto de pantalla** es la via clasica, pero copia por
//!   CPU, no captura ventanas aceleradas por hardware y no funciona con
//!   composicion moderna.
//! - **Duplicacion de escritorio (DXGI)** es rapida, pero se cae en cada cambio
//!   de modo, al bloquear la sesion o al entrar el Control de cuentas de
//!   usuario, y hay que rehacer toda la cadena.
//! - **Windows Graphics Capture** entrega los fotogramas ya en memoria de la
//!   tarjeta grafica, sobrevive a los cambios de resolucion y es la via que
//!   Microsoft mantiene. Necesita Windows 10 1803 o posterior.
//!
//! # Camino que recorre un fotograma
//!
//! ```text
//! GraphicsCaptureItem  (el monitor)
//!   -> Direct3D11CaptureFramePool  (cola de fotogramas en la GPU)
//!     -> Direct3D11CaptureFrame -> IDirect3DSurface
//!       -> ID3D11Texture2D  (sigue en la GPU, no se puede leer)
//!         -> textura de preparacion  (copia legible por la CPU)
//!           -> Map -> bytes BGRA -> Frame
//! ```
//!
//! El paso por la textura de preparacion es obligatorio: la textura que entrega
//! la captura vive en memoria de la tarjeta y la CPU no puede leerla
//! directamente.

#![allow(unsafe_code)] // Llamadas a Direct3D y WinRT, justificadas una a una.

use nexus_core::{MonitorLayout, Timestamp};
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::E_POINTER;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::Interface;

use crate::capturer::ScreenCapturer;
use crate::error::{CaptureError, CaptureResult};
use crate::frame::{Frame, PixelFormat};

/// Numero de fotogramas que la cola mantiene en vuelo.
///
/// Dos basta para un consumidor que lee tan rapido como puede: mientras se lee
/// uno, el sistema escribe el siguiente. Mas buffers solo anaden latencia.
const FRAME_POOL_BUFFERS: i32 = 2;

/// Sesion de captura activa sobre un monitor concreto.
struct ActiveCapture {
    /// El elemento capturado.
    ///
    /// Hay que conservarlo aunque no se consulte en cada fotograma: si se
    /// suelta, Windows da la captura por terminada y la cola deja de recibir
    /// imagenes. [`WindowsCapturer::captured_item_name`] lo usa ademas para
    /// diagnostico.
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
    staging: Option<ID3D11Texture2D>,
    monitor_index: u8,
}

impl Drop for ActiveCapture {
    fn drop(&mut self) {
        // Cerrar explicitamente libera los recursos de la tarjeta grafica sin
        // esperar al recolector de `WinRT`. Si falla no hay nada que hacer: ya
        // se esta destruyendo.
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// Capturador de Windows.
pub struct WindowsCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    handles: Vec<HMONITOR>,
    layout: MonitorLayout,
    active: Option<ActiveCapture>,
    selected: u8,
}

impl std::fmt::Debug for WindowsCapturer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Los objetos de Direct3D y WinRT no aportan nada legible en un
        // registro: se resume el estado util en su lugar.
        f.debug_struct("WindowsCapturer")
            .field("monitores", &self.layout.len())
            .field("seleccionado", &self.selected)
            .field("capturando", &self.active.is_some())
            .finish_non_exhaustive()
    }
}

impl WindowsCapturer {
    /// Crea el capturador y su dispositivo Direct3D.
    ///
    /// # Errors
    ///
    /// - [`CaptureError::PermissionDenied`] si el sistema no soporta Windows
    ///   Graphics Capture (anterior a Windows 10 1803).
    /// - [`CaptureError::System`] si no se puede crear el dispositivo Direct3D
    ///   o enumerar los monitores.
    pub fn new() -> CaptureResult<Self> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err(CaptureError::PermissionDenied(
                "este Windows no soporta Windows Graphics Capture; \
                 hace falta Windows 10 version 1803 o posterior"
                    .to_owned(),
            ));
        }

        let (device, context) = create_d3d_device()?;
        let winrt_device = wrap_for_winrt(&device)?;
        let (handles, layout) = super::windows_monitors::enumerate_with_handles()?;

        let selected = layout.primary().map_or(0, |m| m.index);

        Ok(Self {
            device,
            context,
            winrt_device,
            handles,
            layout,
            active: None,
            selected,
        })
    }

    /// Nombre que Windows da a lo que se esta capturando.
    ///
    /// Sirve para comprobar en los registros que la sesion quedo atada al
    /// monitor correcto, que es el fallo mas facil de cometer con varias
    /// pantallas.
    #[must_use]
    pub fn captured_item_name(&self) -> Option<String> {
        let active = self.active.as_ref()?;
        active
            .item
            .DisplayName()
            .ok()
            .map(|name| name.to_string_lossy())
    }

    /// Vuelve a enumerar los monitores y cierra la captura en curso.
    ///
    /// Hay que llamarlo cuando cambia la configuracion de pantallas: los
    /// manejadores anteriores dejan de ser validos.
    ///
    /// # Errors
    ///
    /// [`CaptureError::System`] si la enumeracion falla.
    pub fn refresh_monitors(&mut self) -> CaptureResult<()> {
        self.active = None;
        let (handles, layout) = super::windows_monitors::enumerate_with_handles()?;
        self.handles = handles;
        self.layout = layout;

        // Si el monitor que se estaba capturando ya no existe, se vuelve al
        // principal en lugar de quedarse apuntando a la nada.
        if self.layout.get(self.selected).is_none() {
            self.selected = self.layout.primary().map_or(0, |m| m.index);
        }
        Ok(())
    }

    /// Arranca la captura del monitor seleccionado, si no estaba ya en marcha.
    fn ensure_active(&mut self) -> CaptureResult<()> {
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.monitor_index == self.selected)
        {
            return Ok(());
        }
        self.active = None;

        let handle =
            *self
                .handles
                .get(usize::from(self.selected))
                .ok_or(CaptureError::UnknownMonitor {
                    index: self.selected,
                })?;

        let item = create_capture_item(handle)?;
        let size = item
            .Size()
            .map_err(|e| system("no se pudo leer el tamano del monitor", &e))?;

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.winrt_device,
            // BGRA de 8 bits por canal es el formato nativo del compositor de
            // Windows: pedir otro obligaria a una conversion por cada fotograma.
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            FRAME_POOL_BUFFERS,
            size,
        )
        .map_err(|e| system("no se pudo crear la cola de fotogramas", &e))?;

        let session = pool
            .CreateCaptureSession(&item)
            .map_err(|e| system("no se pudo crear la sesion de captura", &e))?;

        // El cursor se dibuja en el cliente, no se graba en el video: asi se
        // mueve con fluidez aunque el video vaya a menos fotogramas.
        let _ = session.SetIsCursorCaptureEnabled(false);
        // Sin el borde amarillo de "esta aplicacion esta capturando". Windows
        // lo permite desde la version 2104; en versiones anteriores falla y se
        // ignora, que es lo correcto: el borde es molesto pero no impide nada.
        let _ = session.SetIsBorderRequired(false);

        session
            .StartCapture()
            .map_err(|e| system("no se pudo iniciar la captura", &e))?;

        self.active = Some(ActiveCapture {
            item,
            pool,
            session,
            size,
            staging: None,
            monitor_index: self.selected,
        });
        Ok(())
    }

    /// Copia una textura de la tarjeta grafica a memoria legible por la CPU.
    fn read_texture(&mut self, texture: &ID3D11Texture2D, now: Timestamp) -> CaptureResult<Frame> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SEGURIDAD: `GetDesc` solo escribe en la estructura que se le pasa,
        // que es local y esta viva durante la llamada.
        unsafe { texture.GetDesc(std::ptr::from_mut(&mut desc)) };

        if desc.Width == 0 || desc.Height == 0 {
            return Err(CaptureError::System(format!(
                "la captura entrego una textura de {}x{}",
                desc.Width, desc.Height
            )));
        }

        // La cola se pidio en BGRA de 8 bits, y el resto del metodo cuenta con
        // ello: `copy_rows` asume 4 bytes por pixel y el fotograma se etiqueta
        // como `Bgra8`. Si algun dia llegara otro formato, es mejor fallar que
        // entregar bytes mal etiquetados, que se verian como colores erroneos
        // sin ninguna pista de por que.
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err(CaptureError::System(format!(
                "la captura entrego el formato {:?}, y se esperaba B8G8R8A8_UNORM",
                desc.Format
            )));
        }

        // Una textura multimuestreada no se puede mapear. La captura nunca las
        // entrega, pero comprobarlo cuesta nada y el fallo seria opaco.
        if desc.SampleDesc.Count != 1 {
            return Err(CaptureError::System(format!(
                "la captura entrego una textura con {} muestras por pixel",
                desc.SampleDesc.Count
            )));
        }

        let staging = self.staging_texture(&desc)?;

        // SEGURIDAD: ambas texturas son recursos validos del mismo dispositivo
        // y tienen el mismo tamano y formato, que es lo que `CopyResource`
        // exige. La textura de preparacion se crea a partir de `desc`.
        unsafe { self.context.CopyResource(&staging, texture) };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SEGURIDAD: la textura de preparacion se creo con acceso de lectura
        // desde la CPU, que es lo que `D3D11_MAP_READ` requiere.
        unsafe {
            self.context.Map(
                &staging,
                0,
                D3D11_MAP_READ,
                0,
                Some(std::ptr::from_mut(&mut mapped)),
            )
        }
        .map_err(|e| system("no se pudo mapear la textura", &e))?;

        let result = copy_rows(&mapped, desc.Width, desc.Height);

        // SEGURIDAD: se desmapea siempre, tambien si la copia fallo. Dejar una
        // textura mapeada bloquearia la tarjeta grafica.
        unsafe { self.context.Unmap(&staging, 0) };

        let (data, stride) = result?;

        let frame = Frame {
            width: desc.Width,
            height: desc.Height,
            format: PixelFormat::Bgra8,
            stride,
            data,
            captured_at: now,
            monitor_index: self.selected,
        };
        frame.validate()?;
        Ok(frame)
    }

    /// Textura de preparacion con el tamano adecuado, creandola si hace falta.
    ///
    /// Se reutiliza entre fotogramas: crear una por fotograma seria un
    /// desperdicio notable a 60 imagenes por segundo.
    fn staging_texture(&mut self, desc: &D3D11_TEXTURE2D_DESC) -> CaptureResult<ID3D11Texture2D> {
        let needs_new = self
            .active
            .as_ref()
            .and_then(|a| a.staging.as_ref())
            .is_none_or(|t| {
                let mut existing = D3D11_TEXTURE2D_DESC::default();
                // SEGURIDAD: igual que arriba, `GetDesc` solo escribe en la local.
                unsafe { t.GetDesc(std::ptr::from_mut(&mut existing)) };
                existing.Width != desc.Width
                    || existing.Height != desc.Height
                    || existing.Format != desc.Format
            });

        if needs_new {
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: desc.Width,
                Height: desc.Height,
                MipLevels: 1,
                ArraySize: 1,
                Format: desc.Format,
                SampleDesc: desc.SampleDesc,
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0.unsigned_abs(),
                MiscFlags: 0,
            };

            let mut texture: Option<ID3D11Texture2D> = None;
            // SEGURIDAD: `staging_desc` es una estructura local viva durante la
            // llamada, y `texture` recibe la referencia que crea Direct3D.
            unsafe {
                self.device.CreateTexture2D(
                    std::ptr::from_ref(&staging_desc),
                    None,
                    Some(std::ptr::from_mut(&mut texture)),
                )
            }
            .map_err(|e| system("no se pudo crear la textura de preparacion", &e))?;

            let texture = texture.ok_or_else(|| {
                CaptureError::System("Direct3D no devolvio la textura".to_owned())
            })?;

            if let Some(active) = self.active.as_mut() {
                active.staging = Some(texture.clone());
            }
            return Ok(texture);
        }

        self.active
            .as_ref()
            .and_then(|a| a.staging.clone())
            .ok_or_else(|| CaptureError::System("no hay textura de preparacion".to_owned()))
    }
}

impl ScreenCapturer for WindowsCapturer {
    fn name(&self) -> &'static str {
        "windows-graphics-capture"
    }

    fn enumerate_monitors(&self) -> CaptureResult<MonitorLayout> {
        Ok(self.layout.clone())
    }

    fn select_monitor(&mut self, index: u8) -> CaptureResult<()> {
        if self.layout.get(index).is_none() {
            return Err(CaptureError::UnknownMonitor { index });
        }
        if index != self.selected {
            self.selected = index;
            // La sesion anterior se cierra: cada sesion captura un monitor.
            self.active = None;
        }
        Ok(())
    }

    fn capture_frame(&mut self, now: Timestamp) -> CaptureResult<Frame> {
        self.ensure_active()?;

        let frame = {
            let active = self
                .active
                .as_ref()
                .ok_or_else(|| CaptureError::System("sin sesion de captura".to_owned()))?;

            match active.pool.TryGetNextFrame() {
                Ok(frame) => frame,
                // La cola devuelve un puntero nulo cuando todavia no hay
                // fotograma nuevo, y la conversion al tipo de Rust lo traduce
                // a `E_POINTER`. No es un fallo: significa "vuelva luego".
                //
                // Se compara con la constante de la biblioteca y no con el
                // valor escrito a mano: equivocarse de digito haria que un
                // error real se confundiera con "aun no hay fotograma".
                Err(e) if e.code() == E_POINTER => return Err(CaptureError::WouldBlock),
                Err(e) => return Err(session_lost("se perdio la cola de fotogramas", &e)),
            }
        };

        // Si cambio la resolucion del monitor, la cola hay que rehacerla con el
        // tamano nuevo. El fotograma actual se descarta.
        let content_size = frame
            .ContentSize()
            .map_err(|e| system("no se pudo leer el tamano del fotograma", &e))?;
        if let Some(active) = self.active.as_mut()
            && (content_size.Width != active.size.Width
                || content_size.Height != active.size.Height)
        {
            active
                .pool
                .Recreate(
                    &self.winrt_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    FRAME_POOL_BUFFERS,
                    content_size,
                )
                .map_err(|e| system("no se pudo rehacer la cola tras el cambio de tamano", &e))?;
            active.size = content_size;
            active.staging = None;
            let _ = frame.Close();
            return Err(CaptureError::WouldBlock);
        }

        let surface = frame
            .Surface()
            .map_err(|e| system("el fotograma no trae superficie", &e))?;

        // La superficie de `WinRT` envuelve una textura de Direct3D. Esta
        // interfaz de interoperabilidad es la via oficial para sacarla.
        let access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .map_err(|e| system("la superficie no expone la interfaz de Direct3D", &e))?;

        // SEGURIDAD: `GetInterface` devuelve una referencia contada al objeto
        // envuelto. El tipo pedido es el que la superficie contiene de verdad.
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }
            .map_err(|e| system("la superficie no contiene una textura", &e))?;

        let result = self.read_texture(&texture, now);

        // El fotograma se devuelve a la cola pase lo que pase: si no, la cola
        // se queda sin buffers y la captura se detiene.
        let _ = frame.Close();

        result
    }
}

/// Crea el dispositivo Direct3D que usara la captura.
///
/// Intenta primero la tarjeta grafica y cae en el rasterizador por software si
/// no hay ninguna disponible, que es lo que ocurre en una maquina virtual sin
/// aceleracion. Es mas lento, pero funciona.
fn create_d3d_device() -> CaptureResult<(ID3D11Device, ID3D11DeviceContext)> {
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;

        // SEGURIDAD: las dos variables de salida son locales vivas durante la
        // llamada. `D3D11CreateDevice` las rellena o devuelve error.
        let result = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                // La captura entrega texturas BGRA; sin esta marca el
                // dispositivo no podria tratarlas.
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(std::ptr::from_mut(&mut device)),
                None,
                Some(std::ptr::from_mut(&mut context)),
            )
        };

        if result.is_ok()
            && let (Some(device), Some(context)) = (device, context)
        {
            return Ok((device, context));
        }
    }

    Err(CaptureError::System(
        "no se pudo crear ningun dispositivo Direct3D, ni por hardware ni por software".to_owned(),
    ))
}

/// Envuelve el dispositivo de Direct3D en el tipo que espera `WinRT`.
fn wrap_for_winrt(device: &ID3D11Device) -> CaptureResult<IDirect3DDevice> {
    let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice = device
        .cast()
        .map_err(|e| system("el dispositivo no expone la interfaz DXGI", &e))?;

    // SEGURIDAD: `dxgi` es una referencia valida al mismo dispositivo. La
    // funcion devuelve un objeto de `WinRT` con su propia cuenta de referencias.
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .map_err(|e| system("no se pudo envolver el dispositivo para WinRT", &e))?;

    inspectable
        .cast()
        .map_err(|e| system("el dispositivo envuelto no es un IDirect3DDevice", &e))
}

/// Crea el elemento de captura que representa un monitor.
fn create_capture_item(monitor: HMONITOR) -> CaptureResult<GraphicsCaptureItem> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| system("no se pudo obtener la fabrica de captura", &e))?;

    // SEGURIDAD: `monitor` es un manejador que acaba de devolver la
    // enumeracion del sistema, y el tipo pedido es el que la interfaz crea.
    unsafe { interop.CreateForMonitor(monitor) }
        .map_err(|e| system("no se pudo crear el elemento de captura del monitor", &e))
}

/// Copia las filas de una textura mapeada a un buffer contiguo.
///
/// La textura mapeada tiene un paso de fila (`RowPitch`) que puede ser mayor
/// que el ancho util, por alineacion. Se copia fila a fila respetandolo.
fn copy_rows(
    mapped: &D3D11_MAPPED_SUBRESOURCE,
    width: u32,
    height: u32,
) -> CaptureResult<(Vec<u8>, usize)> {
    if mapped.pData.is_null() {
        return Err(CaptureError::System(
            "la textura mapeada no tiene datos".to_owned(),
        ));
    }

    let row_pitch = mapped.RowPitch as usize;
    let row_bytes = width as usize * 4;
    if row_pitch < row_bytes {
        return Err(CaptureError::System(format!(
            "paso de fila {row_pitch} menor que el minimo {row_bytes}"
        )));
    }

    let height = height as usize;
    let mut data = Vec::with_capacity(row_bytes * height);

    for row in 0..height {
        // SEGURIDAD: `pData` apunta a una region de al menos
        // `row_pitch * height` bytes, garantizada por `Map`, y `row` nunca
        // llega a `height`. Se leen solo `row_bytes` de cada fila, que es como
        // mucho `row_pitch`.
        let slice = unsafe {
            std::slice::from_raw_parts(mapped.pData.cast::<u8>().add(row * row_pitch), row_bytes)
        };
        data.extend_from_slice(slice);
    }

    // El buffer resultante es contiguo: su paso de fila es el ancho util.
    Ok((data, row_bytes))
}

fn system(context: &str, error: &windows::core::Error) -> CaptureError {
    CaptureError::System(format!("{context}: {error}"))
}

fn session_lost(context: &str, error: &windows::core::Error) -> CaptureError {
    CaptureError::SessionLost(format!("{context}: {error}"))
}
