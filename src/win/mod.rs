mod gfx;
mod hook;
mod note;
mod overlay;
mod sites;
mod store;

use hook::*;
use note::{Kind, Mode, Saved};
use crate::common::{anim, settings::{self, After, Outline, Settings, PRESETS}};
use std::{
    cell::RefCell,
    collections::HashSet,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::*, Registry::*, Threading::{CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize}},
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const WM_TRAY: u32 = WM_APP + 10;
/// Re-read settings.ini; the control panel posts this after it saves.
const WM_RELOAD_SETTINGS: u32 = WM_APP + 11;
const FOLLOW_TIMER: usize = 1;
const SAVE_TIMER: usize = 2; // notes.ini is written a moment after the last change
const DISPLAY_TIMER: usize = 3; // screens settle a moment after a display change
const SNIP_TIMER: usize = 4; // lets the tray menu fade before the screen is captured
const HOTKEY_SNIP: i32 = 1;
const HOTKEY_HIDE: i32 = 2;
const HOTKEY_RESTORE: i32 = 3;
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const REPO: &str = "https://github.com/BeForce1/dont-forget";

struct App {
    settings: Settings,
    notes: Vec<HWND>,
    followers: Vec<HWND>, // newest first, stacked next to the cursor
    /// Deleted notes for undo, oldest group first; each group is what one delete removed.
    trash: Vec<Vec<Saved>>,
    hidden: bool,
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
        let keep = settings.keep_notes;
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                settings,
                notes: Vec::new(),
                followers: Vec::new(),
                trash: Vec::new(),
                hidden: false,
                icon: gfx::make_icon(),
            })
        });
        add_tray();
        if keep {
            restore_saved();
        } else {
            store::wipe();
        }
        hook::load_known_games();
        hook::start();
        sites::start();
        let taken = register_hotkeys();
        trim_memory();
        if first_run {
            show_help();
            with_app(|a| {
                a.settings.shown_intro = true;
                a.settings.save();
            });
        } else if !taken.is_empty() {
            balloon(&format!("{} is already used by another app. Pick another shortcut in the settings file.", taken.join(", ")));
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
        WM_SELECT_BEGIN => overlay::begin(point(wparam, lparam), false),
        WM_SELECT_END => overlay::finish(point(wparam, lparam)),
        WM_DELETE_NOTE => {
            if is_note(wparam as HWND) {
                delete_notes(vec![wparam as HWND]);
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
        WM_TIMER if wparam == SAVE_TIMER => save_now(),
        WM_TIMER if wparam == DISPLAY_TIMER => {
            KillTimer(hwnd, DISPLAY_TIMER);
            for h in with_app(|a| a.notes.clone()) {
                if note::mode(h) == Mode::Pinned {
                    keep_on_screen(h);
                }
            }
            notes_changed();
        }
        WM_TIMER if wparam == SNIP_TIMER => {
            KillTimer(hwnd, SNIP_TIMER);
            overlay::begin(cursor(), true);
        }
        WM_DISPLAYCHANGE => {
            SetTimer(hwnd, DISPLAY_TIMER, 1000, None);
        }
        WM_HOTKEY => match wparam as i32 {
            HOTKEY_SNIP if !PAUSED.load(Ordering::Relaxed) => overlay::begin(cursor(), true),
            HOTKEY_HIDE => set_hidden(!with_app(|a| a.hidden)),
            HOTKEY_RESTORE => restore_deleted(),
            _ => {}
        },
        WM_CLOSE => exit(), // the control panel's Stop
        WM_QUERYENDSESSION => return 1,
        WM_ENDSESSION if wparam != 0 => save_now(), // signing out or shutting down
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
            register_hotkeys();
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
    note::set_style(note::Style { snip: s.snip_color, add: s.add_color, outline: s.outline, width: s.outline_width });
    for h in NOTES.lock().unwrap().clone() {
        note::restyle(h as HWND);
    }
    HOLD_MS.store(s.hold_ms, Ordering::Relaxed);
    BLOCK_GAMES.store(s.block_games, Ordering::Relaxed);
    *ALLOWED_GAMES.lock().unwrap() = s.allowed_games.clone();
    *sites::SITES.lock().unwrap() = s.sites.clone();
    sites::wake();
    *IGNORED.lock().unwrap() = s
        .ignored
        .iter()
        .map(|a| a.to_lowercase().trim_end_matches(".exe").to_string())
        .collect();
}

/// "Ctrl+Alt+S" -> (MOD_* flags, virtual key). Keys: letters, digits, F1-F24.
fn parse_hotkey(text: &str) -> Option<(u32, u32)> {
    let (mut mods, mut key) = (0, None);
    for part in text.split('+').map(|p| p.trim().to_ascii_uppercase()) {
        match part.as_str() {
            "CTRL" | "CONTROL" => mods |= MOD_CONTROL,
            "ALT" => mods |= MOD_ALT,
            "SHIFT" => mods |= MOD_SHIFT,
            "WIN" => mods |= MOD_WIN,
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => key = Some(k.as_bytes()[0] as u32),
            k if k.starts_with('F') => match k[1..].parse::<u32>() {
                Ok(n @ 1..=24) => key = Some(VK_F1 as u32 + n - 1),
                _ => return None,
            },
            _ => return None,
        }
    }
    Some((mods, key?))
}

/// (Re-)registers the shortcuts from the settings; returns the ones another app already holds.
fn register_hotkeys() -> Vec<String> {
    let keys = with_app(|a| [(HOTKEY_SNIP, a.settings.snip_key.clone()), (HOTKEY_HIDE, a.settings.hide_key.clone()), (HOTKEY_RESTORE, a.settings.restore_key.clone())]);
    let mut taken = Vec::new();
    for (id, text) in keys {
        unsafe { UnregisterHotKey(hidden(), id) };
        let Some((mods, vk)) = parse_hotkey(&text) else { continue };
        if unsafe { RegisterHotKey(hidden(), id, mods | MOD_NOREPEAT, vk) } == 0 {
            taken.push(text);
        }
    }
    taken
}

pub fn add_note(image: gfx::Dib, area: RECT, additive: bool) {
    if !additive {
        pin_followers();
    }
    set_hidden(false);
    if with_app(|a| a.settings.copy_snips) {
        note::copy_image(&image);
    }
    let id = store::new_id();
    // written right away, so it survives a crash and can come back after a delete
    let _ = std::fs::create_dir_all(store::dir());
    image.save_png(&store::image(id));
    let kind = if additive { Kind::Add } else { Kind::Snip };
    let hwnd = note::create(image, area.left, area.top, kind, id, 1.0, 100);
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
    notes_changed();
}

/// Opens a saved note again, pinned where it was.
fn open_saved(s: &Saved) -> Option<HWND> {
    let image = gfx::Dib::load_png(&store::image(s.id))?;
    let hwnd = note::create(image, s.x, s.y, s.kind, s.id, s.scale, s.opacity);
    NOTES.lock().unwrap().push(hwnd as isize);
    with_app(|a| a.notes.push(hwnd));
    keep_on_screen(hwnd);
    Some(hwnd)
}

/// Brings back the notes that were open when the app last stopped, and the undo list.
fn restore_saved() {
    let (open, trash) = store::read();
    let mut keep: HashSet<u64> = trash.iter().flatten().map(|s| s.id).collect();
    for s in &open {
        if open_saved(s).is_some() {
            keep.insert(s.id);
        }
    }
    with_app(|a| a.trash = trash);
    store::remove_orphans(&keep);
    save_now();
}

/// Deletes notes, keeping them (as one undo step) so the restore shortcut can bring them back.
pub fn delete_notes(list: Vec<HWND>) {
    let group: Vec<Saved> = list.iter().filter(|&&h| !note::is_closing(h)).filter_map(|&h| note::saved(h)).collect();
    if group.is_empty() {
        return;
    }
    with_app(|a| {
        a.trash.push(group);
        // forget the oldest deletions beyond the limit, with their images
        while a.trash.iter().map(Vec::len).sum::<usize>() > store::MAX_TRASH && a.trash.len() > 1 {
            for s in a.trash.remove(0) {
                store::remove_image(s.id);
            }
        }
    });
    for h in list {
        note::close(h);
    }
    notes_changed();
}

/// Undo: reopens the most recently deleted note (or everything "Delete all notes" removed).
fn restore_deleted() {
    let Some(group) = with_app(|a| a.trash.pop()) else { return };
    set_hidden(false);
    for s in &group {
        if let Some(h) = open_saved(s) {
            note::set_mode(h, Mode::Pinned);
        }
    }
    notes_changed();
}

/// Something about the notes changed; write notes.ini shortly (many changes, one write).
pub fn notes_changed() {
    unsafe { SetTimer(hidden(), SAVE_TIMER, 400, None) };
}

fn save_now() {
    unsafe { KillTimer(hidden(), SAVE_TIMER) };
    let (notes, trash) = with_app(|a| (a.notes.clone(), a.trash.clone()));
    let open: Vec<Saved> = notes.iter().filter(|&&h| !note::is_closing(h)).filter_map(|&h| note::saved(h)).collect();
    store::write(&open, &trash);
}

/// Moves a note back onto a screen when (almost) none of it is visible, e.g. after unplugging a monitor.
fn keep_on_screen(h: HWND) {
    let Some(r) = note::full_rect(h) else { return };
    unsafe {
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST), &mut mi);
        let (w, hgt) = (r.right - r.left, r.bottom - r.top);
        let shown = gfx::intersect(&r, &mi.rcMonitor).map_or(0, |i| (i.right - i.left).min(48) * (i.bottom - i.top).min(48));
        if shown >= w.min(48) * hgt.min(48) {
            return;
        }
        let wa = mi.rcWork;
        let x = r.left.clamp(wa.left, (wa.right - w).max(wa.left));
        let y = r.top.clamp(wa.top, (wa.bottom - hgt).max(wa.top));
        note::move_full_to(h, x, y);
    }
}

