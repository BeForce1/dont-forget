// Linux (X11) front end.
//
// The middle button is caught with a passive grab on the root window in synchronous mode, so at
// the moment of the press we decide: over a note -> delete it; over an ignored app -> replay the
// press to that app untouched (drags keep working); otherwise keep the grab. A quick click is then
// replayed with XTest, a hold or drag opens the selection overlay. Notes are override-redirect
// windows; following notes get an empty input shape so clicks pass through them.
//
// Keyboard shortcuts are passive key grabs on the root window. Notes are kept as PNGs next to the
// settings file (see common::store), and a new snip is offered on the CLIPBOARD selection as
// image/png, in pieces (INCR) when it is large.
mod draw;
mod overlay;
mod png;

use crate::common::{
    anim,
    games::{is_game_path, same_app},
    keys::{self, Key},
    settings::{After, Settings},
    store::{self, Kind, Saved},
};
use draw::{Frame, Image};
use overlay::Overlay;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::{Connection, RequestConnection},
    protocol::{
        res::{self, ConnectionExt as _},
        shape::{self, ConnectionExt as _},
        xproto::*,
        xtest::ConnectionExt as _,
        Event,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
    CURRENT_TIME, NONE,
};

pub type Res<T> = Result<T, Box<dyn Error>>;
pub type Conn = Arc<RustConnection>;

/// notes.ini is written this long after the last change (many changes, one write).
const SAVE_DELAY: Duration = Duration::from_millis(400);
/// Clipboard data bigger than this goes out in pieces (the INCR protocol).
const CLIP_CHUNK: usize = 256 * 1024;
const XK_ESCAPE: u32 = 0xff1b;

enum Msg {
    X(Event),
    Cmd(String, UnixStream),
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Pinned,
    Follow,
    Drop,
}

#[derive(Clone, Copy, PartialEq)]
enum Press {
    Idle,
    Pending { x: i16, y: i16, at: Instant },
    /// The snip shortcut was pressed; the overlay is up, waiting for a left-button drag.
    KeyWait,
    /// Dragging out a selection with this button (2 = wheel, 1 = after the shortcut).
    Selecting(u8),
    Consumed, // the press deleted a note or was cancelled; swallow its release
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Snip,
    Hide,
    Restore,
}

struct Note {
    win: Window,
    id: u64,
    image: Image,
    kind: Kind,
    scale: f32,
    mode: Mode,
    pos: (i32, i32),
    size: (i32, i32),
    hover: bool,
    close_hot: bool,
    click_through: bool,
    drag: Option<((i16, i16), (i32, i32))>, // pointer start, window start
    dragged: bool,
}

/// What we know about the process behind a window, cached per process id.
struct Proc {
    name: String,
    game: bool,
}

/// A large clipboard transfer in progress: the next piece goes out when the requestor deletes
/// the property it read.
struct Incr {
    win: Window,
    prop: Atom,
    data: Arc<Vec<u8>>,
    sent: usize,
}

struct Atoms {
    net_wm_pid: Atom,
    wm_state: Atom,
    clipboard: Atom,
    targets: Atom,
    png: Atom,
    incr: Atom,
}

pub struct App {
    conn: Conn,
    root: Window,
    depth: u8,
    visual: Visualid,
    screen: (i32, i32),
    gc: Gcontext,
    crosshair: Cursor,
    atoms: Atoms,
    settings: Settings,
    notes: Vec<Note>,
    followers: Vec<Window>, // newest first
    /// Deleted notes for undo, oldest group first; each group is what one delete removed.
    trash: Vec<Vec<Saved>>,
    hidden: bool,
    press: Press,
    overlay: Option<Overlay>,
    procs: HashMap<u32, Proc>,
    drop_grabbed: bool,
    /// Grabbed shortcuts: (what it does, keycode, modifiers).
    hotkeys: Vec<(Action, Keycode, u16)>,
    /// The window that owns the clipboard, and what it offers.
    clip_win: Window,
    clip: Option<Arc<Vec<u8>>>,
    incr: Vec<Incr>,
    save_at: Option<Instant>,
    last_follow: Instant,
    quit: bool,
}

pub fn main() {
    if let Err(e) = run() {
        eprintln!("dont-forget: {e}");
        std::process::exit(1);
    }
}

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("dont-forget.sock")
}

const USAGE: &str = "usage: dont-forget             start (runs in the background of your X session)
       dont-forget --status    is it running, how many notes
       dont-forget --snip      snip now: drag with the left button
       dont-forget --hide      hide all notes, or show them again
       dont-forget --restore   bring back the last deleted note
       dont-forget --clear     delete all notes (--restore brings them back)
       dont-forget --reload    re-read the settings file
       dont-forget --quit      stop it (notes are kept)";

const COMMANDS: [&str; 7] = ["status", "snip", "hide", "restore", "clear", "reload", "quit"];

