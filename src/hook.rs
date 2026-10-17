// Global low-level mouse hook on its own thread so a busy UI thread never stalls the system mouse.
// Wheel-button press: quick click -> replayed to the app (or releases following notes),
// hold or drag -> screen selection. Results are posted to the hidden main window.
use std::{
    cell::RefCell,
    collections::HashMap,
    mem::{size_of, zeroed},
    path::Path,
    ptr::{null, null_mut},
    sync::{atomic::*, Mutex},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
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
    names: HashMap<u32, String>,
}

thread_local! {
    static HOOK: RefCell<Hook> = RefCell::new(Hook {
        state: State::Idle,
        down: POINT { x: 0, y: 0 },
        eat_middle_up: false,
        eat_left_up: false,
        timer: 0,
        names: HashMap::new(),
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
            if is_ignored(h, root) {
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
            if DROP_ARMED.load(Ordering::Relaxed) && h.state == State::Idle && !is_note(root_at(pt)) {
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

fn is_ignored(h: &mut Hook, hwnd: HWND) -> bool {
    let ignored = IGNORED.lock().unwrap();
    if ignored.is_empty() {
        return false;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let name = h.names.entry(pid).or_insert_with(|| process_name(pid));
    ignored.iter().any(|a| a == name)
}

fn process_name(pid: u32) -> String {
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
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&full).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
    }
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
