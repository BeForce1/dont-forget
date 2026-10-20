// Global low-level mouse hook on its own thread so a busy UI thread never stalls the system mouse.
// Wheel-button press: quick click -> replayed to the app (or releases following notes),
// hold or drag -> screen selection. Results are posted to the hidden main window.
use crate::common::games::{is_game_path, same_app};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    mem::{size_of, zeroed},
    path::Path,
    ptr::{null, null_mut},
    sync::{atomic::*, Mutex},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

pub const WM_SELECT_BEGIN: u32 = WM_APP + 1;
pub const WM_SELECT_END: u32 = WM_APP + 2;
pub const WM_DELETE_NOTE: u32 = WM_APP + 3;
pub const WM_DROP_CLICK: u32 = WM_APP + 4;
pub const WM_RELEASE_FOLLOWERS: u32 = WM_APP + 5;

pub static HIDDEN: AtomicIsize = AtomicIsize::new(0);
pub static DROP_ARMED: AtomicBool = AtomicBool::new(false);
/// While notes follow the cursor, a quick wheel click releases them instead of reaching the app.
pub static FOLLOW_ARMED: AtomicBool = AtomicBool::new(false);
pub static PAUSED: AtomicBool = AtomicBool::new(false);
pub static HOLD_MS: AtomicU32 = AtomicU32::new(250);
pub static NOTES: Mutex<Vec<isize>> = Mutex::new(Vec::new());
pub static IGNORED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Stay out of games unless they're in ALLOWED_GAMES.
pub static BLOCK_GAMES: AtomicBool = AtomicBool::new(true);
pub static ALLOWED_GAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Lowercase exe paths of the games Windows itself has recognized (Game Bar's list).
pub static KNOWN_GAMES: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static THREAD_ID: AtomicU32 = AtomicU32::new(0);

const MAGIC: usize = 0x4446_5247; // marks our own replayed clicks

#[derive(PartialEq)]
enum State {
    Idle,
    Pending,
    Selecting,
}

struct Hook {
    state: State,
    down: POINT,
    eat_middle_up: bool,
    eat_left_up: bool,
    timer: usize,
    procs: HashMap<(u32, u64), Proc>,
}

/// What we know about the process behind a window, cached per process id and start time
/// (Windows reuses ids, so the id alone could hand a new process an old one's verdict).
struct Proc {
    name: String,
    game: bool,
}

thread_local! {
    static HOOK: RefCell<Hook> = RefCell::new(Hook {
        state: State::Idle,
        down: POINT { x: 0, y: 0 },
        eat_middle_up: false,
        eat_left_up: false,
        timer: 0,
        procs: HashMap::new(),
    });
}

pub fn start() {
    std::thread::spawn(|| unsafe {
        THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(callback), GetModuleHandleW(null()), 0);
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_TIMER {
                on_hold_timer(msg.wParam);
            }
        }
        UnhookWindowsHookEx(hook);
    });
}

pub fn stop() {
    unsafe { PostThreadMessageW(THREAD_ID.load(Ordering::SeqCst), WM_QUIT, 0, 0) };
}

fn post(msg: u32, w: usize, l: isize) {
    unsafe { PostMessageW(HIDDEN.load(Ordering::Relaxed) as HWND, msg, w, l) };
}

fn post_point(msg: u32, p: POINT) {
    post(msg, p.x as isize as usize, p.y as isize);
}

pub fn is_note(hwnd: HWND) -> bool {
    NOTES.lock().map(|n| n.contains(&(hwnd as isize))).unwrap_or(false)
}