fn run() -> Res<()> {
    let sock = socket_path();
    if let Some(arg) = std::env::args().nth(1) {
        let cmd = arg.trim_start_matches('-').to_string();
        if !COMMANDS.contains(&cmd.as_str()) {
            println!("{USAGE}");
            return Ok(());
        }
        match UnixStream::connect(&sock) {
            Ok(mut s) => {
                writeln!(s, "{cmd}")?;
                let mut reply = String::new();
                BufReader::new(s).read_line(&mut reply)?;
                print!("{reply}");
            }
            Err(_) => println!("not running"),
        }
        return Ok(());
    }
    if UnixStream::connect(&sock).is_ok() {
        println!("already running");
        return Ok(());
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;

    let (conn, screen_num) = x11rb::connect(None)?;
    let conn: Conn = Arc::new(conn);
    let mut app = App::new(conn.clone(), screen_num)?;
    app.grab_middle()?;
    let taken = app.grab_keys()?;
    if app.settings.keep_notes {
        app.restore_saved()?;
    } else {
        store::wipe();
    }
    conn.flush()?;

    let (tx, rx) = mpsc::channel();
    {
        let (conn, tx) = (conn.clone(), tx.clone());
        thread::spawn(move || {
            while let Ok(e) = conn.wait_for_event() {
                if tx.send(Msg::X(e)).is_err() {
                    break;
                }
            }
        });
    }
    thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(&mut s).read_line(&mut line).is_ok() && tx.send(Msg::Cmd(line.trim().into(), s)).is_err() {
                break;
            }
        }
    });

    println!("Don't Forget is running. Hold the mouse wheel and drag to snip. `dont-forget --quit` stops it.");
    if !taken.is_empty() {
        println!("{} is already used by another app. Pick another shortcut in the settings file.", taken.join(", "));
    }
    while !app.quit {
        match rx.recv_timeout(app.next_wakeup()) {
            Ok(Msg::X(e)) => app.on_event(e)?,
            Ok(Msg::Cmd(cmd, s)) => app.on_command(&cmd, s)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        app.tick()?;
        conn.flush()?;
    }
    app.save_now();
    if !app.settings.keep_notes {
        store::wipe();
    }
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

impl App {
    fn new(conn: Conn, screen_num: usize) -> Res<App> {
        let s = &conn.setup().roots[screen_num];
        let (root, depth, visual) = (s.root, s.root_depth, s.root_visual);
        let screen = (s.width_in_pixels as i32, s.height_in_pixels as i32);
        if depth != 24 && depth != 32 {
            return Err(format!("unsupported screen depth {depth} (need 24 or 32 bit color)").into());
        }
        for ext in [x11rb::protocol::xtest::X11_EXTENSION_NAME, shape::X11_EXTENSION_NAME] {
            if conn.extension_information(ext)?.is_none() {
                return Err(format!("the X server lacks the {ext} extension").into());
            }
        }
        let gc = conn.generate_id()?;
        conn.create_gc(gc, root, &CreateGCAux::new().graphics_exposures(0))?;
        let font = conn.generate_id()?;
        conn.open_font(font, b"cursor")?;
        let crosshair = conn.generate_id()?;
        conn.create_glyph_cursor(crosshair, font, font, 34, 35, 0, 0, 0, 0xffff, 0xffff, 0xffff)?;
        conn.close_font(font)?;
        let atom = |name: &[u8]| -> Res<Atom> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
        let atoms = Atoms {
            net_wm_pid: atom(b"_NET_WM_PID")?,
            wm_state: atom(b"WM_STATE")?,
            clipboard: atom(b"CLIPBOARD")?,
            targets: atom(b"TARGETS")?,
            png: atom(b"image/png")?,
            incr: atom(b"INCR")?,
        };
        if conn.extension_information(res::X11_EXTENSION_NAME)?.is_some() {
            conn.res_query_version(1, 2)?.reply()?;
        }
        // screen size changes (monitors plugged in or out) arrive as ConfigureNotify on the root
        conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY))?;
        let clip_win = conn.generate_id()?;
        conn.create_window(0, clip_win, root, -10, -10, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?;
        Ok(App {
            conn,
            root,
            depth,
            visual,
            screen,
            gc,
            crosshair,
            atoms,
            settings: Settings::load(),
            notes: Vec::new(),
            followers: Vec::new(),
            trash: Vec::new(),
            hidden: false,
            press: Press::Idle,
            overlay: None,
            procs: HashMap::new(),
            drop_grabbed: false,
            hotkeys: Vec::new(),
            clip_win,
            clip: None,
            incr: Vec::new(),
            save_at: None,
            last_follow: Instant::now(),
            quit: false,
        })
    }

    fn grab_middle(&self) -> Res<()> {
        let mask = EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
        self.conn
            .grab_button(false, self.root, mask, GrabMode::SYNC, GrabMode::ASYNC, NONE, NONE, ButtonIndex::M2, ModMask::ANY)?
            .check()
            .map_err(|e| format!("could not grab the middle mouse button (another app holds it?): {e:?}"))?;
        Ok(())
    }

    fn next_wakeup(&self) -> Duration {
        let busy = matches!(self.press, Press::Pending { .. }) || (!self.followers.is_empty() && !self.hidden);
        let mut wait = Duration::from_millis(if busy { anim::FRAME_MS as u64 } else { 1000 });
        if let Some(at) = self.save_at {
            wait = wait.min(at.saturating_duration_since(Instant::now()));
        }
        wait
    }

    fn note_index(&self, win: Window) -> Option<usize> {
        self.notes.iter().position(|n| n.win == win)
    }

    fn border(&self) -> i32 {
        self.settings.outline_width as i32
    }

    fn frame(&self, kind: Kind) -> Frame {
        let color = if kind == Kind::Add { self.settings.add_color } else { self.settings.snip_color };
        Frame { color, width: self.border(), outline: self.settings.outline }
    }

    // ---------- events ----------

    fn on_event(&mut self, e: Event) -> Res<()> {
        match e {
            Event::ButtonPress(e) if e.event == self.root => self.on_root_press(e)?,
            Event::ButtonRelease(e) if e.event == self.root => self.on_root_release(e)?,
            Event::MotionNotify(e) if e.event == self.root => self.on_root_motion(e.root_x, e.root_y)?,
            Event::KeyPress(e) => self.on_key(e)?,
            Event::MappingNotify(e) if e.request == Mapping::KEYBOARD || e.request == Mapping::MODIFIER => {
                self.grab_keys()?;
            }
            Event::ConfigureNotify(e) if e.window == self.root => {
                self.screen = (e.width as i32, e.height as i32);
                for i in 0..self.notes.len() {
                    self.keep_on_screen(i)?;
                }
            }
            Event::SelectionRequest(e) => self.on_selection_request(e)?,
            Event::SelectionClear(e) if e.selection == self.atoms.clipboard => {
                self.clip = None;
                self.incr.clear();
            }
            Event::PropertyNotify(e) if e.state == Property::DELETE => self.on_property_deleted(e.window, e.atom)?,
            Event::Expose(e) if e.count == 0 => {
                if let Some(ov) = &self.overlay {
                    if ov.win == e.window {
                        ov.redraw_all(&self.conn)?;
                    }
                }
                if let Some(i) = self.note_index(e.window) {
                    self.render_note(i)?;
                }
            }
            Event::EnterNotify(e) => {
                if let Some(i) = self.note_index(e.event) {
                    self.notes[i].hover = true;
                    self.render_note(i)?;
                }
            }
            Event::LeaveNotify(e) => {
                if let Some(i) = self.note_index(e.event) {
                    let n = &mut self.notes[i];
                    n.hover = false;
                    n.close_hot = false;
                    self.render_note(i)?;
                }
            }
            Event::ButtonPress(e) => {
                if let Some(i) = self.note_index(e.event) {
                    self.on_note_press(i, e)?;
                }
            }
            Event::MotionNotify(e) => {
                if let Some(i) = self.note_index(e.event) {
                    self.on_note_motion(i, e.root_x, e.root_y, e.event_x, e.event_y)?;
                }
            }
            Event::ButtonRelease(e) => {
                if let Some(i) = self.note_index(e.event) {
                    self.on_note_release(i, e)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_root_press(&mut self, e: ButtonPressEvent) -> Res<()> {
        let conn = self.conn.clone();
        match (e.detail, self.press) {
            // after the snip shortcut: the left button starts the selection, the right one cancels
            (1, Press::KeyWait) => {
                if let Some(ov) = &mut self.overlay {
                    ov.start = (e.root_x as i32, e.root_y as i32);
                    ov.cur = ov.start;
                }
                self.press = Press::Selecting(1);
            }
            (3, Press::KeyWait | Press::Selecting(_)) => self.cancel_select()?,
            (2, _) => {
                // the pointer is frozen (synchronous grab) until we decide
                if self.press != Press::Idle {
                    conn.allow_events(Allow::ASYNC_POINTER, e.time)?;
                } else if self.note_index(e.child).is_some() {
                    self.delete_notes(vec![e.child])?;
                    self.press = Press::Consumed;
                    conn.allow_events(Allow::ASYNC_POINTER, e.time)?;
                } else if self.is_ignored(e.child) {
                    conn.allow_events(Allow::REPLAY_POINTER, e.time)?;
                } else {
                    self.press = Press::Pending { x: e.root_x, y: e.root_y, at: Instant::now() };
                    conn.allow_events(Allow::ASYNC_POINTER, e.time)?;
                }
            }
            (1, _) if self.drop_grabbed => {
                // "follow until I click": this click places the notes, except in games and ignored apps
                if self.is_ignored(e.child) {
                    conn.allow_events(Allow::REPLAY_POINTER, e.time)?;
                    return Ok(());
                }
                conn.allow_events(Allow::ASYNC_POINTER, e.time)?;
                for w in self.followers.clone() {
                    if self.notes.iter().any(|n| n.win == w && n.mode == Mode::Drop) {
                        self.set_mode(w, Mode::Pinned)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_root_motion(&mut self, x: i16, y: i16) -> Res<()> {
        match self.press {
            Press::Pending { x: dx, y: dy, .. } if (x - dx).abs() > 6 || (y - dy).abs() > 6 => self.begin_select(),
            Press::Selecting(_) => {
                if let Some(ov) = &mut self.overlay {
                    ov.track(&self.conn, (x as i32, y as i32))?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn on_root_release(&mut self, e: ButtonReleaseEvent) -> Res<()> {
        match (e.detail, self.press) {
            (1, Press::Selecting(1)) => {
                self.press = Press::Idle;
                self.finish_select((e.root_x as i32, e.root_y as i32))?;
                self.ungrab_input()?;
            }
            (2, _) => match std::mem::replace(&mut self.press, Press::Idle) {
                Press::Pending { .. } => {
                    if self.followers.is_empty() {
                        self.replay_click()?;
                    } else {
                        self.pin_followers()?; // a quick wheel click drops the following notes
                    }
                }
                Press::Selecting(2) => self.finish_select((e.root_x as i32, e.root_y as i32))?,
                // a wheel release that isn't ours to handle (e.g. during a keyboard snip)
                p @ (Press::KeyWait | Press::Selecting(_)) => self.press = p,
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    /// Sends a quick wheel click on to the app under the pointer.
    fn replay_click(&self) -> Res<()> {
        let c = &self.conn;
        c.ungrab_button(ButtonIndex::M2, self.root, ModMask::ANY)?;
        c.xtest_fake_input(BUTTON_PRESS_EVENT, 2, CURRENT_TIME, self.root, 0, 0, 0)?;
        c.xtest_fake_input(BUTTON_RELEASE_EVENT, 2, CURRENT_TIME, self.root, 0, 0, 0)?;
        c.sync()?;
        self.grab_middle()
    }

    fn on_command(&mut self, cmd: &str, mut s: UnixStream) -> Res<()> {
        let reply = match cmd {
            "quit" => {
                self.quit = true;
                "stopped".to_string()
            }
            "clear" => {
                let all: Vec<Window> = self.notes.iter().map(|n| n.win).collect();
                let n = all.len();
                self.delete_notes(all)?;
                format!("{n} note(s) deleted (--restore brings them back)")
            }
            "restore" => {
                if self.restore_deleted()? {
                    "restored".into()
                } else {
                    "nothing to restore".into()
                }
            }
            "hide" => {
                self.set_hidden(!self.hidden)?;
                if self.hidden { "notes hidden" } else { "notes shown" }.into()
            }
            "snip" => match self.begin_key_select()? {
                true => "drag with the left button to snip (right click or Esc cancels)".into(),
                false => "could not grab the mouse (a menu or another app has it?)".into(),
            },
            "reload" => {
                let taken = self.reload_settings()?;
                if taken.is_empty() {
                    "settings reloaded".into()
                } else {
                    format!("settings reloaded; {} is already used by another app", taken.join(", "))
                }
            }
            _ => format!(
                "running, {} note(s), {} following{}, {} deleted kept for undo",
                self.notes.len(),
                self.followers.len(),
                if self.hidden { ", hidden" } else { "" },
                self.trash.iter().map(Vec::len).sum::<usize>()
            ),
        };
        let _ = writeln!(s, "{reply}");
        Ok(())
    }

    /// Applies a changed settings file to everything on screen right away.
    fn reload_settings(&mut self) -> Res<Vec<String>> {
        let old_border = self.border();
        self.settings = Settings::load();
        let taken = self.grab_keys()?;
        let shift = self.border() - old_border;
        for i in 0..self.notes.len() {
            if shift != 0 {
                // keep the snip itself where it is; the frame grows or shrinks around it
                let (x, y) = self.notes[i].pos;
                self.move_note(i, x - shift, y - shift)?;
            }
            self.set_scale(i, self.notes[i].scale)?;
        }
        self.mark_dirty();
        Ok(taken)
    }

    // ---------- keyboard ----------

    /// (Re-)grabs the shortcuts from the settings; returns the ones another app already holds.
    fn grab_keys(&mut self) -> Res<Vec<String>> {
        let conn = self.conn.clone();
        let locks = [0, u16::from(ModMask::LOCK), u16::from(ModMask::M2), u16::from(ModMask::LOCK | ModMask::M2)];
        for &(_, code, mods) in &self.hotkeys {
            for extra in locks {
                conn.ungrab_key(code, self.root, (mods | extra).into())?;
            }
        }
        self.hotkeys.clear();
        let mut taken = Vec::new();
        let wanted = [
            (Action::Snip, self.settings.snip_key.clone()),
            (Action::Hide, self.settings.hide_key.clone()),
            (Action::Restore, self.settings.restore_key.clone()),
        ];
        for (action, text) in wanted {
            let Some(s) = keys::parse(&text) else { continue };
            let sym = match s.key {
                Key::Char(c) => c.to_ascii_lowercase() as u32,
                Key::F(n) => 0xffbe + n as u32 - 1, // XK_F1..
            };
            let Some(code) = self.keycode(sym)? else { continue };
            let mut mods = 0u16;
            for (on, m) in [(s.ctrl, ModMask::CONTROL), (s.alt, ModMask::M1), (s.shift, ModMask::SHIFT), (s.win, ModMask::M4)] {
                if on {
                    mods |= u16::from(m);
                }
            }
            // also with Caps Lock and Num Lock on, which X counts as modifiers
            let mut ok = true;
            for extra in locks {
                let grab = conn.grab_key(false, self.root, (mods | extra).into(), code, GrabMode::ASYNC, GrabMode::ASYNC)?;
                ok &= grab.check().is_ok();
            }
            if ok {
                self.hotkeys.push((action, code, mods));
            } else {
                for extra in locks {
                    conn.ungrab_key(code, self.root, (mods | extra).into())?;
                }
                taken.push(text);
            }
        }
        Ok(taken)
    }

    /// The keycode that types a keysym, if the keyboard has one.
    fn keycode(&self, sym: u32) -> Res<Option<Keycode>> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self.conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        let per = map.keysyms_per_keycode.max(1) as usize;
        Ok(map.keysyms.chunks(per).position(|syms| syms.contains(&sym)).map(|i| min + i as u8))
    }

    fn on_key(&mut self, e: KeyPressEvent) -> Res<()> {
        if matches!(self.press, Press::KeyWait | Press::Selecting(1)) {
            if self.keycode(XK_ESCAPE)? == Some(e.detail) {
                self.cancel_select()?;
            }
            return Ok(());
        }
        let relevant = u16::from(ModMask::CONTROL | ModMask::SHIFT | ModMask::M1 | ModMask::M4);
        let state = u16::from(e.state) & relevant;
        let Some(&(action, ..)) = self.hotkeys.iter().find(|h| h.1 == e.detail && h.2 == state) else { return Ok(()) };
        match action {
            Action::Snip => {
                self.begin_key_select()?;
            }
            Action::Hide => self.set_hidden(!self.hidden)?,
            Action::Restore => {
                self.restore_deleted()?;
            }
        }
        Ok(())
    }

    // ---------- ignored apps ----------

    /// Should this press go straight to the app under the pointer? Ignored apps, and games unless allowed.
    fn is_ignored(&mut self, top: Window) -> bool {
        if top == NONE {
            return false;
        }
        let Some(pid) = self.window_pid(top) else { return false };
        let p = self.procs.entry(pid).or_insert_with(|| {
            let read = |f: &str| std::fs::read(format!("/proc/{pid}/{f}")).unwrap_or_default();
            let exe = std::fs::read_link(format!("/proc/{pid}/exe")).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            // Proton/Wine games run as wine processes; the game's path is in the command line
            let cmdline = String::from_utf8_lossy(&read("cmdline")).replace('\0', " ");
            Proc {
                name: String::from_utf8_lossy(&read("comm")).trim().to_lowercase(),
                game: is_game_path(&exe) || is_game_path(&cmdline),
            }
        });
        let s = &self.settings;
        if s.ignored.iter().any(|a| same_app(a, &p.name)) {
            return true;
        }
        s.block_games && p.game && !s.allowed_games.iter().any(|a| same_app(a, &p.name))
    }

    /// Process id of the app behind a top-level window. Under a window manager that window is
    /// the manager's frame, so first find the app's own window (the one with WM_STATE), then use
    /// its _NET_WM_PID or, for apps that don't set it, ask the server who owns it (X-Resource).
    fn window_pid(&self, top: Window) -> Option<u32> {
        let client = self.find_client(top, 3).unwrap_or(top);
        let prop = self.conn.get_property(false, client, self.atoms.net_wm_pid, AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
        if let Some(pid) = prop.value32().and_then(|mut v| v.next()) {
            return Some(pid);
        }
        let spec = res::ClientIdSpec { client, mask: res::ClientIdMask::LOCAL_CLIENT_PID };
        let reply = self.conn.res_query_client_ids(&[spec]).ok()?.reply().ok()?;
        reply.ids.iter().find_map(|id| id.value.first().copied())
    }

    fn find_client(&self, win: Window, depth: u8) -> Option<Window> {
        let state = self.conn.get_property(false, win, self.atoms.wm_state, AtomEnum::ANY, 0, 0).ok()?.reply().ok()?;
        if state.type_ != NONE {
            return Some(win);
        }
        if depth == 0 {
            return None;
        }
        let tree = self.conn.query_tree(win).ok()?.reply().ok()?;
        tree.children.iter().rev().find_map(|&c| self.find_client(c, depth - 1))
    }

    // ---------- selection ----------

    fn open_overlay(&mut self, at: (i32, i32)) -> Res<()> {
        let s = &self.settings;
        let style = overlay::Style { colors: (s.snip_color, s.add_color), outline: s.outline, width: s.outline_width };
        let ov = Overlay::open(&self.conn, self.root, self.depth, self.visual, self.screen, at, style)?;
        self.overlay = Some(ov);
        Ok(())
    }

    /// A held or dragged wheel turns into a selection.
    fn begin_select(&mut self) -> Res<()> {
        let Press::Pending { x, y, .. } = self.press else { return Ok(()) };
        self.press = Press::Selecting(2);
        self.open_overlay((x as i32, y as i32))?;
        let mask = EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
        self.conn.change_active_pointer_grab(self.crosshair, CURRENT_TIME, mask)?;
        Ok(())
    }

    /// The snip shortcut: freeze the screen and wait for a left-button drag. False if the mouse
    /// couldn't be grabbed.
    fn begin_key_select(&mut self) -> Res<bool> {
        if self.press != Press::Idle {
            return Ok(true);
        }
        let mask = EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
        let grab = self.conn.grab_pointer(false, self.root, mask, GrabMode::ASYNC, GrabMode::ASYNC, NONE, self.crosshair, CURRENT_TIME)?.reply()?;
        if grab.status != GrabStatus::SUCCESS {
            return Ok(false);
        }
        // the keyboard is only needed for Esc; right click cancels too, so a failure is fine
        self.conn.grab_keyboard(false, self.root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)?.reply()?;
        let p = self.conn.query_pointer(self.root)?.reply()?;
        self.open_overlay((p.root_x as i32, p.root_y as i32))?;
        self.press = Press::KeyWait;
        Ok(true)
    }

    fn ungrab_input(&self) -> Res<()> {
        self.conn.ungrab_pointer(CURRENT_TIME)?;
        self.conn.ungrab_keyboard(CURRENT_TIME)?;
        Ok(())
    }

    fn cancel_select(&mut self) -> Res<()> {
        if let Some(ov) = self.overlay.take() {
            ov.close(&self.conn)?;
        }
        if matches!(self.press, Press::KeyWait | Press::Selecting(1)) {
            self.ungrab_input()?;
            self.press = Press::Idle;
        } else {
            self.press = Press::Consumed; // swallow the wheel's release
        }
        Ok(())
    }

    fn finish_select(&mut self, end: (i32, i32)) -> Res<()> {
        let Some(mut ov) = self.overlay.take() else { return Ok(()) };
        ov.cur = end;
        let crop = ov.crop();
        let (rect, additive) = (ov.selection(), end.0 < ov.start.0);
        ov.close(&self.conn)?;
        if let Some(img) = crop {
            self.add_note(img, (rect.0, rect.1), additive)?;
        }
        Ok(())
    }

    // ---------- notes ----------

    fn add_note(&mut self, image: Image, at: (i32, i32), additive: bool) -> Res<()> {
        if !additive {
            self.pin_followers()?;
        }
        self.set_hidden(false)?;
        let id = store::new_id();
        // written right away, so it survives a crash and can come back after a delete
        let png = png::encode(&image);
        let _ = std::fs::create_dir_all(store::dir());
        let _ = std::fs::write(store::image(id), &png);
        if self.settings.copy_snips {
            self.offer_clipboard(png)?;
        }
        let kind = if additive { Kind::Add } else { Kind::Snip };
        let win = self.create_note(image, at, kind, id, 1.0)?;
        let mode = match self.settings.after {
            After::Follow => Mode::Follow,
            After::Drop => Mode::Drop,
            After::Stay => Mode::Pinned,
        };
        self.set_mode(win, mode)?;
        self.mark_dirty();
        Ok(())
    }

    /// Opens a note window, pinned, with the snip's top-left at `at`.
    fn create_note(&mut self, image: Image, at: (i32, i32), kind: Kind, id: u64, scale: f32) -> Res<Window> {
        let b = self.border();
        let size = note_size(&image, scale, b);
        let pos = (at.0 - b, at.1 - b);
        let win = self.conn.generate_id()?;
        let events = EventMask::EXPOSURE
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::ENTER_WINDOW
            | EventMask::LEAVE_WINDOW;
        self.conn.create_window(
            self.depth,
            win,
            self.root,
            pos.0 as i16,
            pos.1 as i16,
            size.0 as u16,
            size.1 as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            self.visual,
            &CreateWindowAux::new().override_redirect(1).event_mask(events).background_pixel(0),
        )?;
        let wm_name = b"Don't Forget note";
        self.conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, wm_name)?;
        self.notes.push(Note {
            win,
            id,
            image,
            kind,
            scale,
            mode: Mode::Pinned,
            pos,
            size,
            hover: false,
            close_hot: false,
            click_through: false,
            drag: None,
            dragged: false,
        });
        self.apply_shape(win)?;
        if !self.hidden {
            self.conn.map_window(win)?;
            self.conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        }
        Ok(win)
    }

    /// What it takes to bring a note back later.
    fn saved(&self, n: &Note) -> Saved {
        let b = self.border();
        Saved { id: n.id, x: n.pos.0 + b, y: n.pos.1 + b, scale: n.scale, opacity: 100, kind: n.kind }
    }

    /// Opens a saved note again, pinned where it was.
    fn open_saved(&mut self, s: &Saved) -> Res<Option<Window>> {
        let Some(image) = std::fs::read(store::image(s.id)).ok().and_then(|d| png::decode(&d)) else { return Ok(None) };
        let win = self.create_note(image, (s.x, s.y), s.kind, s.id, s.scale.clamp(0.2, 4.0))?;
        self.keep_on_screen(self.notes.len() - 1)?;
        Ok(Some(win))
    }

    /// Brings back the notes that were open when the app last stopped, and the undo list.
    fn restore_saved(&mut self) -> Res<()> {
        let (open, trash) = store::read();
        let mut keep: HashSet<u64> = trash.iter().flatten().map(|s| s.id).collect();
        for s in &open {
            if self.open_saved(s)?.is_some() {
                keep.insert(s.id);
            }
        }
        self.trash = trash;
        store::remove_orphans(&keep);
        self.save_now();
        Ok(())
    }

    /// Deletes notes, keeping them (as one undo step) so the restore shortcut can bring them back.
    fn delete_notes(&mut self, list: Vec<Window>) -> Res<()> {
        let group: Vec<Saved> = list.iter().filter_map(|&w| self.note_index(w)).map(|i| self.saved(&self.notes[i])).collect();
        if group.is_empty() {
            return Ok(());
        }
        self.trash.push(group);
        // forget the oldest deletions beyond the limit, with their images
        while self.trash.iter().map(Vec::len).sum::<usize>() > store::MAX_TRASH && self.trash.len() > 1 {
            for s in self.trash.remove(0) {
                store::remove_image(s.id);
            }
        }
        for win in list {
            if let Some(i) = self.note_index(win) {
                self.notes.remove(i);
                self.followers.retain(|&w| w != win);
                self.conn.destroy_window(win)?;
            }
        }
        self.update_drop_grab()?;
        self.mark_dirty();
        Ok(())
    }

    /// Undo: reopens the most recently deleted note (or everything one `--clear` removed).
    fn restore_deleted(&mut self) -> Res<bool> {
        let Some(group) = self.trash.pop() else { return Ok(false) };
        self.set_hidden(false)?;
        for s in &group {
            self.open_saved(s)?;
        }
        self.mark_dirty();
        Ok(true)
    }

    fn mark_dirty(&mut self) {
        self.save_at.get_or_insert_with(|| Instant::now() + SAVE_DELAY);
    }

    fn save_now(&mut self) {
        self.save_at = None;
        let open: Vec<Saved> = self.notes.iter().map(|n| self.saved(n)).collect();
        store::write(&open, &self.trash);
    }

    /// Hides every note without deleting it (screen sharing, or one is in the way), or shows them again.
    fn set_hidden(&mut self, hide: bool) -> Res<()> {
        if self.hidden == hide {
            return Ok(());
        }
        self.hidden = hide;
        for n in &self.notes {
            if hide {
                self.conn.unmap_window(n.win)?;
            } else {
                self.conn.map_window(n.win)?;
                self.conn.configure_window(n.win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
            }
        }
        Ok(())
    }

    /// Moves a note back onto the screen when (almost) none of it is visible, e.g. after a
    /// monitor was unplugged or the notes were saved on a bigger screen.
    fn keep_on_screen(&mut self, i: usize) -> Res<()> {
        let n = &self.notes[i];
        let ((x, y), (w, h)) = (n.pos, n.size);
        let (sw, sh) = self.screen;
        let vis_w = ((x + w).min(sw) - x.max(0)).clamp(0, 48);
        let vis_h = ((y + h).min(sh) - y.max(0)).clamp(0, 48);
        if vis_w * vis_h >= w.min(48) * h.min(48) {
            return Ok(());
        }
        let nx = x.clamp(0, (sw - w).max(0));
        let ny = y.clamp(0, (sh - h).max(0));
        self.move_note(i, nx, ny)?;
        self.mark_dirty();
        Ok(())
    }

    fn set_mode(&mut self, win: Window, mode: Mode) -> Res<()> {
        let Some(i) = self.note_index(win) else { return Ok(()) };
        self.notes[i].mode = mode;
        self.notes[i].click_through = mode != Mode::Pinned;
        self.apply_shape(win)?;
        self.followers.retain(|&w| w != win);
        if mode != Mode::Pinned {
            self.followers.insert(0, win);
        }
        self.mark_dirty();
        self.update_drop_grab()
    }

    fn pin_followers(&mut self) -> Res<()> {
        for w in self.followers.clone() {
            self.set_mode(w, Mode::Pinned)?;
        }
        Ok(())
    }

    /// While a note waits to be placed, the next left click anywhere places it.
    fn update_drop_grab(&mut self) -> Res<()> {
        let want = self.notes.iter().any(|n| n.mode == Mode::Drop);
        if want && !self.drop_grabbed {
            let mask = EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE;
            self.conn.grab_button(false, self.root, mask, GrabMode::SYNC, GrabMode::ASYNC, NONE, NONE, ButtonIndex::M1, ModMask::ANY)?;
        } else if !want && self.drop_grabbed {
            self.conn.ungrab_button(ButtonIndex::M1, self.root, ModMask::ANY)?;
        }
        self.drop_grabbed = want;
        Ok(())
    }

    /// Rounded outline; following notes also get an empty input region so clicks pass through.
    fn apply_shape(&self, win: Window) -> Res<()> {
        let Some(i) = self.note_index(win) else { return Ok(()) };
        let n = &self.notes[i];
        let rects: Vec<Rectangle> = draw::rounded_shape(n.size.0, n.size.1, draw::WINDOW_RADIUS)
            .into_iter()
            .map(|(x, y, w, h)| Rectangle { x: x as i16, y: y as i16, width: w as u16, height: h as u16 })
            .collect();
        let c = &self.conn;
        c.shape_rectangles(shape::SO::SET, shape::SK::BOUNDING, ClipOrdering::UNSORTED, win, 0, 0, &rects)?;
        if n.click_through {
            c.shape_rectangles(shape::SO::SET, shape::SK::INPUT, ClipOrdering::UNSORTED, win, 0, 0, &[])?;
        } else {
            c.shape_rectangles(shape::SO::SET, shape::SK::INPUT, ClipOrdering::UNSORTED, win, 0, 0, &rects)?;
        }
        Ok(())
    }

    fn render_note(&self, i: usize) -> Res<()> {
        let n = &self.notes[i];
        let pm = draw::render_note(&n.image, n.size.0, n.size.1, self.frame(n.kind), n.hover, n.close_hot);
        put_image(&self.conn, n.win, self.gc, self.depth, n.size.0, n.size.1, &draw::to_bgrx(&pm))
    }

    fn set_scale(&mut self, i: usize, scale: f32) -> Res<()> {
        let b = self.border();
        let n = &mut self.notes[i];
        n.scale = scale.clamp(0.2, 4.0);
        n.size = note_size(&n.image, n.scale, b);
        let (win, (w, h)) = (n.win, n.size);
        self.conn.configure_window(win, &ConfigureWindowAux::new().width(w as u32).height(h as u32))?;
        self.apply_shape(win)?;
        self.mark_dirty();
        self.render_note(i)
    }

    fn move_note(&mut self, i: usize, x: i32, y: i32) -> Res<()> {
        let n = &mut self.notes[i];
        if n.pos != (x, y) {
            n.pos = (x, y);
            self.conn.configure_window(n.win, &ConfigureWindowAux::new().x(x).y(y).stack_mode(StackMode::ABOVE))?;
        }
        Ok(())
    }

    fn on_note_press(&mut self, i: usize, e: ButtonPressEvent) -> Res<()> {
        let n = &mut self.notes[i];
        match e.detail {
            1 => {
                n.drag = Some(((e.root_x, e.root_y), n.pos));
                n.dragged = false;
            }
            3 => {
                // right click: switch between following and pinned
                let (win, next) = (n.win, if n.mode == Mode::Pinned { Mode::Follow } else { Mode::Pinned });
                self.set_mode(win, next)?;
            }
            4 => self.set_scale(i, self.notes[i].scale * 1.1)?,
            5 => self.set_scale(i, self.notes[i].scale / 1.1)?,
            _ => {}
        }
        Ok(())
    }

    fn on_note_motion(&mut self, i: usize, rx: i16, ry: i16, ex: i16, ey: i16) -> Res<()> {
        let n = &mut self.notes[i];
        if let Some(((sx, sy), (wx, wy))) = n.drag {
            if !n.dragged && ((rx - sx).abs() > 3 || (ry - sy).abs() > 3) {
                n.dragged = true;
                if n.mode != Mode::Pinned {
                    let win = n.win;
                    self.set_mode(win, Mode::Pinned)?;
                }
            }
            if self.notes[i].dragged {
                self.move_note(i, wx + (rx - sx) as i32, wy + (ry - sy) as i32)?;
            }
        }
        let b = self.border();
        let n = &mut self.notes[i];
        let (cx, cy, s) = draw::close_rect(n.size.0, n.size.1, b);
        let hot = draw::has_room_for_close(n.size.0, n.size.1)
            && (ex as i32) >= cx
            && (ex as i32) < cx + s
            && (ey as i32) >= cy
            && (ey as i32) < cy + s;
        if hot != n.close_hot {
            n.close_hot = hot;
            self.render_note(i)?;
        }
        Ok(())
    }

    fn on_note_release(&mut self, i: usize, e: ButtonReleaseEvent) -> Res<()> {
        if e.detail != 1 {
            return Ok(());
        }
        let n = &mut self.notes[i];
        let (was_drag, hot, win) = (n.drag.take().is_some(), n.close_hot, n.win);
        let clicked = was_drag && !n.dragged;
        if n.dragged {
            self.mark_dirty();
        }
        if clicked && hot {
            self.delete_notes(vec![win])?;
        }
        Ok(())
    }

    // ---------- clipboard ----------

    /// Offers a PNG on the clipboard, for pasting with Ctrl+V.
    fn offer_clipboard(&mut self, png: Vec<u8>) -> Res<()> {
        self.clip = Some(Arc::new(png));
        self.incr.clear();
        self.conn.set_selection_owner(self.clip_win, self.atoms.clipboard, CURRENT_TIME)?;
        Ok(())
    }

    fn on_selection_request(&mut self, e: SelectionRequestEvent) -> Res<()> {
        let a = &self.atoms;
        // obsolete clients leave the property empty and mean the target's name
        let prop = if e.property == NONE { e.target } else { e.property };
        let mut answered = false;
        if let (Some(data), true) = (self.clip.clone(), e.selection == a.clipboard) {
            if e.target == a.targets {
                self.conn.change_property32(PropMode::REPLACE, e.requestor, prop, AtomEnum::ATOM, &[a.targets, a.png])?;
                answered = true;
            } else if e.target == a.png {
                if data.len() <= CLIP_CHUNK {
                    self.conn.change_property8(PropMode::REPLACE, e.requestor, prop, a.png, &data)?;
                } else {
                    // too big for one request: announce the size, then send a piece each time
                    // the requestor deletes the property
                    let events = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
                    self.conn.change_window_attributes(e.requestor, &events)?;
                    self.conn.change_property32(PropMode::REPLACE, e.requestor, prop, a.incr, &[data.len() as u32])?;
                    self.incr.retain(|t| !(t.win == e.requestor && t.prop == prop));
                    self.incr.push(Incr { win: e.requestor, prop, data, sent: 0 });
                }
                answered = true;
            }
        }
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if answered { prop } else { NONE },
        };
        self.conn.send_event(false, e.requestor, EventMask::NO_EVENT, notify)?;
        Ok(())
    }

    /// The next piece of an INCR transfer; an empty piece ends it.
    fn on_property_deleted(&mut self, win: Window, prop: Atom) -> Res<()> {
        let Some(i) = self.incr.iter().position(|t| t.win == win && t.prop == prop) else { return Ok(()) };
        let t = &mut self.incr[i];
        let end = (t.sent + CLIP_CHUNK).min(t.data.len());
        let piece = t.data[t.sent..end].to_vec();
        let done = piece.is_empty();
        t.sent = end;
        self.conn.change_property8(PropMode::REPLACE, win, prop, self.atoms.png, &piece)?;
        if done {
            self.incr.remove(i);
            if !self.incr.iter().any(|t| t.win == win) {
                self.conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT))?;
            }
        }
        Ok(())
    }

    // ---------- per-frame work ----------

    fn tick(&mut self) -> Res<()> {
        if let Press::Pending { at, .. } = self.press {
            if at.elapsed() >= Duration::from_millis(self.settings.hold_ms as u64) {
                self.begin_select()?;
            }
        }
        if !self.followers.is_empty() && !self.hidden && self.last_follow.elapsed() >= Duration::from_millis(anim::FRAME_MS as u64) {
            self.last_follow = Instant::now();
            self.layout_followers()?;
        }
        if self.save_at.is_some_and(|at| Instant::now() >= at) {
            self.save_now();
        }
        Ok(())
    }

    /// Stacks following notes next to the cursor, gliding toward their spot. Shift freezes them.
    fn layout_followers(&mut self) -> Res<()> {
        let p = self.conn.query_pointer(self.root)?.reply()?;
        let freeze = p.mask.contains(KeyButMask::SHIFT);
        for w in self.followers.clone() {
            if let Some(i) = self.note_index(w) {
                if self.notes[i].click_through == freeze {
                    self.notes[i].click_through = !freeze;
                    self.apply_shape(w)?;
                }
            }
        }
        if freeze {
            return Ok(());
        }
        const GAP: i32 = 6;
        let idx: Vec<usize> = self.followers.iter().filter_map(|&w| self.note_index(w)).collect();
        let width = idx.iter().map(|&i| self.notes[i].size.0).max().unwrap_or(0);
        let height = idx.iter().map(|&i| self.notes[i].size.1).sum::<i32>() + GAP * (idx.len() as i32 - 1);
        let (x, mut y) = anim::stack_origin((p.root_x as i32, p.root_y as i32), (width, height), (0, 0, self.screen.0, self.screen.1));
        // not saved on every step: pinning a note saves where it ended up, and so does quitting
        for i in idx {
            let (px, py) = self.notes[i].pos;
            self.move_note(i, anim::glide(px, x), anim::glide(py, y))?;
            y += self.notes[i].size.1 + GAP;
        }
        Ok(())
    }
}

/// Window size of a note: the snip at `scale` plus the frame on each side.
fn note_size(image: &Image, scale: f32, border: i32) -> (i32, i32) {
    (
        (image.width() as f32 * scale).round() as i32 + 2 * border,
        (image.height() as f32 * scale).round() as i32 + 2 * border,
    )
}

/// Uploads a BGRX image, split into requests that fit the server's size limit.
pub fn put_image(conn: &Conn, drawable: Drawable, gc: Gcontext, depth: u8, w: i32, h: i32, data: &[u8]) -> Res<()> {
    let stride = (w * 4) as usize;
    let rows_per = ((conn.maximum_request_bytes() - 64) / stride).max(1);
    for (chunk_i, chunk) in data.chunks(rows_per * stride).enumerate() {
        let rows = (chunk.len() / stride) as u16;
        let y = (chunk_i * rows_per) as i16;
        conn.put_image(ImageFormat::Z_PIXMAP, drawable, gc, w as u16, rows, 0, y, 0, depth, chunk)?;
    }
    let _ = h;
    Ok(())
}
