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
    pub shown_intro: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            after: After::Follow,
            hold_ms: 250,
            ignored: ["SLDWORKS", "blender", "FreeCAD", "acad"].map(String::from).to_vec(),
            sites: Vec::new(),
            shown_intro: false,
        }
    }
}

pub fn path() -> PathBuf {
    PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join("DontForget").join("settings.ini")
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
                    s.sites = value.split(',').map(crate::sites::normalize).filter(|a| !a.is_empty()).collect()
                }
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
             shown_intro={}\n",
            self.hold_ms,
            self.ignored.join(","),
            self.sites.join(","),
            self.shown_intro as u8
        );
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(p, text);
    }
}