/// Hides every note without deleting it (screen sharing, or one is in the way), or shows them again.
fn set_hidden(hide: bool) {
    let (was, notes) = with_app(|a| (std::mem::replace(&mut a.hidden, hide), a.notes.clone()));
    if was == hide {
        return;
    }
    for h in &notes {
        unsafe { ShowWindow(*h, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE }) };
    }
    update_armed();
    update_tip();
}

pub fn note_destroyed(hwnd: HWND) {
    NOTES.lock().unwrap().retain(|&h| h != hwnd as isize);
    with_app(|a| {
        a.notes.retain(|&h| h != hwnd);
        a.followers.retain(|&h| h != hwnd);
    });
    update_armed();
    if NOTES.lock().unwrap().is_empty() {
        trim_memory();
    }
}

/// Hands pages back to Windows while idle; touched again, they come back from RAM in microseconds.
pub fn trim_memory() {
    unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
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
    delete_notes(with_app(|a| a.notes.clone()));
}

fn update_armed() {
    // hidden notes neither follow nor catch clicks
    let list = if with_app(|a| a.hidden) { Vec::new() } else { followers() };
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
    copy_into(&mut nid.szTip, &tip());
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
}

fn tip() -> String {
    let (hidden, count) = with_app(|a| (a.hidden, a.notes.len()));
    if hidden {
        format!("Don't Forget: {count} note{} hidden", if count == 1 { "" } else { "s" })
    } else {
        "Don't Forget".into()
    }
}

