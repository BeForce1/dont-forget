// Full-screen frozen screenshot, dimmed, with the selected area shown bright.
use crate::{gfx::*, wide};
use std::{cell::RefCell, mem::zeroed, ptr::{null, null_mut}};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForSystem, Input::KeyboardAndMouse::VK_ESCAPE, WindowsAndMessaging::*},
};

const CLASS: &str = "DontForgetOverlay";
// Like CAD box selection: left-to-right is green, right-to-left (adds to followers) is blue.
pub const GREEN: (u32, u32) = (0xFF2ECC71, 0x0071CC2E); // (GDI+ ARGB, GDI COLORREF)
pub const BLUE: (u32, u32) = (0xFF3498FF, 0x00FF9834);

struct Overlay {
    hwnd: HWND,
    vs: RECT,
    start: POINT,
    cur: POINT,
    font: HFONT,
    // DCs before bitmaps so they release the bitmaps first when dropped
    shot_dc: MemDc,
    dim_dc: MemDc,
    back_dc: MemDc,
    shot: Dib,
    _dim: Dib,
    _back: Dib,
}

impl Overlay {
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

pub fn begin(start: POINT) {
    if OVERLAY.with(|o| o.borrow().is_some()) {
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

        let px = (12 * GetDpiForSystem() / 96) as i32;
        let face = wide("Segoe UI");
        let font = CreateFontW(-px, 0, 0, 0, FW_BOLD as i32, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, face.as_ptr());

        let class = wide(CLASS);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            null(),
            WS_POPUP,
            x, y, w, h,
            null_mut(), null_mut(), null_mut(), null(),
        );
        let cur = crate::cursor();
        OVERLAY.with(|o| {
            *o.borrow_mut() = Some(Overlay {
                hwnd, vs: rect(x, y, w, h), start, cur, font,
                shot_dc, dim_dc, back_dc, shot, _dim: dim, _back: back,
            })
        });
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetTimer(hwnd, 1, 10, None);
    }
}

pub fn finish(end: POINT) {
    let Some(mut ov) = OVERLAY.with(|o| o.borrow_mut().take()) else { return };
    ov.cur = end;
    let s = ov.selection();
    let (w, h) = (s.right - s.left, s.bottom - s.top);
    let crop = (w >= 4 && h >= 4).then(|| ov.shot.crop(s.left, s.top, w, h));
    let additive = end.x < ov.start.x;
    let screen_rect = rect(s.left + ov.vs.left, s.top + ov.vs.top, w, h);
    unsafe { DestroyWindow(ov.hwnd) };
    drop(ov);
    if let Some(dib) = crop {
        crate::add_note(dib, screen_rect, additive);
    }
}

fn cancel() {
    if let Some(ov) = OVERLAY.with(|o| o.borrow_mut().take()) {
        unsafe { DestroyWindow(ov.hwnd) };
    }
}

fn track() {
    OVERLAY.with(|o| {
        let mut o = o.borrow_mut();
        let Some(ov) = o.as_mut() else { return };
        let p = crate::cursor();
        if p.x == ov.cur.x && p.y == ov.cur.y {
            return;
        }
        let before = ov.selection();
        ov.cur = p;
        let after = ov.selection();
        // union, padded for the outline and the size label above it
        let dirty = RECT {
            left: before.left.min(after.left) - 160,
            top: before.top.min(after.top) - 48,
            right: before.right.max(after.right) + 160,
            bottom: before.bottom.max(after.bottom) + 8,
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
                let color = if ov.cur.x < ov.start.x { BLUE.0 } else { GREEN.0 };
                Gfx::new(back).round_rect(color, 2.0, s.left - 1, s.top - 1, sw + 1, sh + 1, 4);
                draw_label(back, ov.font, &format!("{sw} × {sh}"), s.left, s.top);
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
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    }
    0
}
