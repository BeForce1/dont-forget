use std::{env, fs, path::PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub enum After {
    Follow,
    Drop,
    Stay,
}

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
                "shown_intro" => s.shown_intro = value == "1",
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let after = match self.after {
            After::Follow => "follow",
            After::Drop => "drop",
            After::Stay => "stay",
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
             shown_intro={}\n",
            self.hold_ms,
            self.ignored.join(","),
            self.sites.join(","),
            self.block_games as u8,
            self.allowed_games.join(","),
            self.copy_snips as u8,
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
