mod rects;
mod text;

use anyhow::{Context, Result};

use rects::{Rect, RectRenderer};
use text::TextGrid;
pub use text::{CellSize, FontConfig};

/// One terminal cell, colors in sRGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridCell {
    pub ch: char,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Second half of a double-width character; nothing to draw.
    pub spacer: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct GridFrame<'a> {
    pub cols: usize,
    pub background: [u8; 3],
    /// Row-major cells.
    pub cells: &'a [GridCell],
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    text: TextGrid,
    rects: RectRenderer,
    rect_scratch: Vec<Rect>,
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
        let surface = instance.create_surface(window).context("create surface")?;

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
        let rects = RectRenderer::new(&device, format);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            text,
            rects,
            rect_scratch: Vec::new(),
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

    pub fn render(&mut self, grid: GridFrame<'_>) -> Result<()> {
        let (width, height) = (self.config.width, self.config.height);
        build_rects(
            &grid,
            self.text.cell(),
            self.text.padding(),
            &mut self.rect_scratch,
        );
        self.rects
            .prepare(&self.device, &self.queue, width, height, &self.rect_scratch);
        self.text.prepare(
            &self.device,
            &self.queue,
            width,
            height,
            grid.cols,
            grid.cells,
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

        let [r, g, b, a] = linear(grid.background);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grid"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(r),
                            g: f64::from(g),
                            b: f64::from(b),
                            a: f64::from(a),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.rects.render(&mut pass);
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

/// Backgrounds that differ from the default (merged per row) and underlines.
fn build_rects(grid: &GridFrame<'_>, cell: CellSize, padding: f32, out: &mut Vec<Rect>) {
    out.clear();
    if grid.cols == 0 {
        return;
    }
    let underline_h = (cell.height / 16.0).max(1.0).round();
    for (row, cells) in grid.cells.chunks(grid.cols).enumerate() {
        let top = padding + row as f32 * cell.height;
        let mut col = 0;
        while col < cells.len() {
            let bg = cells[col].bg;
            let start = col;
            while col < cells.len() && cells[col].bg == bg {
                col += 1;
            }
            if bg != grid.background {
                out.push(Rect {
                    pos: [padding + start as f32 * cell.width, top],
                    size: [(col - start) as f32 * cell.width, cell.height],
                    color: linear(bg),
                });
            }
        }
        for (col, c) in cells.iter().enumerate() {
            if c.underline && !c.spacer {
                out.push(Rect {
                    pos: [
                        padding + col as f32 * cell.width,
                        top + cell.height - underline_h,
                    ],
                    size: [cell.width, underline_h],
                    color: linear(c.fg),
                });
            }
        }
    }
}

fn linear(srgb: [u8; 3]) -> [f32; 4] {
    let ch = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [ch(srgb[0]), ch(srgb[1]), ch(srgb[2]), 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: [u8; 3] = [10, 10, 10];

    fn cell(bg: [u8; 3]) -> GridCell {
        GridCell {
            ch: ' ',
            fg: [200, 200, 200],
            bg,
            bold: false,
            italic: false,
            underline: false,
            spacer: false,
        }
    }

    #[test]
    fn adjacent_backgrounds_merge_and_default_is_skipped() {
        let red = [200, 0, 0];
        let cells = [cell(red), cell(red), cell(BG), cell(red)];
        let grid = GridFrame {
            cols: 4,
            background: BG,
            cells: &cells,
        };
        let size = CellSize {
            width: 10.0,
            height: 20.0,
        };
        let mut rects = Vec::new();
        build_rects(&grid, size, 0.0, &mut rects);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].size, [20.0, 20.0]);
        assert_eq!(rects[1].pos, [30.0, 0.0]);
    }

    #[test]
    fn srgb_to_linear_endpoints() {
        assert_eq!(linear([0, 0, 0]), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(linear([255, 255, 255]), [1.0, 1.0, 1.0, 1.0]);
    }
}
