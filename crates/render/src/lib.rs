mod text;

use anyhow::{Context, Result};

use text::TextGrid;
pub use text::{CellSize, FontConfig};

#[derive(Debug, Clone, Copy)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    text: TextGrid,
}

#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub background: Rgba,
    pub foreground: Rgba,
    pub text: &'a str,
}

impl Renderer {
    pub async fn new<W>(
        window: W,
        width: u32,
        height: u32,
        scale: f32,
        font: FontConfig,
    ) -> Result<Self>
    where
        W: wgpu::wgt::WgpuHasDisplayHandle + Clone + Into<wgpu::SurfaceTarget<'static>>,
    {
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let target = window;
        let surface = instance.create_surface(target).context("create surface")?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .context("request adapter")?;
        log::info!("adapter: {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("argus"),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .context("request device")?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let text = TextGrid::new(&device, &queue, format, font, scale);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            text,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn cell_size(&self) -> CellSize {
        self.text.cell()
    }

    pub fn grid_size(&self) -> (usize, usize) {
        self.text.grid_size(self.config.width, self.config.height)
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.text.set_scale(scale);
    }

    pub fn render(&mut self, frame_data: Frame<'_>) -> Result<()> {
        self.text.set_text(frame_data.text);
        self.text.prepare(
            &self.device,
            &self.queue,
            self.config.width,
            self.config.height,
            to_glyph_color(frame_data.foreground),
        )?;

        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => anyhow::bail!("surface validation error"),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: frame_data.background.r,
                            g: frame_data.background.g,
                            b: frame_data.background.b,
                            a: frame_data.background.a,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.text.render(&mut pass)?;
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        self.text.trim();
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(())
    }
}

fn to_glyph_color(c: Rgba) -> glyphon::Color {
    let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    glyphon::Color::rgba(ch(c.r), ch(c.g), ch(c.b), ch(c.a))
}
