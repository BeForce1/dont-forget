// One floating note: a snipped piece of the screen that is pinned somewhere or rides next to the cursor.
use crate::{gfx::*, wide};
use std::{cell::Cell, mem::{size_of, zeroed}, ptr::{null, null_mut}};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{DataExchange::*, Memory::*},
    UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub const BORDER: i32 = 2;
/// Corner radius of the screenshot inside the frame.
const IMAGE_RADIUS: i32 = 6;
const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: &str = "DontForgetNote";
const OPACITIES: [u32; 5] = [100, 85, 70, 50, 30];
const SIZES: [u32; 5] = [50, 75, 100, 150, 200];

// Animation timing. The timer only runs while something is moving.
const ANIM_TIMER: usize = 1;
const FRAME_MS: f32 = 15.0;
const APPEAR_MS: f32 = 260.0;
const CLOSE_MS: f32 = 170.0;
const PULSE_MS: f32 = 280.0;
const BUTTON_MS: f32 = 130.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Pinned,
    Follow,
    Drop,
}

struct Note {
    image: Dib,
    edge: (u32, u32), // (GDI+ ARGB, GDI COLORREF)
    scale: Cell<f32>,
    opacity: Cell<u32>,
    mode: Cell<Mode>,
    hover: Cell<bool>,
    close_hot: Cell<bool>,
    click_through: Cell<bool>,
    left_down: Cell<bool>,
    dragged: Cell<bool>,
    down_screen: Cell<POINT>,
    down_origin: Cell<POINT>,
    // animation progress, each 0..=1
    appear: Cell<f32>,
    closing: Cell<Option<f32>>,
    pulse: Cell<f32>,
    button: Cell<f32>,
    button_hot: Cell<f32>,
    animating: Cell<bool>,
}

fn get(hwnd: HWND) -> Option<&'static Note> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Note).as_ref() }
}

pub fn mode(hwnd: HWND) -> Mode {
    get(hwnd).map_or(Mode::Pinned, |n| n.mode.get())
}

pub fn register(inst: HINSTANCE) {
    unsafe {
        let class = wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassExW(&wc);
    }
}

pub fn create(image: Dib, x: i32, y: i32, edge: (u32, u32)) -> HWND {
    let (w, h) = (image.w + 2 * BORDER, image.h + 2 * BORDER);
    let origin = POINT { x: 0, y: 0 };
    let note = Box::new(Note {
        image,
        edge,
        scale: Cell::new(1.0),
        opacity: Cell::new(100),
        mode: Cell::new(Mode::Pinned),
        hover: Cell::new(false),
        close_hot: Cell::new(false),
        click_through: Cell::new(false),
        left_down: Cell::new(false),
        dragged: Cell::new(false),
        down_screen: Cell::new(origin),
        down_origin: Cell::new(origin),
        appear: Cell::new(0.0),
        closing: Cell::new(None),
        pulse: Cell::new(1.0),
        button: Cell::new(0.0),
        button_hot: Cell::new(0.0),
        animating: Cell::new(false),
    });
    unsafe {
        let (class, title) = (wide(CLASS), wide("Don't Forget note"));
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_LAYERED,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            x, y, w, h,
            null_mut(), null_mut(), null_mut(), null(),
        );
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(note) as isize);
        apply_style(hwnd); // starts fully transparent; the pop-in animation fades it in
        // Windows 11 rounded corners with a matching border that follows the curve.
        let round = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&round as *const i32).cast(), 4);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, (&edge.1 as *const u32).cast(), 4);
        apply_geometry(hwnd, true);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        animate(hwnd);
        hwnd
    }
}

pub fn set_mode(hwnd: HWND, mode: Mode) {
    let Some(n) = get(hwnd) else { return };
    let was = n.mode.replace(mode);
    if mode == Mode::Pinned && was != Mode::Pinned {
        n.pulse.set(0.0); // little "thunk" when it lands
        animate(hwnd);
    }
    n.click_through.set(mode != Mode::Pinned);
    apply_style(hwnd);
    crate::on_mode_changed(hwnd, mode);
}

/// Fades and shrinks the note away, then destroys it.
pub fn close(hwnd: HWND) {
    let Some(n) = get(hwnd) else { return };
    if n.closing.get().is_some() {
        return;
    }
    n.closing.set(Some(0.0));
    n.click_through.set(true);
    apply_style(hwnd);
    crate::note_destroyed(hwnd); // stop following and stop counting it right away
    animate(hwnd);
}

