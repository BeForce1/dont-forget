// One floating note: a snipped piece of the screen that is pinned somewhere or rides next to the cursor.
use crate::{gfx::*, wide};
use std::{cell::Cell, mem::{size_of, zeroed}, ptr::{null, null_mut}};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{DataExchange::*, Memory::*},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub const BORDER: i32 = 3;
const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: &str = "DontForgetNote";
const OPACITIES: [u32; 5] = [100, 85, 70, 50, 30];
const SIZES: [u32; 5] = [50, 75, 100, 150, 200];

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Pinned,
    Follow,
    Drop,
}

struct Note {
    image: Dib,
    edge: u32,
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

pub fn create(image: Dib, x: i32, y: i32, edge: u32) -> HWND {
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
        apply_style(hwnd);
        // Windows 11 rounded corners with a matching border that follows the curve.
        let round = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&round as *const i32).cast(), 4);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, (&edge as *const u32).cast(), 4);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        hwnd
    }
}

pub fn set_mode(hwnd: HWND, mode: Mode) {
    let Some(n) = get(hwnd) else { return };
    n.mode.set(mode);
    n.click_through.set(mode != Mode::Pinned);
    apply_style(hwnd);
    crate::on_mode_changed(hwnd, mode);
}

/// Following notes let clicks pass through, except while frozen (Shift held or a menu open).
pub fn set_click_through(hwnd: HWND, on: bool) {
    if let Some(n) = get(hwnd) {
        if n.click_through.replace(on) != on {
            apply_style(hwnd);
        }
    }
}

fn apply_style(hwnd: HWND) {
    let Some(n) = get(hwnd) else { return };
    unsafe {
        let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 | WS_EX_LAYERED;
        if n.click_through.get() { ex |= WS_EX_TRANSPARENT } else { ex &= !WS_EX_TRANSPARENT }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
        SetLayeredWindowAttributes(hwnd, 0, (n.opacity.get() * 255 / 100) as u8, LWA_ALPHA);
    }
}

fn set_scale(hwnd: HWND, scale: f32) {
    let Some(n) = get(hwnd) else { return };
    let s = scale.clamp(0.2, 4.0);
    n.scale.set(s);
    let w = (n.image.w as f32 * s).round() as i32 + 2 * BORDER;
    let h = (n.image.h as f32 * s).round() as i32 + 2 * BORDER;
    unsafe {
        SetWindowPos(hwnd, null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        InvalidateRect(hwnd, null(), 0);
    }
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(hwnd, &mut rc) };
    (rc.right, rc.bottom)
}

fn close_rect(hwnd: HWND) -> RECT {
    rect(client_size(hwnd).0 - 24, 6, 18, 18)
}

fn shows_close(hwnd: HWND, n: &Note) -> bool {
    let (w, h) = client_size(hwnd);
    n.hover.get() && w > 52 && h > 32
}

fn in_rect(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn paint(hwnd: HWND, n: &Note) {
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let (cw, ch) = client_size(hwnd);
        let back = Dib::new(cw, ch);
        let bdc = MemDc::new(back.bmp);
        let brush = CreateSolidBrush(n.edge);
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

        if shows_close(hwnd, n) {
            let r = close_rect(hwnd);
            let g = Gfx::new(bdc.dc);
            g.fill_ellipse(if n.close_hot.get() { 0xE6D22D2D } else { 0xAA000000 }, r.left, r.top, 18, 18);
            let i = 5;
            g.line(0xFFFFFFFF, 2.0, r.left + i, r.top + i, r.right - i, r.bottom - i);
            g.line(0xFFFFFFFF, 2.0, r.right - i, r.top + i, r.left + i, r.bottom - i);
        }
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
            40 => {
                DestroyWindow(hwnd);
            }
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
        WM_DESTROY => crate::note_destroyed(hwnd),
        WM_MOUSEMOVE => {
            if !n.hover.replace(true) {
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut t);
                InvalidateRect(hwnd, null(), 0);
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
            let hot = shows_close(hwnd, n) && in_rect(&close_rect(hwnd), x, y);
            if n.close_hot.replace(hot) != hot {
                InvalidateRect(hwnd, null(), 0);
            }
        }
        WM_MOUSELEAVE => {
            n.hover.set(false);
            n.close_hot.set(false);
            InvalidateRect(hwnd, null(), 0);
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
                    if shows_close(hwnd, n) && in_rect(&close_rect(hwnd), x, y) {
                        DestroyWindow(hwnd);
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
