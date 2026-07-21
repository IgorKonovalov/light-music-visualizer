//! wgpu device/surface ownership. All raw GPU access lives behind this layer
//! (ADR-0001): scene code sees wgpu types, never a backend.

use wgpu::{CreateSurfaceError, RequestAdapterError, RequestDeviceError, SurfaceTarget};

/// Something went wrong bringing up or drawing with the GPU context.
#[derive(Debug)]
pub enum RenderError {
    /// Creating the wgpu surface for the window failed.
    CreateSurface(CreateSurfaceError),
    /// No GPU adapter compatible with the surface was found.
    RequestAdapter(RequestAdapterError),
    /// Requesting a logical device from the adapter failed.
    RequestDevice(RequestDeviceError),
    /// The surface reported no supported configuration on this adapter.
    UnsupportedSurface,
    /// Acquiring the frame raised a validation error — a bug, not a
    /// recoverable surface state.
    SurfaceValidation,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::CreateSurface(e) => write!(f, "surface creation failed: {e}"),
            RenderError::RequestAdapter(e) => write!(f, "no suitable GPU adapter: {e}"),
            RenderError::RequestDevice(e) => write!(f, "device request failed: {e}"),
            RenderError::UnsupportedSurface => write!(f, "surface has no supported config"),
            RenderError::SurfaceValidation => {
                write!(f, "surface texture acquisition failed validation")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// Owns the wgpu instance, surface, device, and queue for one output window.
pub struct RenderContext {
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) config: wgpu::SurfaceConfiguration,
}

impl RenderContext {
    /// Create a context rendering into `target` (any window-handle provider —
    /// the core never sees the windowing library behind it).
    pub fn new(
        target: impl Into<SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(target)
            .map_err(RenderError::CreateSurface)?;
        Self::from_surface(&instance, surface, width, height)
    }

    /// Context from raw display/window handles — the C ABI path, where the
    /// host (e.g. the foobar2000 shim) owns the window.
    ///
    /// # Safety
    /// The handles must be valid and the window must outlive this context.
    pub unsafe fn new_unsafe(
        target: wgpu::SurfaceTargetUnsafe,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = unsafe { instance.create_surface_unsafe(target) }
            .map_err(RenderError::CreateSurface)?;
        Self::from_surface(&instance, surface, width, height)
    }

    fn from_surface(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(RenderError::RequestAdapter)?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lmv-device"),
            ..Default::default()
        }))
        .map_err(RenderError::RequestDevice)?;

        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or(RenderError::UnsupportedSurface)?;
        // Vsync everywhere; the render loop paces itself off the display.
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        Ok(Self {
            surface,
            device,
            queue,
            config,
        })
    }

    /// Reconfigure the surface for a new size (a zero dimension is ignored).
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return; // minimized; keep the old config until we're visible again
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// The texture format the surface is configured with.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Re-apply the current configuration (after a Lost/Outdated surface).
    pub(crate) fn reconfigure(&self) {
        self.surface.configure(&self.device, &self.config);
    }
}
