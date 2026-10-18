mod gfx;
mod hook;
mod note;
mod overlay;
mod sites;

use hook::*;
use note::Mode;
use crate::common::{anim, settings::{self, After, Settings}};
use std::{
    cell::RefCell,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::*, Registry::*, Threading::CreateMutexW},
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const WM_TRAY: u32 = WM_APP + 10;
const WM_RELOAD_SETTINGS: u32 = WM_APP + 11;
const FOLLOW_TIMER: usize = 1;
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const HELP: &str = "Hold the wheel and drag to snip (drag right-to-left to add to the notes following you).\n\
                    Wheel-click anywhere: drop following notes. Wheel-click a note: delete it.\n\
                    Hold Shift to freeze notes. Click a note for options.";

struct App {
    settings: Settings,
    notes: Vec<HWND>,
    followers: Vec<HWND>, // newest first, stacked next to the cursor
    icon: HICON,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Set while a popup menu runs its modal loop; following notes freeze so they can be reached.
static MENU_OPEN: AtomicBool = AtomicBool::new(false);
/// Broadcast when Explorer restarts, so the tray icon can be re-added.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(a.borrow_mut().as_mut().expect("app")))
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn hidden() -> HWND {
    HIDDEN.load(Ordering::Relaxed) as HWND
}

pub fn cursor() -> POINT {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    p
}

pub fn main() {
    if std::env::args().any(|a| a == "--probe-url") {
        sites::probe();
        return;
    }
    unsafe {
        let mutex_name = wide("DontForget.SingleInstance");
        let _mutex = CreateMutexW(null(), 1, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        gfx::gdiplus_start();
        apply_menu_theme();

        let inst = GetModuleHandleW(null());
        note::register(inst);
        overlay::register(inst);
        let class = wide("DontForgetMain");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(main_proc),
            hInstance: inst,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassExW(&wc);
        let created = wide("TaskbarCreated");
        TASKBAR_CREATED.store(RegisterWindowMessageW(created.as_ptr()), Ordering::SeqCst);
        let title = wide("Don't Forget");
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), inst, null());
        HIDDEN.store(hwnd as isize, Ordering::SeqCst);

        let settings = Settings::load();
        apply_settings(&settings);
        let first_run = !settings.shown_intro;
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                settings,
                notes: Vec::new(),
                followers: Vec::new(),
                icon: gfx::make_icon(),
            })
        });
        add_tray();
        hook::load_known_games();
        hook::start();
        sites::start();
        if first_run {
            show_help();
            with_app(|a| {
                a.settings.shown_intro = true;
                a.settings.save();
            });
        }

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        hook::stop();
    }
}

fn point(wparam: WPARAM, lparam: LPARAM) -> POINT {
    POINT { x: wparam as isize as i32, y: lparam as i32 }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SELECT_BEGIN => overlay::begin(point(wparam, lparam)),
        WM_SELECT_END => overlay::finish(point(wparam, lparam)),
        WM_DELETE_NOTE => {
            if is_note(wparam as HWND) {
                note::close(wparam as HWND);
            }
        }
        WM_DROP_CLICK => {
            for h in followers() {
                if note::mode(h) == Mode::Drop {
                    note::set_mode(h, Mode::Pinned);
                }
            }
        }
        WM_RELEASE_FOLLOWERS => pin_followers(),
        WM_TIMER if wparam == FOLLOW_TIMER => layout_followers(),
        WM_TRAY => match (lparam & 0xFFFF) as u32 {
            WM_LBUTTONUP => show_help(),
            WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(),
            _ => {}
        },
        WM_SETTINGCHANGE => apply_menu_theme(), // light/dark switch
        WM_RELOAD_SETTINGS => {
            hook::load_known_games();
            let s = Settings::load();
            apply_settings(&s);
            with_app(|a| a.settings = s);
        }
        _ if msg == TASKBAR_CREATED.load(Ordering::Relaxed) => add_tray(), // Explorer restarted
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    }
    0
}

/// Windows 11-style dark popup menus while Windows is in dark mode. Uses the undocumented
/// uxtheme ordinals 135 (SetPreferredAppMode) and 136 (FlushMenuThemes) that Explorer itself
/// uses; if they are missing the menus simply stay light.
fn apply_menu_theme() {
    unsafe {
        let (key, name) = (wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), wide("AppsUseLightTheme"));
        let (mut light, mut size) = (1u32, 4u32);
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, null_mut(), (&mut light as *mut u32).cast(), &mut size);
        let ux = LoadLibraryW(wide("uxtheme.dll").as_ptr());
        if ux.is_null() {
            return;
        }
        if let Some(f) = GetProcAddress(ux, 135 as *const u8) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set_mode(if light == 0 { 2 } else { 3 }); // ForceDark / ForceLight
        }
        if let Some(f) = GetProcAddress(ux, 136 as *const u8) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(f);
            flush();
        }
    }
}

