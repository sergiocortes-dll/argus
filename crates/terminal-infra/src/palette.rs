use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

const ANSI: [Rgb; 16] = [
    rgb(0x1d, 0x1f, 0x21),
    rgb(0xcc, 0x66, 0x66),
    rgb(0xb5, 0xbd, 0x68),
    rgb(0xf0, 0xc6, 0x74),
    rgb(0x81, 0xa2, 0xbe),
    rgb(0xb2, 0x94, 0xbb),
    rgb(0x8a, 0xbe, 0xb7),
    rgb(0xc5, 0xc8, 0xc6),
    rgb(0x66, 0x66, 0x66),
    rgb(0xd5, 0x4e, 0x53),
    rgb(0xb9, 0xca, 0x4a),
    rgb(0xe7, 0xc5, 0x47),
    rgb(0x7a, 0xa6, 0xda),
    rgb(0xc3, 0x97, 0xd8),
    rgb(0x70, 0xc0, 0xb1),
    rgb(0xea, 0xea, 0xea),
];

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

#[derive(Debug, Clone)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    indexed: [Rgb; 256],
}

impl Default for Palette {
    fn default() -> Self {
        let mut indexed = [rgb(0, 0, 0); 256];
        indexed[..16].copy_from_slice(&ANSI);
        let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
        for i in 0..216u8 {
            let (r, g, b) = (i / 36, (i / 6) % 6, i % 6);
            indexed[16 + i as usize] = rgb(level(r), level(g), level(b));
        }
        for i in 0..24u8 {
            let v = 8 + i * 10;
            indexed[232 + i as usize] = rgb(v, v, v);
        }
        Self {
            foreground: rgb(0xd8, 0xdc, 0xe3),
            background: rgb(0x0a, 0x0d, 0x12),
            cursor: rgb(0xd8, 0xdc, 0xe3),
            indexed,
        }
    }
}

impl Palette {
    /// `overrides` are colors the running program changed with OSC 4/10/11/12.
    pub(crate) fn resolve(&self, color: Color, overrides: &Colors) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(i) => overrides[i as usize].unwrap_or(self.indexed[i as usize]),
            Color::Named(name) => overrides[name].unwrap_or_else(|| self.named(name)),
        }
    }

    fn named(&self, name: NamedColor) -> Rgb {
        match name {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground),
            NamedColor::DimBlack => dim(self.indexed[0]),
            NamedColor::DimRed => dim(self.indexed[1]),
            NamedColor::DimGreen => dim(self.indexed[2]),
            NamedColor::DimYellow => dim(self.indexed[3]),
            NamedColor::DimBlue => dim(self.indexed[4]),
            NamedColor::DimMagenta => dim(self.indexed[5]),
            NamedColor::DimCyan => dim(self.indexed[6]),
            NamedColor::DimWhite => dim(self.indexed[7]),
            other => self.indexed[other as usize],
        }
    }
}

pub(crate) fn dim(c: Rgb) -> Rgb {
    let f = |v: u8| ((v as u16 * 2) / 3) as u8;
    rgb(f(c.r), f(c.g), f(c.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_cube_and_grayscale_match_xterm() {
        let p = Palette::default();
        assert_eq!(p.indexed[16], rgb(0, 0, 0));
        assert_eq!(p.indexed[231], rgb(255, 255, 255));
        assert_eq!(p.indexed[196], rgb(255, 0, 0));
        assert_eq!(p.indexed[232], rgb(8, 8, 8));
        assert_eq!(p.indexed[255], rgb(238, 238, 238));
    }
}
