use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};

#[derive(Debug, Clone)]
pub struct FontConfig {
    pub family: Option<String>,
    pub size: f32,
    pub line_height: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: None,
            size: 14.0,
            line_height: 1.25,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellSize {
    pub width: f32,
    pub height: f32,
}

pub(crate) struct TextGrid {
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    buffer: Buffer,
    font: FontConfig,
    scale: f32,
    cell: CellSize,
    padding: f32,
}

impl TextGrid {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font: FontConfig,
        scale: f32,
    ) -> Self {
        let mut font_system = FontSystem::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let metrics = metrics(&font, scale);
        let buffer = Buffer::new(&mut font_system, metrics);
        let cell = measure_cell(&mut font_system, &font, scale);
        log::info!("cell size: {cell:?}");

        Self {
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            buffer,
            padding: 8.0 * scale,
            font,
            scale,
            cell,
        }
    }

    pub(crate) fn cell(&self) -> CellSize {
        self.cell
    }

    pub(crate) fn grid_size(&self, width: u32, height: u32) -> (usize, usize) {
        let usable_w = (width as f32 - 2.0 * self.padding).max(0.0);
        let usable_h = (height as f32 - 2.0 * self.padding).max(0.0);
        let cols = (usable_w / self.cell.width).floor() as usize;
        let rows = (usable_h / self.cell.height).floor() as usize;
        (cols.max(1), rows.max(1))
    }

    pub(crate) fn set_scale(&mut self, scale: f32) {
        if (scale - self.scale).abs() < f32::EPSILON {
            return;
        }
        self.scale = scale;
        self.padding = 8.0 * scale;
        self.buffer.set_metrics(metrics(&self.font, scale));
        self.cell = measure_cell(&mut self.font_system, &self.font, scale);
    }

    pub(crate) fn set_text(&mut self, text: &str) {
        let family = self.font.family.clone();
        let attrs = attrs(family.as_deref());
        self.buffer.set_text(text, &attrs, Shaping::Advanced, None);
    }

    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        color: Color,
    ) -> anyhow::Result<()> {
        self.viewport.update(queue, Resolution { width, height });
        self.buffer.set_size(
            Some(width as f32 - 2.0 * self.padding),
            Some(height as f32 - 2.0 * self.padding),
        );
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            [TextArea {
                buffer: &self.buffer,
                left: self.padding,
                top: self.padding,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                },
                default_color: color,
                custom_glyphs: &[],
            }],
            &mut self.swash_cache,
        )?;
        Ok(())
    }

    pub(crate) fn render(&self, pass: &mut wgpu::RenderPass<'_>) -> anyhow::Result<()> {
        self.renderer.render(&self.atlas, &self.viewport, pass)?;
        Ok(())
    }

    pub(crate) fn trim(&mut self) {
        self.atlas.trim();
    }
}

fn metrics(font: &FontConfig, scale: f32) -> Metrics {
    let size = font.size * scale;
    Metrics::new(size, (size * font.line_height).round())
}

fn attrs(family: Option<&str>) -> Attrs<'_> {
    match family {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new().family(Family::Monospace),
    }
}

fn measure_cell(font_system: &mut FontSystem, font: &FontConfig, scale: f32) -> CellSize {
    let metrics = metrics(font, scale);
    let mut probe = Buffer::new(font_system, metrics);
    probe.set_text("M", &attrs(font.family.as_deref()), Shaping::Advanced, None);
    probe.shape_until_scroll(font_system, false);
    let width = probe
        .layout_runs()
        .next()
        .and_then(|run| run.glyphs.first().map(|g| g.w))
        .unwrap_or(metrics.font_size * 0.6);
    CellSize {
        width,
        height: metrics.line_height,
    }
}