fn apply_settings(s: &Settings) {
    HOLD_MS.store(s.hold_ms, Ordering::Relaxed);
    BLOCK_GAMES.store(s.block_games, Ordering::Relaxed);
    *ALLOWED_GAMES.lock().unwrap() = s.allowed_games.clone();
    *sites::SITES.lock().unwrap() = s.sites.clone();
    *IGNORED.lock().unwrap() = s
        .ignored
        .iter()
        .map(|a| a.to_lowercase().trim_end_matches(".exe").to_string())
        .collect();
}

pub fn add_note(image: gfx::Dib, area: RECT, additive: bool) {
    if !additive {
        pin_followers();
    }
    let edge = if additive { overlay::BLUE } else { overlay::GREEN };
    let hwnd = note::create(image, area.left - note::BORDER, area.top - note::BORDER, edge);
    NOTES.lock().unwrap().push(hwnd as isize);
    let after = with_app(|a| {
        a.notes.push(hwnd);
        a.settings.after
    });
    note::set_mode(
        hwnd,
        match after {
            After::Follow => Mode::Follow,
            After::Drop => Mode::Drop,
            After::Stay => Mode::Pinned,
        },
    );
}

pub fn note_destroyed(hwnd: HWND) {
    NOTES.lock().unwrap().retain(|&h| h != hwnd as isize);
    with_app(|a| {
        a.notes.retain(|&h| h != hwnd);
        a.followers.retain(|&h| h != hwnd);
    });
    update_armed();
}

pub fn on_mode_changed(hwnd: HWND, mode: Mode) {
    with_app(|a| {
        a.followers.retain(|&h| h != hwnd);
        if mode != Mode::Pinned {
            a.followers.insert(0, hwnd);
        }
    });
    // Only re-stack when a note joins; pinning several at once must leave each where it is.
    if mode != Mode::Pinned {
        layout_followers();
    }
    update_armed();
}

fn followers() -> Vec<HWND> {
    with_app(|a| a.followers.clone())
}

fn pin_followers() {
    for h in followers() {
        note::set_mode(h, Mode::Pinned);
    }
}

pub fn clear_all() {
    for h in with_app(|a| a.notes.clone()) {
        note::close(h);
    }
}

fn update_armed() {
    let list = followers();
    DROP_ARMED.store(list.iter().any(|&h| note::mode(h) == Mode::Drop), Ordering::Relaxed);
    FOLLOW_ARMED.store(!list.is_empty(), Ordering::Relaxed);
    unsafe {
        if list.is_empty() {
            KillTimer(hidden(), FOLLOW_TIMER);
        } else {
            SetTimer(hidden(), FOLLOW_TIMER, 15, None);
        }
    }
}

/// Stacks following notes below-right of the cursor, flipping at screen edges. Each note
/// glides a fraction of the way there per frame, so a fresh snip flies over to the cursor.
/// Holding Shift or having a menu open freezes them so they can be reached.
fn layout_followers() {
    let list = followers();
    if list.is_empty() {
        return;
    }
    let freeze = MENU_OPEN.load(Ordering::Relaxed) || unsafe { GetAsyncKeyState(VK_SHIFT as i32) } < 0;
    for &h in &list {
        note::set_click_through(h, !freeze);
    }
    if freeze {
        return;
    }
    unsafe {
        const GAP: i32 = 6;
        let rects: Vec<RECT> = list
            .iter()
            .map(|&h| {
                let mut r: RECT = zeroed();
                GetWindowRect(h, &mut r);
                r
            })
            .collect();
        let sizes: Vec<(i32, i32)> = rects.iter().map(|r| (r.right - r.left, r.bottom - r.top)).collect();
        let width = sizes.iter().map(|s| s.0).max().unwrap_or(0);
        let height = sizes.iter().map(|s| s.1).sum::<i32>() + GAP * (sizes.len() as i32 - 1);
        let c = cursor();
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromPoint(c, MONITOR_DEFAULTTONEAREST), &mut mi);
        let wa = mi.rcWork;
        let (x, mut y) = anim::stack_origin((c.x, c.y), (width, height), (wa.left, wa.top, wa.right, wa.bottom));
        for ((&h, &(_, nh)), r) in list.iter().zip(&sizes).zip(&rects) {
            let (nx, ny) = (anim::glide(r.left, x), anim::glide(r.top, y));
            if nx != r.left || ny != r.top {
                SetWindowPos(h, null_mut(), nx, ny, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
            y += nh + GAP;
        }
    }
}

/// Shows a popup menu at the cursor and returns the chosen command id (0 if dismissed).
pub fn track_menu(menu: HMENU) -> usize {
    unsafe {
        MENU_OPEN.store(true, Ordering::Relaxed);
        let p = cursor();
        SetForegroundWindow(hidden());
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, p.x, p.y, 0, hidden(), null());
        PostMessageW(hidden(), WM_NULL, 0, 0);
        MENU_OPEN.store(false, Ordering::Relaxed);
        cmd as usize
    }
}

