// Saved notes: one PNG per note plus notes.ini, which lists where each open note sits and the
// deleted ones still kept for undo. Lives next to settings.ini, in a "notes" folder
// (%APPDATA%\DontForget\notes on Windows, ~/.config/dont-forget/notes on Linux).
use crate::common::settings;
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Left-to-right snip, or a right-to-left one that was added to the stack. Picks the frame color.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Snip,
    Add,
}

/// What it takes to bring a note back: the image file is `<id>.png`, (x, y) is the image's top-left.
#[derive(Clone, Copy)]
pub struct Saved {
    pub id: u64,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    pub opacity: u32,
    pub kind: Kind,
}

/// Deleted notes kept for undo, counted across all groups.
pub const MAX_TRASH: usize = 20;

pub fn dir() -> PathBuf {
    settings::path().with_file_name("notes")
}

pub fn image(id: u64) -> PathBuf {
    dir().join(format!("{id}.png"))
}

/// A fresh id: microseconds since 1970, bumped if two notes are made within the same one.
pub fn new_id() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64);
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let id = now.max(last + 1);
        match LAST.compare_exchange(last, id, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return id,
            Err(seen) => last = seen,
        }
    }
}

fn format(s: &Saved) -> String {
    let kind = if s.kind == Kind::Add { "add" } else { "snip" };
    // scale in thousandths: integers keep float parsing and printing out of the exe
    format!("{},{},{},{},{},{kind}", s.id, s.x, s.y, (s.scale * 1000.0).round() as u32, s.opacity)
}

fn parse(v: &str) -> Option<Saved> {
    let f: Vec<&str> = v.split(',').map(str::trim).collect();
    let [id, x, y, scale, opacity, kind] = f[..] else { return None };
    Some(Saved {
        id: id.parse().ok()?,
        x: x.parse().ok()?,
        y: y.parse().ok()?,
        scale: scale.parse::<u32>().ok()? as f32 / 1000.0,
        opacity: opacity.parse().ok()?,
        kind: if kind == "add" { Kind::Add } else { Kind::Snip },
    })
}

/// Open notes in stacking order (bottom first), and deleted groups (oldest first; a group is
/// whatever one delete removed, e.g. everything for "Delete all notes").
pub fn write(open: &[Saved], trash: &[Vec<Saved>]) {
    let mut text = String::from("; Don't Forget notes. Each one is <id>.png in this folder.\n");
    for s in open {
        text += &format!("note={}\n", format(s));
    }
    for (group, list) in trash.iter().enumerate() {
        for s in list {
            text += &format!("deleted={group},{}\n", format(s));
        }
    }
    let p = dir().join("notes.ini");
    let _ = fs::create_dir_all(dir());
    let tmp = p.with_extension("ini.tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, &p);
    }
}

pub fn read() -> (Vec<Saved>, Vec<Vec<Saved>>) {
    let (mut open, mut trash): (Vec<Saved>, Vec<Vec<Saved>>) = (Vec::new(), Vec::new());
    let Ok(text) = fs::read_to_string(dir().join("notes.ini")) else { return (open, trash) };
    let mut last_group = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("note=") {
            open.extend(parse(v));
        } else if let Some(v) = line.strip_prefix("deleted=") {
            let Some((group, rest)) = v.split_once(',') else { continue };
            let Some(s) = parse(rest) else { continue };
            if last_group != Some(group.to_string()) {
                trash.push(Vec::new());
                last_group = Some(group.to_string());
            }
            trash.last_mut().unwrap().push(s);
        }
    }
    (open, trash)
}

pub fn remove_image(id: u64) {
    let _ = fs::remove_file(image(id));
}

/// Deletes every saved note and the list (when notes aren't kept between runs).
pub fn wipe() {
    // the folder is flat, so this is enough (and std's remove_dir_all would add ~30 KB to the exe)
    if let Ok(entries) = fs::read_dir(dir()) {
        for e in entries.flatten() {
            let _ = fs::remove_file(e.path());
        }
    }
    let _ = fs::remove_dir(dir());
}

/// Removes images that no list entry points to, e.g. left behind by a crash.
pub fn remove_orphans(keep: &HashSet<u64>) {
    let Ok(entries) = fs::read_dir(dir()) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let id = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u64>().ok());
        if path.extension().is_some_and(|x| x == "png") && id.is_some_and(|id| !keep.contains(&id)) {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Saved { id: 42, x: -10, y: 20, scale: 1.5, opacity: 85, kind: Kind::Add };
        let back = parse(&format(&s)).unwrap();
        assert_eq!((back.id, back.x, back.y, back.opacity), (42, -10, 20, 85));
        assert!(back.kind == Kind::Add && (back.scale - 1.5).abs() < 1e-6);
        assert!(parse("1,2,3").is_none());
    }

    #[test]
    fn ids_are_unique() {
        let (a, b) = (new_id(), new_id());
        assert!(b > a);
    }
}
