use std::{env, fs, path::PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub enum Outline {
    Solid,
    Dashed,
    Dotted,
}

/// Ready-made (snip, add) frame colors as 0xRRGGBB; any other pair can be typed into the file.
pub const PRESETS: [(&str, u32, u32); 4] = [
    ("Amber and violet", 0xFFB400, 0x9B7BFF),
    ("Green and blue", 0x2ECC71, 0x3498FF),
    ("Coral and teal", 0xFF6B6B, 0x2EC4B6),
    ("White and gray", 0xF2F2F2, 0x8E8E93),
];

#[derive(Clone, Copy, PartialEq)]
pub enum After {
    Follow,
    Drop,
    Stay,
}

#[derive(Clone)]
pub struct Settings {
    pub after: After,
    pub hold_ms: u32,
    /// Process names (no .exe) where the wheel button is left alone, e.g. CAD apps that orbit with it.
    pub ignored: Vec<String>,
    /// Websites (host, optionally with a path) where it is off, e.g. "youtube.com".
    pub sites: Vec<String>,
    /// Stay out of games (detected automatically) unless they are listed in `allowed_games`.
    pub block_games: bool,
    pub allowed_games: Vec<String>,
    /// Put every new snip on the clipboard, ready to paste.
    pub copy_snips: bool,
    /// Save notes to disk and bring them back after a restart.
    pub keep_notes: bool,
    /// Frame colors (0xRRGGBB) of a left-to-right snip and of a right-to-left one that adds to the stack.
    pub snip_color: u32,
    pub add_color: u32,
    pub outline: Outline,
    /// Frame thickness in pixels at 100% display scaling, 1..=6.
    pub outline_width: u32,
    /// Keyboard shortcuts like "Ctrl+Alt+S"; empty turns one off.
    pub snip_key: String,
    pub hide_key: String,
    pub restore_key: String,
    pub shown_intro: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            after: After::Follow,
            hold_ms: 250,
            ignored: ["SLDWORKS", "blender", "FreeCAD", "acad"].map(String::from).to_vec(),
            sites: Vec::new(),
            block_games: true,
            allowed_games: Vec::new(),
            copy_snips: true,
            keep_notes: true,
            snip_color: PRESETS[0].1,
            add_color: PRESETS[0].2,
            outline: Outline::Solid,
            outline_width: 2,
            snip_key: "Ctrl+Alt+S".into(),
            hide_key: "Ctrl+Alt+H".into(),
            restore_key: "Ctrl+Alt+Z".into(),
            shown_intro: false,
        }
    }
}

/// Windows: %APPDATA%\DontForget\settings.ini, Linux: $XDG_CONFIG_HOME (or ~/.config)/dont-forget/settings.ini
pub fn path() -> PathBuf {
    if cfg!(windows) {
        return PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join("DontForget").join("settings.ini");
    }
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("dont-forget").join("settings.ini")
}

impl Settings {
    pub fn load() -> Settings {
        let mut s = Settings::default();
        let Ok(text) = fs::read_to_string(path()) else { return s };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.split(';').next().unwrap_or("").trim();
            match key.trim() {
                "after_capture" => {
                    s.after = match value {
                        "drop" => After::Drop,
                        "stay" => After::Stay,
                        _ => After::Follow,
                    }
                }
                "hold_ms" => s.hold_ms = value.parse().unwrap_or(s.hold_ms).clamp(50, 2000),
                "ignored_apps" => {
                    s.ignored = value.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect()
                }
                "ignored_sites" => {
                    s.sites = value.split(',').map(crate::common::sites::normalize).filter(|a| !a.is_empty()).collect()
                }
                "block_games" => s.block_games = value != "0",
                "allowed_games" => {
                    s.allowed_games = value.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect()
                }
                "copy_snips" => s.copy_snips = value != "0",
                "keep_notes" => s.keep_notes = value != "0",
                "snip_color" => s.snip_color = parse_color(value).unwrap_or(s.snip_color),
                "add_color" => s.add_color = parse_color(value).unwrap_or(s.add_color),
                "outline" => {
                    s.outline = match value {
                        "dashed" => Outline::Dashed,
                        "dotted" => Outline::Dotted,
                        _ => Outline::Solid,
                    }
                }
                "outline_width" => s.outline_width = value.parse().unwrap_or(s.outline_width).clamp(1, 6),
                "snip_key" => s.snip_key = value.to_string(),
                "hide_key" => s.hide_key = value.to_string(),
                "restore_key" => s.restore_key = value.to_string(),
                "shown_intro" => s.shown_intro = value == "1",
                _ => {}
            }
        }
        s
    }

    #[cfg_attr(not(windows), allow(dead_code))] // Linux has no settings UI that writes the file
    pub fn save(&self) {
        let after = match self.after {
            After::Follow => "follow",
            After::Drop => "drop",
            After::Stay => "stay",
        };
        let outline = match self.outline {
            Outline::Solid => "solid",
            Outline::Dashed => "dashed",
            Outline::Dotted => "dotted",
        };
        let text = format!(
            "; Don't Forget settings. Changes apply when you close Notepad.\n\
             after_capture={after}   ; follow | drop | stay\n\
             hold_ms={}   ; how long to hold the wheel before snipping\n\
             ignored_apps={}   ; process names where the wheel is left alone\n\
             ignored_sites={}   ; websites where it is off, e.g. youtube.com\n\
             block_games={}   ; 1 = off in games unless listed in allowed_games\n\
             allowed_games={}   ; games where it still works\n\
             copy_snips={}   ; 1 = every new snip is also copied to the clipboard\n\
             keep_notes={}   ; 1 = notes come back after a restart\n\
             snip_color=#{:06X}   ; frame of a left-to-right snip\n\
             add_color=#{:06X}   ; frame of a right-to-left snip (adds to the stack)\n\
             outline={outline}   ; solid | dashed | dotted\n\
             outline_width={}   ; 1 to 6 pixels\n\
             snip_key={}   ; snip with the keyboard, then drag with the left button\n\
             hide_key={}   ; hide or show all notes\n\
             restore_key={}   ; bring back the last deleted note\n\
             shown_intro={}\n",
            self.hold_ms,
            self.ignored.join(","),
            self.sites.join(","),
            self.block_games as u8,
            self.allowed_games.join(","),
            self.copy_snips as u8,
            self.keep_notes as u8,
            self.snip_color,
            self.add_color,
            self.outline_width,
            self.snip_key,
            self.hide_key,
            self.restore_key,
            self.shown_intro as u8
        );
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // write a temp file and rename it over the old one, so a crash mid-write can't wipe the settings
        let tmp = p.with_extension("ini.tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, &p);
        }
    }
}

/// "#RRGGBB" or "RRGGBB" -> 0xRRGGBB.
pub fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#ffb400"), Some(0xFFB400));
        assert_eq!(parse_color(" 2ECC71 "), Some(0x2ECC71));
        assert_eq!(parse_color("#fff"), None);
        assert_eq!(parse_color("#gg0000"), None);
    }
}
