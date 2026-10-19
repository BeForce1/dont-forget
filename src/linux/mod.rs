// Linux (X11) front end.
//
// The middle button is caught with a passive grab on the root window in synchronous mode, so at
// the moment of the press we decide: over a note -> delete it; over an ignored app -> replay the
// press to that app untouched (drags keep working); otherwise keep the grab. A quick click is then
// replayed with XTest, a hold or drag opens the selection overlay. Notes are override-redirect
// windows; following notes get an empty input shape so clicks pass through them.
mod draw;
mod overlay;

use crate::common::{
    anim,
    games::{is_game_path, same_app},
    settings::{After, Settings},
};
use overlay::Overlay;
use std::{
    collections::HashMap,
    error::Error,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};
use draw::Image;
use x11rb::{
    connection::{Connection, RequestConnection},
    wrapper::ConnectionExt as _,
    protocol::{
        res::{self, ConnectionExt as _},
        shape::{self, ConnectionExt as _},
        xproto::*,
        xtest::ConnectionExt as _,
        Event,
    },
    rust_connection::RustConnection,
    CURRENT_TIME, NONE,
};

pub type Res<T> = Result<T, Box<dyn Error>>;
pub type Conn = Arc<RustConnection>;

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
    Selecting,
    Consumed, // the press deleted a note; swallow its release
}

