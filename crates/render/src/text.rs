use std::hash::{DefaultHasher, Hash, Hasher};

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, Style,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};

use crate::GridCell;

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

struct Row {
    hash: u64,
    buffer: Buffer,
}

pub(crate) struct TextGrid {
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    rows: Vec<Row>,
    font: FontConfig,
    scale: f32,
    cell: CellSize,
    padding: f32,
    line: String,
    spans: Vec<(usize, usize, SpanStyle)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SpanStyle {
    fg: [u8; 3],
    bold: bool,
    italic: bool,
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
        let cell = measure_cell(&mut font_system, &font, scale);
        log::info!("cell size: {cell:?}");

        Self {
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            rows: Vec::new(),
            padding: 8.0 * scale,
            font,
            scale,
            cell,
            line: String::new(),
            spans: Vec::new(),
        }
    }

    pub(crate) fn cell(&self) -> CellSize {
        self.cell
    }

    pub(crate) fn padding(&self) -> f32 {
        self.padding
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
        self.cell = measure_cell(&mut self.font_system, &self.font, scale);
        self.rows.clear();
    }

    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        cols: usize,
        cells: &[GridCell],
    ) -> anyhow::Result<()> {
        self.viewport.update(queue, Resolution { width, height });
        let row_count = cells.len().checked_div(cols).unwrap_or(0);
        let metrics = metrics(&self.font, self.scale);
        let row_width = cols as f32 * self.cell.width + self.cell.width;

        self.rows.truncate(row_count);
        while self.rows.len() < row_count {
            let mut buffer = Buffer::new(&mut self.font_system, metrics);
            buffer.set_size(Some(row_width), Some(self.cell.height));
            self.rows.push(Row { hash: 0, buffer });
        }

        for (index, row_cells) in cells.chunks(cols.max(1)).enumerate() {
            let hash = row_hash(row_cells, cols);
            let row = &mut self.rows[index];
            if row.hash == hash {
                continue;
            }
            row.hash = hash;
            build_spans(row_cells, &mut self.line, &mut self.spans);
            let family = self.font.family.as_deref();
            let line = &self.line;
            let spans = self
                .spans
                .iter()
                .map(|&(start, end, style)| (&line[start..end], span_attrs(family, style)));
            row.buffer.set_size(Some(row_width), Some(self.cell.height));
            row.buffer
                .set_rich_text(spans, &base_attrs(family), Shaping::Advanced, None);
            row.buffer.shape_until_scroll(&mut self.font_system, false);
        }

        let padding = self.padding;
        let cell_height = self.cell.height;
        let areas = self.rows.iter().enumerate().map(|(index, row)| TextArea {
            buffer: &row.buffer,
            left: padding,
            top: padding + index as f32 * cell_height,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            },
            default_color: Color::rgb(255, 255, 255),
            custom_glyphs: &[],
        });
        self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
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

fn row_hash(cells: &[GridCell], cols: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    cols.hash(&mut hasher);
    for cell in cells {
        (cell.ch, cell.fg, cell.bold, cell.italic, cell.spacer).hash(&mut hasher);
    }
    hasher.finish() | 1
}

fn build_spans(cells: &[GridCell], line: &mut String, spans: &mut Vec<(usize, usize, SpanStyle)>) {
    line.clear();
    spans.clear();
    for cell in cells.iter().filter(|c| !c.spacer) {
        let style = SpanStyle {
            fg: cell.fg,
            bold: cell.bold,
            italic: cell.italic,
        };
        let start = line.len();
        line.push(if cell.ch == '\0' { ' ' } else { cell.ch });
        match spans.last_mut() {
            Some(last) if last.2 == style => last.1 = line.len(),
            _ => spans.push((start, line.len(), style)),
        }
    }
}

fn metrics(font: &FontConfig, scale: f32) -> Metrics {
    let size = font.size * scale;
    Metrics::new(size, (size * font.line_height).round())
}

fn base_attrs(family: Option<&str>) -> Attrs<'_> {
    match family {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new().family(Family::Monospace),
    }
}

fn span_attrs(family: Option<&str>, style: SpanStyle) -> Attrs<'_> {
    let [r, g, b] = style.fg;
    let mut attrs = base_attrs(family).color(Color::rgb(r, g, b));
    if style.bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if style.italic {
        attrs = attrs.style(Style::Italic);
    }
    attrs
}

fn measure_cell(font_system: &mut FontSystem, font: &FontConfig, scale: f32) -> CellSize {
    let metrics = metrics(font, scale);
    let mut probe = Buffer::new(font_system, metrics);
    probe.set_text(
        "M",
        &base_attrs(font.family.as_deref()),
        Shaping::Advanced,
        None,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char, fg: [u8; 3]) -> GridCell {
        GridCell {
            ch,
            fg,
            bg: [0, 0, 0],
            bold: false,
            italic: false,
            underline: false,
            spacer: false,
        }
    }

    #[test]
    fn consecutive_cells_with_same_style_share_a_span() {
        let red = [255, 0, 0];
        let blue = [0, 0, 255];
        let cells = [
            cell('a', red),
            cell('b', red),
            cell('ñ', blue),
            cell(' ', blue),
        ];
        let mut line = String::new();
        let mut spans = Vec::new();
        build_spans(&cells, &mut line, &mut spans);
        assert_eq!(line, "abñ ");
        assert_eq!(spans.len(), 2);
        assert_eq!(&line[spans[0].0..spans[0].1], "ab");
        assert_eq!(&line[spans[1].0..spans[1].1], "ñ ");
    }

    #[test]
    fn spacer_cells_are_skipped() {
        let mut wide = cell('中', [1, 1, 1]);
        wide.spacer = false;
        let mut spacer = cell(' ', [1, 1, 1]);
        spacer.spacer = true;
        let mut line = String::new();
        let mut spans = Vec::new();
        build_spans(&[wide, spacer, cell('x', [1, 1, 1])], &mut line, &mut spans);
        assert_eq!(line, "中x");
    }
}