/// Following notes let clicks pass through, except while frozen (Shift held or a menu open).
pub fn set_click_through(hwnd: HWND, on: bool) {
    if let Some(n) = get(hwnd) {
        if n.closing.get().is_none() && n.click_through.replace(on) != on {
            apply_style(hwnd);
        }
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_out_back(t: f32) -> f32 {
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// Current animated (scale, alpha) multipliers.
fn anim_values(n: &Note) -> (f32, f32) {
    let a = n.appear.get();
    let mut scale = 0.8 + 0.2 * ease_out_back(a);
    let mut alpha = ease_out_cubic((a * 1.8).min(1.0));
    if let Some(c) = n.closing.get() {
        scale *= 1.0 - 0.15 * c * c;
        alpha *= 1.0 - ease_out_cubic(c);
    }
    scale *= 1.0 + 0.06 * (n.pulse.get() * std::f32::consts::PI).sin();
    (scale, alpha)
}

fn animate(hwnd: HWND) {
    if let Some(n) = get(hwnd) {
        if !n.animating.replace(true) {
            unsafe { SetTimer(hwnd, ANIM_TIMER, FRAME_MS as u32, None) };
        }
    }
}

fn step(cell: &Cell<f32>, target: f32, ms: f32) -> bool {
    let v = cell.get();
    if v == target {
        return false;
    }
    let d = FRAME_MS / ms;
    cell.set(if v < target { (v + d).min(target) } else { (v - d).max(target) });
    true
}

fn tick(hwnd: HWND, n: &Note) {
    let mut moved = step(&n.appear, 1.0, APPEAR_MS);
    moved |= step(&n.pulse, 1.0, PULSE_MS);
    if let Some(c) = n.closing.get() {
        let c = (c + FRAME_MS / CLOSE_MS).min(1.0);
        n.closing.set(Some(c));
        if c >= 1.0 {
            unsafe { DestroyWindow(hwnd) };
            return;
        }
        moved = true;
    }
    let mut repaint = step(&n.button, if n.hover.get() && n.closing.get().is_none() { 1.0 } else { 0.0 }, BUTTON_MS);
    repaint |= step(&n.button_hot, if n.close_hot.get() { 1.0 } else { 0.0 }, BUTTON_MS);
    if moved {
        apply_geometry(hwnd, true);
        apply_style(hwnd);
    }
    if repaint {
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
    if !moved && !repaint {
        n.animating.set(false);
        unsafe { KillTimer(hwnd, ANIM_TIMER) };
    }
}

fn apply_style(hwnd: HWND) {
    let Some(n) = get(hwnd) else { return };
    let alpha = anim_values(n).1 * n.opacity.get() as f32 / 100.0;
    unsafe {
        let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 | WS_EX_LAYERED;
        if n.click_through.get() { ex |= WS_EX_TRANSPARENT } else { ex &= !WS_EX_TRANSPARENT }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
        SetLayeredWindowAttributes(hwnd, 0, (alpha * 255.0).round() as u8, LWA_ALPHA);
    }
}

/// Sizes the window from the note's zoom and the current animation scale,
/// keeping either its center (animations) or its top-left corner (zooming) in place.
fn apply_geometry(hwnd: HWND, keep_center: bool) {
    let Some(n) = get(hwnd) else { return };
    let s = n.scale.get() * anim_values(n).0;
    let w = (n.image.w as f32 * s).round().max(1.0) as i32 + 2 * BORDER;
    let h = (n.image.h as f32 * s).round().max(1.0) as i32 + 2 * BORDER;
    unsafe {
        let mut r: RECT = zeroed();
        GetWindowRect(hwnd, &mut r);
        let (cw, ch) = (r.right - r.left, r.bottom - r.top);
        if cw == w && ch == h {
            return;
        }
        let (x, y) = if keep_center { (r.left + (cw - w) / 2, r.top + (ch - h) / 2) } else { (r.left, r.top) };
        SetWindowPos(hwnd, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        InvalidateRect(hwnd, null(), 0);
    }
}

fn set_scale(hwnd: HWND, scale: f32) {
    let Some(n) = get(hwnd) else { return };
    n.scale.set(scale.clamp(0.2, 4.0));
    apply_geometry(hwnd, false);
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(hwnd, &mut rc) };
    (rc.right, rc.bottom)
}

fn dpi_scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}

/// The close button: a rounded square, inset the same distance from the top and right edges.
fn close_rect(hwnd: HWND) -> RECT {
    let k = dpi_scale(hwnd);
    let (size, margin) = ((22.0 * k).round() as i32, (BORDER as f32 + 6.0 * k).round() as i32);
    rect(client_size(hwnd).0 - margin - size, margin, size, size)
}

fn has_room_for_close(hwnd: HWND) -> bool {
    let (w, h) = client_size(hwnd);
    let r = close_rect(hwnd);
    w > (r.right - r.left) * 2 + 24 && h > (r.bottom - r.top) + 16
}

fn over_close(hwnd: HWND, x: i32, y: i32) -> bool {
    let r = close_rect(hwnd);
    has_room_for_close(hwnd) && x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Blends two 0xAARRGGBB colors and multiplies the result's alpha by `fade`.
fn mix(a: u32, b: u32, t: f32, fade: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        x + (y - x) * t
    };
    let alpha = (ch(24) * fade).round() as u32;
    alpha << 24 | (ch(16).round() as u32) << 16 | (ch(8).round() as u32) << 8 | ch(0).round() as u32
}

fn paint(hwnd: HWND, n: &Note) {
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let (cw, ch) = client_size(hwnd);
        let back = Dib::new(cw, ch);
        let bdc = MemDc::new(back.bmp);
        let brush = CreateSolidBrush(n.edge.1);
        FillRect(bdc.dc, &rect(0, 0, cw, ch), brush);
        DeleteObject(brush);

        let src = MemDc::new(n.image.bmp);
        let (iw, ih) = (cw - 2 * BORDER, ch - 2 * BORDER);
        if iw == n.image.w && ih == n.image.h {
            BitBlt(bdc.dc, BORDER, BORDER, iw, ih, src.dc, 0, 0, SRCCOPY);
        } else {
            SetStretchBltMode(bdc.dc, HALFTONE);
            SetBrushOrgEx(bdc.dc, 0, 0, null_mut());
            StretchBlt(bdc.dc, BORDER, BORDER, iw, ih, src.dc, 0, 0, n.image.w, n.image.h, SRCCOPY);
        }
        drop(src);

        let g = Gfx::new(bdc.dc);
        g.round_corners(n.edge.0, BORDER, BORDER, iw, ih, IMAGE_RADIUS);

        let shown = ease_out_cubic(n.button.get());
        if shown > 0.01 && has_room_for_close(hwnd) {
            let r = close_rect(hwnd);
            let k = dpi_scale(hwnd);
            let full = (r.right - r.left) as f32;
            // grows in from 80% around its own center
            let size = full * (0.8 + 0.2 * shown);
            let (cx, cy) = (r.left as f32 + full / 2.0, r.top as f32 + full / 2.0);
            let (x, y) = (cx - size / 2.0, cy - size / 2.0);
            let hot = ease_out_cubic(n.button_hot.get());
            g.fill_round_rect_f(mix(0xE0232326, 0xF5E5484D, hot, shown), x, y, size, size, 6.0 * k);
            g.round_rect_f(mix(0x55FFFFFF, 0x00FFFFFF, hot, shown), 1.0, x + 0.5, y + 0.5, size - 1.0, size - 1.0, 5.5 * k);
            let arm = size * 0.19;
            let white = mix(0xFFFFFFFF, 0xFFFFFFFF, 0.0, shown);
            g.line_f(white, 1.8 * k, cx - arm, cy - arm, cx + arm, cy + arm);
            g.line_f(white, 1.8 * k, cx + arm, cy - arm, cx - arm, cy + arm);
        }
        drop(g);
        BitBlt(hdc, 0, 0, cw, ch, bdc.dc, 0, 0, SRCCOPY);
        EndPaint(hwnd, &ps);
    }
}

fn menu(hwnd: HWND) {
    let Some(n) = get(hwnd) else { return };
    unsafe {
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let add = |m: HMENU, flags: u32, id: usize, text: &str| {
            let t = wide(text);
            AppendMenuW(m, MF_STRING | flags, id, t.as_ptr());
        };
        let m = CreatePopupMenu();
        add(m, check(n.mode.get() != Mode::Pinned), 1, "Follow my mouse");
        add(m, check(n.mode.get() == Mode::Pinned), 2, "Pin here");
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        let op = CreatePopupMenu();
        for (i, v) in OPACITIES.iter().enumerate() {
            add(op, check(*v == n.opacity.get()), 10 + i, &format!("{v}%"));
        }
        add(m, MF_POPUP, op as usize, "Opacity");
        let size = CreatePopupMenu();
        let pct = (n.scale.get() * 100.0).round() as u32;
        for (i, v) in SIZES.iter().enumerate() {
            add(size, check(*v == pct), 20 + i, &format!("{v}%"));
        }
        add(m, MF_POPUP, size as usize, "Size  (or scroll on it)");
        add(m, 0, 30, "Copy image");
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        add(m, 0, 40, "Delete\tWheel click");
        add(m, 0, 41, "Delete all notes");

        let cmd = crate::track_menu(m);
        DestroyMenu(m);
        if IsWindow(hwnd) == 0 {
            return;
        }
        match cmd {
            1 => set_mode(hwnd, Mode::Follow),
            2 => set_mode(hwnd, Mode::Pinned),
            10..=14 => {
                n.opacity.set(OPACITIES[cmd - 10]);
                apply_style(hwnd);
            }
            20..=24 => set_scale(hwnd, SIZES[cmd - 20] as f32 / 100.0),
            30 => copy_image(&n.image),
            40 => close(hwnd),
            41 => crate::clear_all(),
            _ => {}
        }
    }
}

/// Puts the image on the clipboard as a bottom-up CF_DIB.
fn copy_image(img: &Dib) {
    unsafe {
        let header = size_of::<BITMAPINFOHEADER>();
        let bytes = header + (img.w * img.h * 4) as usize;
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if mem.is_null() {
            return;
        }
        let p = GlobalLock(mem) as *mut u8;
        let bi = p as *mut BITMAPINFOHEADER;
        *bi = zeroed();
        (*bi).biSize = header as u32;
        (*bi).biWidth = img.w;
        (*bi).biHeight = img.h;
        (*bi).biPlanes = 1;
        (*bi).biBitCount = 32;
        (*bi).biCompression = BI_RGB;
        let dst = std::slice::from_raw_parts_mut(p.add(header) as *mut u32, (img.w * img.h) as usize);
        let src = img.pixels();
        let w = img.w as usize;
        for row in 0..img.h as usize {
            let from = (img.h as usize - 1 - row) * w;
            dst[row * w..(row + 1) * w].copy_from_slice(&src[from..from + w]);
        }
        GlobalUnlock(mem);
        if OpenClipboard(crate::hidden()) != 0 {
            EmptyClipboard();
            SetClipboardData(8 /* CF_DIB */, mem);
            CloseClipboard();
        }
    }
}

fn mouse_pos(lparam: LPARAM) -> (i32, i32) {
    ((lparam & 0xFFFF) as i16 as i32, ((lparam >> 16) & 0xFFFF) as i16 as i32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCDESTROY {
        let p = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Note;
        if !p.is_null() {
            drop(Box::from_raw(p));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let Some(n) = get(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => return 1,
        WM_PAINT => paint(hwnd, n),
        WM_TIMER if wparam == ANIM_TIMER => tick(hwnd, n),
        WM_DESTROY => crate::note_destroyed(hwnd),
        WM_MOUSEMOVE => {
            if !n.hover.replace(true) {
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut t);
                animate(hwnd);
            }
            if n.left_down.get() {
                let p = crate::cursor();
                let d = n.down_screen.get();
                if !n.dragged.get() && ((p.x - d.x).abs() > 3 || (p.y - d.y).abs() > 3) {
                    n.dragged.set(true);
                    if n.mode.get() != Mode::Pinned {
                        set_mode(hwnd, Mode::Pinned);
                    }
                }
                if n.dragged.get() {
                    let o = n.down_origin.get();
                    SetWindowPos(hwnd, null_mut(), o.x + p.x - d.x, o.y + p.y - d.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            let (x, y) = mouse_pos(lparam);
            if n.close_hot.replace(over_close(hwnd, x, y)) != n.close_hot.get() {
                animate(hwnd);
            }
        }
        WM_MOUSELEAVE => {
            n.hover.set(false);
            n.close_hot.set(false);
            animate(hwnd);
        }
        WM_LBUTTONDOWN => {
            SetCapture(hwnd);
            n.left_down.set(true);
            n.dragged.set(false);
            n.down_screen.set(crate::cursor());
            let mut r: RECT = zeroed();
            GetWindowRect(hwnd, &mut r);
            n.down_origin.set(POINT { x: r.left, y: r.top });
        }
        WM_LBUTTONUP => {
            if n.left_down.replace(false) {
                ReleaseCapture();
                if !n.dragged.get() {
                    let (x, y) = mouse_pos(lparam);
                    if over_close(hwnd, x, y) {
                        close(hwnd);
                    } else {
                        menu(hwnd);
                    }
                }
            }
        }
        WM_RBUTTONUP => menu(hwnd),
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as i16;
            set_scale(hwnd, n.scale.get() * if delta > 0 { 1.1 } else { 1.0 / 1.1 });
        }
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    }
    0
}