struct Note {
    win: Window,
    image: Image,
    edge: (u8, u8, u8),
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

pub struct App {
    conn: Conn,
    root: Window,
    depth: u8,
    visual: Visualid,
    screen: (i32, i32),
    gc: Gcontext,
    crosshair: Cursor,
    net_wm_pid: Atom,
    wm_state: Atom,
    settings: Settings,
    notes: Vec<Note>,
    followers: Vec<Window>, // newest first
    press: Press,
    overlay: Option<Overlay>,
    procs: HashMap<u32, Proc>,
    drop_grabbed: bool,
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

const USAGE: &str = "usage: dont-forget            start (runs in the background of your X session)
       dont-forget --status   is it running, how many notes
       dont-forget --clear    delete all notes
       dont-forget --reload   re-read the settings file
       dont-forget --quit     stop it";

fn run() -> Res<()> {
    let sock = socket_path();
    if let Some(arg) = std::env::args().nth(1) {
        let cmd = arg.trim_start_matches('-').to_string();
        if !["status", "clear", "reload", "quit"].contains(&cmd.as_str()) {
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
        let net_wm_pid = conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        let wm_state = conn.intern_atom(false, b"WM_STATE")?.reply()?.atom;
        if conn.extension_information(res::X11_EXTENSION_NAME)?.is_some() {
            conn.res_query_version(1, 2)?.reply()?;
        }
        Ok(App {
            conn,
            root,
            depth,
            visual,
            screen,
            gc,
            crosshair,
            net_wm_pid,
            wm_state,
            settings: Settings::load(),
            notes: Vec::new(),
            followers: Vec::new(),
            press: Press::Idle,
            overlay: None,
            procs: HashMap::new(),
            drop_grabbed: false,
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
        let busy = matches!(self.press, Press::Pending { .. }) || !self.followers.is_empty();
        Duration::from_millis(if busy { anim::FRAME_MS as u64 } else { 1000 })
    }

    fn note_index(&self, win: Window) -> Option<usize> {
        self.notes.iter().position(|n| n.win == win)
    }

    // ---------- events ----------

    fn on_event(&mut self, e: Event) -> Res<()> {
        match e {
            Event::ButtonPress(e) if e.event == self.root => self.on_root_press(e)?,
            Event::ButtonRelease(e) if e.event == self.root => self.on_root_release(e)?,
            Event::MotionNotify(e) if e.event == self.root => self.on_root_motion(e.root_x, e.root_y)?,
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
        match e.detail {
            2 => {
                // the pointer is frozen (synchronous grab) until we decide
                if self.press != Press::Idle {
                    conn.allow_events(Allow::ASYNC_POINTER, e.time)?;
                } else if self.note_index(e.child).is_some() {
                    self.close_note(e.child)?;
                    self.press = Press::Consumed;
                    conn.allow_events(Allow::ASYNC_POINTER, e.time)?;
                } else if self.is_ignored(e.child) {
                    conn.allow_events(Allow::REPLAY_POINTER, e.time)?;
                } else {
                    self.press = Press::Pending { x: e.root_x, y: e.root_y, at: Instant::now() };
                    conn.allow_events(Allow::ASYNC_POINTER, e.time)?;
                }
            }
            1 if self.drop_grabbed => {
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
            3 if self.press == Press::Selecting => self.cancel_select()?,
            _ => {}
        }
        Ok(())
    }

    fn on_root_motion(&mut self, x: i16, y: i16) -> Res<()> {
        match self.press {
            Press::Pending { x: dx, y: dy, .. } if (x - dx).abs() > 6 || (y - dy).abs() > 6 => self.begin_select(),
            Press::Selecting => {
                if let Some(ov) = &mut self.overlay {
                    ov.track(&self.conn, (x as i32, y as i32))?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn on_root_release(&mut self, e: ButtonReleaseEvent) -> Res<()> {
        if e.detail != 2 {
            return Ok(());
        }
        match std::mem::replace(&mut self.press, Press::Idle) {
            Press::Pending { .. } => {
                if self.followers.is_empty() {
                    self.replay_click()?;
                } else {
                    self.pin_followers()?; // a quick wheel click drops the following notes
                }
            }
            Press::Selecting => self.finish_select((e.root_x as i32, e.root_y as i32))?,
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
                for w in self.notes.iter().map(|n| n.win).collect::<Vec<_>>() {
                    self.close_note(w)?;
                }
                "all notes deleted".into()
            }
            "reload" => {
                self.settings = Settings::load();
                "settings reloaded".into()
            }
            _ => format!("running, {} note(s), {} following", self.notes.len(), self.followers.len()),
        };
        let _ = writeln!(s, "{reply}");
        Ok(())
    }

    // ---------- ignored apps ----------

    /// Is the top-level window under the pointer owned by an ignored process?
    /// Should this press go straight to the app? Ignored apps, and games unless allowed.
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
        let prop = self.conn.get_property(false, client, self.net_wm_pid, AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
        if let Some(pid) = prop.value32().and_then(|mut v| v.next()) {
            return Some(pid);
        }
        let spec = res::ClientIdSpec { client, mask: res::ClientIdMask::LOCAL_CLIENT_PID };
        let reply = self.conn.res_query_client_ids(&[spec]).ok()?.reply().ok()?;
        reply.ids.iter().find_map(|id| id.value.first().copied())
    }

    fn find_client(&self, win: Window, depth: u8) -> Option<Window> {
        let state = self.conn.get_property(false, win, self.wm_state, AtomEnum::ANY, 0, 0).ok()?.reply().ok()?;
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

    fn begin_select(&mut self) -> Res<()> {
        let Press::Pending { x, y, .. } = self.press else { return Ok(()) };
        self.press = Press::Selecting;
        let ov = Overlay::open(&self.conn, self.root, self.depth, self.visual, self.screen, (x as i32, y as i32))?;
        let mask = EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
        self.conn.change_active_pointer_grab(self.crosshair, CURRENT_TIME, mask)?;
        self.overlay = Some(ov);
        Ok(())
    }

    fn cancel_select(&mut self) -> Res<()> {
        if let Some(ov) = self.overlay.take() {
            ov.close(&self.conn)?;
        }
        self.press = Press::Consumed;
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
        let size = (image.width() as i32 + 2 * draw::BORDER, image.height() as i32 + 2 * draw::BORDER);
        let pos = (at.0 - draw::BORDER, at.1 - draw::BORDER);
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
            image,
            edge: if additive { draw::BLUE } else { draw::GREEN },
            scale: 1.0,
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
        self.conn.map_window(win)?;
        self.conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        let mode = match self.settings.after {
            After::Follow => Mode::Follow,
            After::Drop => Mode::Drop,
            After::Stay => Mode::Pinned,
        };
        self.set_mode(win, mode)
    }

    fn close_note(&mut self, win: Window) -> Res<()> {
        if let Some(i) = self.note_index(win) {
            self.notes.remove(i);
            self.followers.retain(|&w| w != win);
            self.conn.destroy_window(win)?;
            self.update_drop_grab()?;
        }
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
        let pm = draw::render_note(&n.image, n.size.0, n.size.1, n.edge, n.hover, n.close_hot);
        put_image(&self.conn, n.win, self.gc, self.depth, n.size.0, n.size.1, &draw::to_bgrx(&pm))
    }

    fn set_scale(&mut self, i: usize, scale: f32) -> Res<()> {
        let n = &mut self.notes[i];
        n.scale = scale.clamp(0.2, 4.0);
        n.size = (
            (n.image.width() as f32 * n.scale).round() as i32 + 2 * draw::BORDER,
            (n.image.height() as f32 * n.scale).round() as i32 + 2 * draw::BORDER,
        );
        let (win, (w, h)) = (n.win, n.size);
        self.conn.configure_window(win, &ConfigureWindowAux::new().width(w as u32).height(h as u32))?;
        self.apply_shape(win)?;
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
        let n = &mut self.notes[i];
        let (cx, cy, s) = draw::close_rect(n.size.0, n.size.1);
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
        let (clicked, hot, win) = (n.drag.take().is_some() && !n.dragged, n.close_hot, n.win);
        if clicked && hot {
            self.close_note(win)?;
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
        if !self.followers.is_empty() && self.last_follow.elapsed() >= Duration::from_millis(anim::FRAME_MS as u64) {
            self.last_follow = Instant::now();
            self.layout_followers()?;
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
        for i in idx {
            let (px, py) = self.notes[i].pos;
            self.move_note(i, anim::glide(px, x), anim::glide(py, y))?;
            y += self.notes[i].size.1 + GAP;
        }
        Ok(())
    }
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
