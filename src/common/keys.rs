// Keyboard shortcuts as written in the settings file, e.g. "Ctrl+Alt+S" or "Shift+F12".

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    /// An uppercase letter or a digit.
    Char(u8),
    /// F1..=F24.
    F(u8),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

/// Parses "Ctrl+Alt+S"; modifiers are Ctrl, Alt, Shift and Win (or Super), keys are letters,
/// digits and F1-F24. None for an empty or unknown shortcut.
pub fn parse(text: &str) -> Option<Shortcut> {
    let (mut ctrl, mut alt, mut shift, mut win, mut key) = (false, false, false, false, None);
    for part in text.split('+').map(|p| p.trim().to_ascii_uppercase()) {
        match part.as_str() {
            "CTRL" | "CONTROL" => ctrl = true,
            "ALT" => alt = true,
            "SHIFT" => shift = true,
            "WIN" | "SUPER" => win = true,
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => key = Some(Key::Char(k.as_bytes()[0])),
            k if k.starts_with('F') => match k[1..].parse::<u8>() {
                Ok(n @ 1..=24) => key = Some(Key::F(n)),
                _ => return None,
            },
            _ => return None,
        }
    }
    Some(Shortcut { ctrl, alt, shift, win, key: key? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts() {
        let s = parse("Ctrl+Alt+s").unwrap();
        assert!(s.ctrl && s.alt && !s.shift && !s.win && s.key == Key::Char(b'S'));
        let s = parse("shift + f12").unwrap();
        assert!(s.shift && s.key == Key::F(12));
        assert_eq!(parse("Super+1").map(|s| (s.win, s.key)), Some((true, Key::Char(b'1'))));
        assert_eq!(parse("Ctrl+Alt"), None);
        assert_eq!(parse("Ctrl+Space"), None);
        assert_eq!(parse("F25"), None);
        assert_eq!(parse(""), None);
    }
}
