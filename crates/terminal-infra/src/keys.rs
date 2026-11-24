//! Keyboard input to VT byte sequences (xterm conventions).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedKey {
    Enter,
    Backspace,
    Tab,
    Escape,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key<'a> {
    Named(NamedKey),
    /// The key's character ignoring modifiers (e.g. `c` for Ctrl+C), plus the text it produced.
    Char {
        base: &'a str,
        text: Option<&'a str>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Mods {
    fn any(self) -> bool {
        self.shift || self.ctrl || self.alt
    }

    /// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModes {
    pub app_cursor: bool,
}

pub fn encode_key(key: &Key<'_>, mods: Mods, modes: InputModes) -> Option<Vec<u8>> {
    match key {
        Key::Named(named) => Some(encode_named(*named, mods, modes)),
        Key::Char { base, text } => encode_char(base, *text, mods),
    }
}

fn encode_named(key: NamedKey, mods: Mods, modes: InputModes) -> Vec<u8> {
    let alt_prefixed = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if mods.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        out
    };
    match key {
        NamedKey::Enter => alt_prefixed(b"\r"),
        NamedKey::Backspace => alt_prefixed(if mods.ctrl { b"\x08" } else { b"\x7f" }),
        NamedKey::Tab if mods.shift => b"\x1b[Z".to_vec(),
        NamedKey::Tab => alt_prefixed(b"\t"),
        NamedKey::Escape => alt_prefixed(b"\x1b"),
        NamedKey::Space if mods.ctrl => alt_prefixed(b"\0"),
        NamedKey::Space => alt_prefixed(b" "),
        NamedKey::Up => cursor_key(b'A', mods, modes),
        NamedKey::Down => cursor_key(b'B', mods, modes),
        NamedKey::Right => cursor_key(b'C', mods, modes),
        NamedKey::Left => cursor_key(b'D', mods, modes),
        NamedKey::Home => cursor_key(b'H', mods, modes),
        NamedKey::End => cursor_key(b'F', mods, modes),
        NamedKey::Insert => tilde_key(2, mods),
        NamedKey::Delete => tilde_key(3, mods),
        NamedKey::PageUp => tilde_key(5, mods),
        NamedKey::PageDown => tilde_key(6, mods),
        NamedKey::F(n @ 1..=4) => ss3_or_csi(b'P' + (n - 1), mods),
        NamedKey::F(n) => match function_code(n) {
            Some(code) => tilde_key(code, mods),
            None => Vec::new(),
        },
    }
}

fn cursor_key(letter: u8, mods: Mods, modes: InputModes) -> Vec<u8> {
    if mods.any() {
        format!("\x1b[1;{}{}", mods.param(), letter as char).into_bytes()
    } else if modes.app_cursor {
        vec![0x1b, b'O', letter]
    } else {
        vec![0x1b, b'[', letter]
    }
}

fn ss3_or_csi(letter: u8, mods: Mods) -> Vec<u8> {
    if mods.any() {
        format!("\x1b[1;{}{}", mods.param(), letter as char).into_bytes()
    } else {
        vec![0x1b, b'O', letter]
    }
}

fn tilde_key(code: u8, mods: Mods) -> Vec<u8> {
    if mods.any() {
        format!("\x1b[{code};{}~", mods.param()).into_bytes()
    } else {
        format!("\x1b[{code}~").into_bytes()
    }
}

fn function_code(n: u8) -> Option<u8> {
    match n {
        5 => Some(15),
        6 => Some(17),
        7 => Some(18),
        8 => Some(19),
        9 => Some(20),
        10 => Some(21),
        11 => Some(23),
        12 => Some(24),
        _ => None,
    }
}

fn encode_char(base: &str, text: Option<&str>, mods: Mods) -> Option<Vec<u8>> {
    // On Windows AltGr arrives as Ctrl+Alt; if it produced printable text, that text wins.
    let altgr_text = mods.ctrl
        && mods.alt
        && text.is_some_and(|t| !t.is_empty() && !t.chars().any(char::is_control));
    if mods.ctrl
        && !altgr_text
        && let Some(code) = control_code(base)
    {
        let mut out = Vec::with_capacity(2);
        if mods.alt {
            out.push(0x1b);
        }
        out.push(code);
        return Some(out);
    }
    let text = text.filter(|t| !t.is_empty())?;
    let mut out = Vec::with_capacity(text.len() + 1);
    if mods.alt && !mods.ctrl {
        out.push(0x1b);
    }
    out.extend_from_slice(text.as_bytes());
    Some(out)
}

fn control_code(base: &str) -> Option<u8> {
    let mut chars = base.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | '2' | ' ' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '8' | '?' => Some(0x7f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const CTRL: Mods = Mods {
        shift: false,
        ctrl: true,
        alt: false,
    };
    const ALT: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: true,
    };
    const SHIFT: Mods = Mods {
        shift: true,
        ctrl: false,
        alt: false,
    };

    fn named(key: NamedKey, mods: Mods) -> Vec<u8> {
        encode_key(&Key::Named(key), mods, InputModes::default()).unwrap()
    }

    fn chr(base: &str, text: Option<&str>, mods: Mods) -> Option<Vec<u8>> {
        encode_key(&Key::Char { base, text }, mods, InputModes::default())
    }

    #[test]
    fn plain_text_is_sent_as_utf8() {
        assert_eq!(chr("a", Some("a"), NONE).unwrap(), b"a");
        assert_eq!(chr("á", Some("á"), NONE).unwrap(), "á".as_bytes());
        assert_eq!(chr("a", Some("A"), SHIFT).unwrap(), b"A");
    }

    #[test]
    fn ctrl_letters_map_to_control_codes() {
        assert_eq!(chr("c", None, CTRL).unwrap(), [0x03]);
        assert_eq!(chr("d", None, CTRL).unwrap(), [0x04]);
        assert_eq!(chr("[", None, CTRL).unwrap(), [0x1b]);
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(chr("b", Some("b"), ALT).unwrap(), b"\x1bb");
        assert_eq!(named(NamedKey::Backspace, ALT), b"\x1b\x7f");
    }

    #[test]
    fn altgr_sends_produced_text() {
        let altgr = Mods {
            shift: false,
            ctrl: true,
            alt: true,
        };
        assert_eq!(chr("@", Some("@"), altgr).unwrap(), b"@");
    }

    #[test]
    fn arrows_follow_cursor_mode_and_modifiers() {
        assert_eq!(named(NamedKey::Up, NONE), b"\x1b[A");
        let app = InputModes { app_cursor: true };
        assert_eq!(
            encode_key(&Key::Named(NamedKey::Up), NONE, app).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(named(NamedKey::Right, CTRL), b"\x1b[1;5C");
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(named(NamedKey::Enter, NONE), b"\r");
        assert_eq!(named(NamedKey::Backspace, NONE), b"\x7f");
        assert_eq!(named(NamedKey::Tab, SHIFT), b"\x1b[Z");
        assert_eq!(named(NamedKey::Delete, NONE), b"\x1b[3~");
        assert_eq!(named(NamedKey::PageUp, SHIFT), b"\x1b[5;2~");
        assert_eq!(named(NamedKey::F(1), NONE), b"\x1bOP");
        assert_eq!(named(NamedKey::F(5), NONE), b"\x1b[15~");
        assert_eq!(named(NamedKey::F(12), CTRL), b"\x1b[24;5~");
    }
}