fn update_tip() {
    let mut nid = tray_data();
    nid.uFlags = NIF_TIP;
    copy_into(&mut nid.szTip, &tip());
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

fn balloon(text: &str) {
    let mut nid = tray_data();
    nid.uFlags = NIF_INFO;
    copy_into(&mut nid.szInfoTitle, "Don't Forget");
    copy_into(&mut nid.szInfo, text);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

fn show_help() {
    let (snip, restore) = with_app(|a| (a.settings.snip_key.clone(), a.settings.restore_key.clone()));
    let keyboard = if snip.is_empty() { String::new() } else { format!(", or press {snip} and drag") };
    let undo = if restore.is_empty() { String::new() } else { format!(" ({restore} brings it back)") };
    balloon(&format!(
        "Hold the wheel and drag to snip{keyboard}. Right-to-left adds to the notes following you.\n\
         Wheel-click: drop notes. On a note: delete it{undo}.\n\
         Shift freezes notes. Click a note for options."
    ));
}

/// Menu text with its shortcut right-aligned.
fn with_key(text: &str, key: &str) -> String {
    if key.is_empty() { text.into() } else { format!("{text}\t{key}") }
}

fn open_url(url: &str) {
    let (verb, url) = (wide("open"), wide(url));
    unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), url.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

fn tray_menu() {
    const HOLDS: [u32; 4] = [150, 250, 400, 600];
    const WIDTHS: [(u32, &str); 3] = [(1, "Thin"), (2, "Normal"), (4, "Thick")];
    const OUTLINES: [(Outline, &str); 3] = [(Outline::Solid, "Solid"), (Outline::Dashed, "Dashed"), (Outline::Dotted, "Dotted")];
    let s = with_app(|a| a.settings.clone());
    let (hidden_now, can_restore, has_notes) = with_app(|a| (a.hidden, !a.trash.is_empty(), !a.notes.is_empty()));
    let startup = starts_with_windows();
    let cmd = unsafe {
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let add = |m: HMENU, flags: u32, id: usize, text: &str| {
            let t = wide(text);
            AppendMenuW(m, MF_STRING | flags, id, t.as_ptr());
        };
        let m = CreatePopupMenu();
        add(m, 0, 160, &with_key("Snip now", &s.snip_key));
        add(m, check(hidden_now) | if has_notes || hidden_now { 0 } else { MF_GRAYED }, 161, &with_key("Hide notes", &s.hide_key));
        add(m, if can_restore { 0 } else { MF_GRAYED }, 162, &with_key("Restore deleted note", &s.restore_key));
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        let a = CreatePopupMenu();
        add(a, check(s.after == After::Follow), 100, "Follow my mouse");
        add(a, check(s.after == After::Drop), 101, "Follow until I click to place it");
        add(a, check(s.after == After::Stay), 102, "Stay where I snipped it");
        add(m, MF_POPUP, a as usize, "After a snip");
        let h = CreatePopupMenu();
        for (i, ms) in HOLDS.iter().enumerate() {
            add(h, check(*ms == s.hold_ms), 110 + i, &format!("{ms} ms"));
        }
        add(m, MF_POPUP, h as usize, "Hold time before snipping");
        let look = CreatePopupMenu();
        for (i, (name, snip, plus)) in PRESETS.iter().enumerate() {
            add(look, check(s.snip_color == *snip && s.add_color == *plus), 170 + i, name);
        }
        AppendMenuW(look, MF_SEPARATOR, 0, null());
        for (i, (o, name)) in OUTLINES.iter().enumerate() {
            add(look, check(s.outline == *o), 180 + i, name);
        }
        AppendMenuW(look, MF_SEPARATOR, 0, null());
        for (i, (w, name)) in WIDTHS.iter().enumerate() {
            add(look, check(s.outline_width == *w), 190 + i, name);
        }
        add(m, MF_POPUP, look as usize, "Look");
        add(m, 0, 120, "Edit settings file…");
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        add(m, if has_notes { 0 } else { MF_GRAYED }, 130, "Delete all notes");
        add(m, check(s.keep_notes), 133, "Keep notes after restart");
        add(m, check(PAUSED.load(Ordering::Relaxed)), 131, "Pause");
        add(m, check(startup), 132, "Start with Windows");
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        add(m, 0, 140, "How to use");
        add(m, 0, 141, &format!("About Don't Forget {}", env!("CARGO_PKG_VERSION")));
        add(m, 0, 150, "Exit");
        let cmd = track_menu(m);
        DestroyMenu(m);
        cmd
    };
    let change = |f: &dyn Fn(&mut Settings)| {
        let s = with_app(|a| {
            f(&mut a.settings);
            a.settings.save();
            a.settings.clone()
        });
        apply_settings(&s);
    };
    match cmd {
        100..=102 => change(&|s| s.after = [After::Follow, After::Drop, After::Stay][cmd - 100]),
        110..=113 => change(&|s| s.hold_ms = HOLDS[cmd - 110]),
        120 => edit_settings(),
        130 => clear_all(),
        131 => {
            PAUSED.fetch_xor(true, Ordering::Relaxed);
        }
        132 => set_starts_with_windows(!startup),
        133 => {
            change(&|s| s.keep_notes = !s.keep_notes);
            save_now();
        }
        140 => show_help(),
        141 => open_url(REPO),
        150 => exit(),
        160 => unsafe {
            SetTimer(hidden(), SNIP_TIMER, 150, None);
        },
        161 => set_hidden(!hidden_now),
        162 => restore_deleted(),
        170..=173 => change(&|s| (s.snip_color, s.add_color) = (PRESETS[cmd - 170].1, PRESETS[cmd - 170].2)),
        180..=182 => change(&|s| s.outline = OUTLINES[cmd - 180].0),
        190..=192 => change(&|s| s.outline_width = WIDTHS[cmd - 190].0),
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

/// Quits, leaving the notes saved for next time (or wiping them when they aren't kept).
fn exit() {
    save_now();
    if !with_app(|a| a.settings.keep_notes) {
        store::wipe();
    }
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &tray_data());
        PostQuitMessage(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys() {
        assert_eq!(parse_hotkey("Ctrl+Alt+S"), Some((MOD_CONTROL | MOD_ALT, b'S' as u32)));
        assert_eq!(parse_hotkey("shift + f12"), Some((MOD_SHIFT, VK_F12 as u32)));
        assert_eq!(parse_hotkey("Win+1"), Some((MOD_WIN, b'1' as u32)));
        assert_eq!(parse_hotkey("Ctrl+Alt"), None);
        assert_eq!(parse_hotkey("Ctrl+Space"), None);
        assert_eq!(parse_hotkey(""), None);
    }
}
