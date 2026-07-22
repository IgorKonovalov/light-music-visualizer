//! wgpu device/surface ownership. All raw GPU access lives behind this layer
//! (ADR-0001): scene code sees wgpu types, never a backend.

// Hot-path panic-denial pragma (Plan 0002 Phase 2). GPU bring-up returns
// Result; the render path must not panic.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use wgpu::{CreateSurfaceError, RequestAdapterError, RequestDeviceError, SurfaceTarget};

/// Offscreen texture format for the headless capture path (Plan 0013). A tight
/// 8-bit RGBA the readback strips straight into a [`crate::render::CaptureImage`].
pub(crate) const HEADLESS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

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
    /// A headless capture failed to map or read back its offscreen buffer
    /// (Plan 0013 tooling path — never the live render path).
    CaptureReadback,
    /// A capture requested a preset name not in the loaded roster (Plan 0013).
    UnknownPreset(String),
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
            RenderError::CaptureReadback => {
                write!(f, "headless capture readback failed")
            }
            RenderError::UnknownPreset(name) => {
                write!(f, "no preset named '{name}' in the roster")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// Owns the wgpu instance, surface, device, and queue for one output window.
///
/// `surface` is `None` for a **headless** context (Plan 0013): a device+queue
/// with no swapchain, drawing into offscreen capture textures. The on-surface
/// present path always has `Some`; `config` still carries the render size and
/// format for both paths.
pub struct RenderContext {
    pub(crate) surface: Option<wgpu::Surface<'static>>,
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
        // Explicit swapchain depth (NFR 12 secondary lever): pin a 2-frame
        // latency (double-buffered) rather than leaving it to the backend
        // default, so the in-flight image count - and its VRAM - is bounded and
        // stated, not implicit.
        config.desired_maximum_frame_latency = 2;
        surface.configure(&device, &config);

        Ok(Self {
            surface: Some(surface),
            device,
            queue,
            config,
        })
    }

    /// Build a surface-less context for headless capture (Plan 0013): a device
    /// and queue with no swapchain, drawing into offscreen textures. No window,
    /// no present, no added dependency. `prefer_software` forces a fallback
    /// adapter (WARP on DX12) so tests rasterize identically on any machine.
    ///
    /// The synthesized [`wgpu::SurfaceConfiguration`] carries only the render
    /// size and the offscreen format ([`HEADLESS_FORMAT`]); its present-related
    /// fields are inert with no surface to configure.
    pub fn new_headless(
        width: u32,
        height: u32,
        prefer_software: bool,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: prefer_software,
            ..Default::default()
        }))
        .map_err(RenderError::RequestAdapter)?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lmv-headless-device"),
            ..Default::default()
        }))
        .map_err(RenderError::RequestDevice)?;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: HEADLESS_FORMAT,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        };

        Ok(Self {
            surface: None,
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
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
    }

    /// The texture format the surface is configured with.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Re-apply the current configuration (after a Lost/Outdated surface).
    /// A no-op on a headless context (no surface to reconfigure).
    pub(crate) fn reconfigure(&self) {
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
    }
}
