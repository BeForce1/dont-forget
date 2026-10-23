// Full-screen frozen screenshot, dimmed, with the selected area shown bright.
// Opened by a wheel press (the hook reports the drag), or by the snip shortcut (a left-button drag here).
use super::{gfx::*, note::{self, Kind}, wide};
use std::{cell::RefCell, mem::zeroed, ptr::{null, null_mut}, sync::atomic::Ordering};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE}, WindowsAndMessaging::*},
};

const CLASS: &str = "DontForgetOverlay";

struct Overlay {
    hwnd: HWND,
    vs: RECT,
    alpha: u32, // fades in over a few frames
    start: POINT,
    cur: POINT,
    /// Opened from the keyboard: the selection starts at the next left-button press.
    keyboard: bool,
    pressed: bool,
    font: HFONT,
    dpi: u32, // of the monitor where the selection starts; sizes the outline and label
    // DCs before bitmaps so they release the bitmaps first when dropped
    shot_dc: MemDc,
    dim_dc: MemDc,
    back_dc: MemDc,
    shot: Dib,
    _dim: Dib,
    _back: Dib,
}

impl Overlay {
    fn k(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    fn selection(&self) -> RECT {
        RECT {
            left: self.start.x.min(self.cur.x) - self.vs.left,
            top: self.start.y.min(self.cur.y) - self.vs.top,
            right: self.start.x.max(self.cur.x) - self.vs.left,
            bottom: self.start.y.max(self.cur.y) - self.vs.top,
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.font) };
    }
}

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

pub fn register(inst: HINSTANCE) {
    unsafe {
        let class = wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            hCursor: LoadCursorW(null_mut(), IDC_CROSS),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassExW(&wc);
    }
}

use std::mem::size_of;

fn label_font(dpi: u32) -> HFONT {
    let px = (12 * dpi / 96) as i32;
    let face = wide("Segoe UI");
    unsafe { CreateFontW(-px, 0, 0, 0, FW_BOLD as i32, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, face.as_ptr()) }
}

pub fn is_open() -> bool {
    OVERLAY.with(|o| o.borrow().is_some())
}

pub fn begin(start: POINT, keyboard: bool) {
    if is_open() {
        return;
    }
    unsafe {
        let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (w, h) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));

        let shot = Dib::new(w, h);
        let shot_dc = MemDc::new(shot.bmp);
        let screen = GetDC(null_mut());
        BitBlt(shot_dc.dc, 0, 0, w, h, screen, x, y, SRCCOPY);
        ReleaseDC(null_mut(), screen);

        let dim = Dib::new(w, h);
        // blend toward a dark blue-gray (R10 G10 B20) at 120/255
        for (d, s) in dim.pixels().iter_mut().zip(shot.pixels().iter()) {
            let ch = |shift: u32, shade: u32| ((((s >> shift) & 0xFF) * 135 + shade * 120) / 255) << shift;
            *d = ch(16, 10) | ch(8, 10) | ch(0, 20);
        }
        let dim_dc = MemDc::new(dim.bmp);
        let back = Dib::new(w, h);
        let back_dc = MemDc::new(back.bmp);

        let dpi = note::dpi_at(start);
        let font = label_font(dpi);
        if keyboard {
            // a hint at the top of the screen the cursor is on, baked into the dimmed picture
            let mut mi: MONITORINFO = zeroed();
            mi.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromPoint(start, MONITOR_DEFAULTTONEAREST), &mut mi);
            let m = mi.rcMonitor;
            let top = m.top - y + (64 * dpi / 96) as i32;
            draw_label(dim_dc.dc, font, "Drag to snip  ·  Esc cancels", (m.left + m.right) / 2 - x, top, true);
        }

        let class = wide(CLASS);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            class.as_ptr(),
            null(),
            WS_POPUP,
            x, y, w, h,
            null_mut(), null_mut(), null_mut(), null(),
        );
        SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
        let cur = if keyboard { start } else { super::cursor() };
        OVERLAY.with(|o| {
            *o.borrow_mut() = Some(Overlay {
                hwnd, vs: rect(x, y, w, h), alpha: 0, start, cur, keyboard, pressed: false, font, dpi,
                shot_dc, dim_dc, back_dc, shot, _dim: dim, _back: back,
            })
        });
        super::hook::KEY_SELECT.store(keyboard, Ordering::Relaxed);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetTimer(hwnd, 1, 10, None);
    }
}

fn take() -> Option<Overlay> {
    let ov = OVERLAY.with(|o| o.borrow_mut().take())?;
    super::hook::KEY_SELECT.store(false, Ordering::Relaxed);
    unsafe { DestroyWindow(ov.hwnd) };
    Some(ov)
}