unsafe extern "system" fn callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && !PAUSED.load(Ordering::Relaxed) {
        let m = &*(lparam as *const MSLLHOOKSTRUCT);
        if m.dwExtraInfo != MAGIC && HOOK.with(|h| handle(&mut h.borrow_mut(), wparam as u32, m.pt)) {
            return 1;
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

/// Returns true to swallow the event.
fn handle(h: &mut Hook, msg: u32, pt: POINT) -> bool {
    match msg {
        WM_MBUTTONDOWN => {
            if h.state != State::Idle {
                return true;
            }
            let root = root_at(pt);
            if is_note(root) {
                h.eat_middle_up = true;
                post(WM_DELETE_NOTE, root as usize, 0);
                return true;
            }
            if is_ignored(h, root) || root as isize == super::sites::BLOCKED_WINDOW.load(Ordering::Relaxed) {
                return false;
            }
            h.state = State::Pending;
            h.down = pt;
            unsafe {
                KillTimer(null_mut(), h.timer);
                h.timer = SetTimer(null_mut(), 0, HOLD_MS.load(Ordering::Relaxed), None);
            }
            true
        }
        WM_MOUSEMOVE => {
            if h.state == State::Pending && ((pt.x - h.down.x).abs() > 6 || (pt.y - h.down.y).abs() > 6) {
                begin_select(h);
            }
            false
        }
        WM_MBUTTONUP => {
            if h.eat_middle_up {
                h.eat_middle_up = false;
                return true;
            }
            match h.state {
                State::Pending => {
                    h.state = State::Idle;
                    unsafe { KillTimer(null_mut(), h.timer) };
                    if FOLLOW_ARMED.load(Ordering::Relaxed) {
                        post(WM_RELEASE_FOLLOWERS, 0, 0);
                    } else {
                        std::thread::spawn(replay_middle_click);
                    }
                    true
                }
                State::Selecting => {
                    h.state = State::Idle;
                    post_point(WM_SELECT_END, pt);
                    true
                }
                State::Idle => false,
            }
        }
        WM_LBUTTONDOWN => {
            let root = root_at(pt);
            if DROP_ARMED.load(Ordering::Relaxed) && h.state == State::Idle && !is_note(root) && !is_ignored(h, root) {
                h.eat_left_up = true;
                post_point(WM_DROP_CLICK, pt);
                return true;
            }
            false
        }
        WM_LBUTTONUP => std::mem::take(&mut h.eat_left_up),
        _ => false,
    }
}

fn on_hold_timer(id: usize) {
    HOOK.with(|h| {
        let h = &mut *h.borrow_mut();
        if id == h.timer && h.state == State::Pending {
            begin_select(h);
        }
    });
}

fn begin_select(h: &mut Hook) {
    h.state = State::Selecting;
    unsafe { KillTimer(null_mut(), h.timer) };
    post_point(WM_SELECT_BEGIN, h.down);
}

fn root_at(p: POINT) -> HWND {
    unsafe { GetAncestor(WindowFromPoint(p), GA_ROOT) }
}

/// Should this press go straight to the app? Ignored apps, and games unless allowed.
fn is_ignored(h: &mut Hook, hwnd: HWND) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let started = process_start(pid);
    if h.procs.len() > 512 {
        h.procs.clear(); // exited processes would otherwise pile up forever
    }
    let p = h.procs.entry((pid, started)).or_insert_with(|| {
        let path = process_path(pid);
        let known = KNOWN_GAMES.lock().unwrap().as_ref().is_some_and(|k| k.contains(&path.to_lowercase()));
        Proc { name: stem(&path), game: known || is_game_path(&path) }
    });
    if IGNORED.lock().unwrap().iter().any(|a| same_app(a, &p.name)) {
        return true;
    }
    if !BLOCK_GAMES.load(Ordering::Relaxed) || ALLOWED_GAMES.lock().unwrap().iter().any(|a| same_app(a, &p.name)) {
        return false;
    }
    p.game || exclusive_fullscreen(hwnd)
}

/// A DirectX game in exclusive full-screen is in front (and is the window under the mouse).
fn exclusive_fullscreen(hwnd: HWND) -> bool {
    let mut state = 0;
    unsafe {
        SHQueryUserNotificationState(&mut state) == 0
            && state == QUNS_RUNNING_D3D_FULL_SCREEN
            && GetAncestor(GetForegroundWindow(), GA_ROOT) == hwnd
    }
}

fn stem(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn process_name(pid: u32) -> String {
    stem(&process_path(pid))
}

/// Process creation time (FILETIME ticks), 0 if the process can't be opened.
fn process_start(pid: u32) -> u64 {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return 0;
        }
        let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) = zeroed();
        let ok = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(process);
        if ok == 0 {
            return 0;
        }
        (created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64
    }
}

fn process_path(pid: u32) -> String {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(process);
        if ok == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

/// Reads the games Windows has recognized: HKCU\System\GameConfigStore\Children\*\MatchedExeFullPath
pub fn load_known_games() {
    use windows_sys::Win32::System::Registry::*;
    let mut games = HashSet::new();
    unsafe {
        let key: Vec<u16> = r"System\GameConfigStore\Children".encode_utf16().chain(Some(0)).collect();
        let value: Vec<u16> = "MatchedExeFullPath".encode_utf16().chain(Some(0)).collect();
        let mut hkey = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut hkey) == ERROR_SUCCESS {
            for i in 0.. {
                let mut name = [0u16; 256];
                let mut name_len = name.len() as u32;
                if RegEnumKeyExW(hkey, i, name.as_mut_ptr(), &mut name_len, null(), null_mut(), null_mut(), null_mut()) != ERROR_SUCCESS {
                    break;
                }
                let mut buf = [0u16; 1024];
                let mut size = (buf.len() * 2) as u32;
                if RegGetValueW(hkey, name.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr().cast(), &mut size) == ERROR_SUCCESS {
                    let n = (size as usize / 2).saturating_sub(1);
                    games.insert(String::from_utf16_lossy(&buf[..n]).to_lowercase());
                }
            }
            RegCloseKey(hkey);
        }
    }
    *KNOWN_GAMES.lock().unwrap() = Some(games);
}

fn replay_middle_click() {
    unsafe {
        let mut inputs: [INPUT; 2] = zeroed();
        for (input, flag) in inputs.iter_mut().zip([MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP]) {
            input.r#type = INPUT_MOUSE;
            input.Anonymous.mi.dwFlags = flag;
            input.Anonymous.mi.dwExtraInfo = MAGIC;
        }
        SendInput(2, inputs.as_ptr(), size_of::<INPUT>() as i32);
    }
}