fn tray_data() -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hidden();
    nid.uID = 1;
    nid
}

fn copy_into(dst: &mut [u16], s: &str) {
    let max = dst.len() - 1;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16().take(max).chain(Some(0))) {
        *d = c;
    }
}

fn add_tray() {
    let mut nid = tray_data();
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = with_app(|a| a.icon);
    copy_into(&mut nid.szTip, "Don't Forget");
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
}

fn show_help() {
    let mut nid = tray_data();
    nid.uFlags = NIF_INFO;
    copy_into(&mut nid.szInfoTitle, "Don't Forget");
    copy_into(&mut nid.szInfo, HELP);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

fn tray_menu() {
    const HOLDS: [u32; 4] = [150, 250, 400, 600];
    let (after, hold) = with_app(|a| (a.settings.after, a.settings.hold_ms));
    let startup = starts_with_windows();
    let cmd = unsafe {
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let add = |m: HMENU, flags: u32, id: usize, text: &str| {
            let t = wide(text);
            AppendMenuW(m, MF_STRING | flags, id, t.as_ptr());
        };
        let m = CreatePopupMenu();
        let a = CreatePopupMenu();
        add(a, check(after == After::Follow), 100, "Follow my mouse");
        add(a, check(after == After::Drop), 101, "Follow until I click to place it");
        add(a, check(after == After::Stay), 102, "Stay where I snipped it");
        add(m, MF_POPUP, a as usize, "After a snip");
        let h = CreatePopupMenu();
        for (i, ms) in HOLDS.iter().enumerate() {
            add(h, check(*ms == hold), 110 + i, &format!("{ms} ms"));
        }
        add(m, MF_POPUP, h as usize, "Hold time before snipping");
        add(m, 0, 120, "Ignored apps…");
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        add(m, 0, 130, "Delete all notes");
        add(m, check(PAUSED.load(Ordering::Relaxed)), 131, "Pause");
        add(m, check(startup), 132, "Start with Windows");
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        add(m, 0, 140, "How to use");
        add(m, 0, 150, "Exit");
        let cmd = track_menu(m);
        DestroyMenu(m);
        cmd
    };
    match cmd {
        100..=102 => with_app(|a| {
            a.settings.after = [After::Follow, After::Drop, After::Stay][cmd - 100];
            a.settings.save();
        }),
        110..=113 => {
            with_app(|a| {
                a.settings.hold_ms = HOLDS[cmd - 110];
                a.settings.save();
            });
            HOLD_MS.store(HOLDS[cmd - 110], Ordering::Relaxed);
        }
        120 => edit_settings(),
        130 => clear_all(),
        131 => {
            PAUSED.fetch_xor(true, Ordering::Relaxed);
        }
        132 => set_starts_with_windows(!startup),
        140 => show_help(),
        150 => exit(),
        _ => {}
    }
}

/// Opens settings.ini in Notepad and reloads it when Notepad closes.
fn edit_settings() {
    with_app(|a| a.settings.save());
    std::thread::spawn(|| {
        let _ = std::process::Command::new("notepad.exe").arg(settings::path()).status();
        unsafe { PostMessageW(hidden(), WM_RELOAD_SETTINGS, 0, 0) };
    });
}

fn starts_with_windows() -> bool {
    let (key, name) = (wide(RUN_KEY), wide("DontForget"));
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), null_mut())
            == ERROR_SUCCESS
    }
}

fn set_starts_with_windows(on: bool) {
    let (key, name) = (wide(RUN_KEY), wide("DontForget"));
    unsafe {
        if on {
            let exe = std::env::current_exe().unwrap_or_default();
            let value = wide(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32);
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr());
        }
    }
}

fn exit() {
    clear_all();
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &tray_data());
        PostQuitMessage(0);
    }
}