pub fn finish(end: POINT) {
    let Some(mut ov) = take() else { return };
    if ov.keyboard && !ov.pressed {
        return;
    }
    ov.cur = end;
    let s = ov.selection();
    let (w, h) = (s.right - s.left, s.bottom - s.top);
    let crop = (w >= 4 && h >= 4).then(|| ov.shot.crop(s.left, s.top, w, h));
    let additive = end.x < ov.start.x;
    let screen_rect = rect(s.left + ov.vs.left, s.top + ov.vs.top, w, h);
    drop(ov);
    if let Some(dib) = crop {
        super::add_note(dib, screen_rect, additive);
    }
}

fn cancel() {
    take();
}

/// Keyboard mode: the left button goes down where the selection starts.
fn press() {
    OVERLAY.with(|o| {
        let mut o = o.borrow_mut();
        let Some(ov) = o.as_mut() else { return };
        if !ov.keyboard || ov.pressed {
            return;
        }
        let p = super::cursor();
        ov.pressed = true;
        ov.start = p;
        ov.cur = p;
        let dpi = note::dpi_at(p);
        if dpi != ov.dpi {
            unsafe { DeleteObject(ov.font) };
            ov.font = label_font(dpi);
            ov.dpi = dpi;
        }
        unsafe { SetCapture(ov.hwnd) };
    });
}

fn track() {
    OVERLAY.with(|o| {
        let mut o = o.borrow_mut();
        let Some(ov) = o.as_mut() else { return };
        if ov.alpha < 255 {
            ov.alpha = (ov.alpha + 50).min(255);
            unsafe { SetLayeredWindowAttributes(ov.hwnd, 0, ov.alpha as u8, LWA_ALPHA) };
        }
        let p = super::cursor();
        if p.x == ov.cur.x && p.y == ov.cur.y || ov.keyboard && !ov.pressed {
            return;
        }
        let before = ov.selection();
        ov.cur = p;
        let after = ov.selection();
        // union, padded for the outline and the size label above it
        let k = ov.k();
        let (pad_x, pad_top, pad_bottom) = ((160.0 * k) as i32, (48.0 * k) as i32, (8.0 * k) as i32 + 8);
        let dirty = RECT {
            left: before.left.min(after.left) - pad_x,
            top: before.top.min(after.top) - pad_top,
            right: before.right.max(after.right) + pad_x,
            bottom: before.bottom.max(after.bottom) + pad_bottom,
        };
        unsafe { InvalidateRect(ov.hwnd, &dirty, 0) };
    });
}

fn paint(hwnd: HWND) {
    OVERLAY.with(|o| unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        if let Some(ov) = o.borrow().as_ref() {
            let rc = ps.rcPaint;
            let (x, y, w, h) = (rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top);
            let back = ov.back_dc.dc;
            BitBlt(back, x, y, w, h, ov.dim_dc.dc, x, y, SRCCOPY);
            let s = ov.selection();
            let (sw, sh) = (s.right - s.left, s.bottom - s.top);
            if sw > 0 && sh > 0 {
                if let Some(i) = intersect(&s, &rc) {
                    BitBlt(back, i.left, i.top, i.right - i.left, i.bottom - i.top, ov.shot_dc.dc, i.left, i.top, SRCCOPY);
                }
                let st = note::style();
                let color = st.color(if ov.cur.x < ov.start.x { Kind::Add } else { Kind::Snip });
                let (k, width) = (ov.k(), (st.width.max(2) as f32 * ov.k()).round());
                let (x, y) = (s.left as f32 - width / 2.0, s.top as f32 - width / 2.0);
                Gfx::new(back).outline(0xFF00_0000 | color, width, st.outline, x, y, sw as f32 + width, sh as f32 + width, 4.0 * k);
                draw_label(back, ov.font, &format!("{sw} × {sh}"), s.left, s.top, false);
            }
            BitBlt(hdc, x, y, w, h, back, x, y, SRCCOPY);
        }
        EndPaint(hwnd, &ps);
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => paint(hwnd),
        WM_ERASEBKGND => return 1,
        WM_TIMER => track(),
        WM_KEYDOWN if wparam as u16 == VK_ESCAPE => cancel(),
        WM_RBUTTONDOWN => cancel(),
        WM_LBUTTONDOWN => press(),
        WM_LBUTTONUP if OVERLAY.with(|o| o.borrow().as_ref().is_some_and(|ov| ov.pressed)) => {
            ReleaseCapture();
            finish(super::cursor());
        }
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    }
    0
}
