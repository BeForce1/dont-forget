// Small GDI / GDI+ helpers: 32-bit DIBs, memory DCs, anti-aliased shapes and the tray icon.
use std::{mem::{size_of, zeroed}, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Gdi::*, GdiPlus::*},
    UI::WindowsAndMessaging::*,
};

/// Top-down 32 bpp bitmap with direct pixel access (0xAARRGGBB).
pub struct Dib {
    pub bmp: HBITMAP,
    bits: *mut u32,
    pub w: i32,
    pub h: i32,
}

impl Dib {
    pub fn new(w: i32, h: i32) -> Dib {
        unsafe {
            let mut bi: BITMAPINFO = zeroed();
            bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut bits = null_mut();
            let bmp = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            Dib { bmp, bits: bits.cast(), w, h }
        }
    }

    pub fn pixels(&self) -> &mut [u32] {
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize)
        }
    }

    /// Copies a rectangle out, forcing opaque alpha (screen captures come back with alpha 0).
    pub fn crop(&self, x: i32, y: i32, w: i32, h: i32) -> Dib {
        let out = Dib::new(w, h);
        let (src, dst) = (self.pixels(), out.pixels());
        for row in 0..h {
            let s = ((y + row) * self.w + x) as usize;
            let d = (row * w) as usize;
            for i in 0..w as usize {
                dst[d + i] = src[s + i] | 0xFF00_0000;
            }
        }
        out
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.bmp) };
    }
}

/// Memory DC with a bitmap selected into it; restores and frees on drop.
pub struct MemDc {
    pub dc: HDC,
    old: HGDIOBJ,
}

impl MemDc {
    pub fn new(bmp: HBITMAP) -> MemDc {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let old = SelectObject(dc, bmp);
            MemDc { dc, old }
        }
    }
}

impl Drop for MemDc {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteDC(self.dc);
        }
    }
}

pub fn gdiplus_start() {
    unsafe {
        let mut token = 0usize;
        let mut input: GdiplusStartupInput = zeroed();
        input.GdiplusVersion = 1;
        GdiplusStartup(&mut token, &input, null_mut());
    }
}

/// Anti-aliased drawing on a DC. Colors are 0xAARRGGBB.
pub struct Gfx(*mut GpGraphics);

impl Gfx {
    pub fn new(dc: HDC) -> Gfx {
        unsafe {
            let mut g = null_mut();
            GdipCreateFromHDC(dc, &mut g);
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(g, PixelOffsetModeHalf); // shapes line up with pixel edges
            Gfx(g)
        }
    }

    fn pen(argb: u32, width: f32) -> *mut GpPen {
        let mut pen = null_mut();
        unsafe { GdipCreatePen1(argb, width, UnitPixel, &mut pen) };
        pen
    }

    fn add_round_figure(path: *mut GpPath, x: i32, y: i32, w: i32, h: i32, radius: i32) {
        Self::add_round_figure_f(path, x as f32, y as f32, w as f32, h as f32, radius as f32);
    }

    fn add_round_figure_f(path: *mut GpPath, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let d = (radius * 2.0).min(w).min(h).max(1.0);
        unsafe {
            GdipStartPathFigure(path);
            GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
            GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
            GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
            GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
            GdipClosePathFigure(path);
        }
    }

    pub fn fill_round_rect_f(&self, argb: u32, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let mut path = null_mut();
        unsafe { GdipCreatePath(FillModeAlternate, &mut path) };
        Self::add_round_figure_f(path, x, y, w, h, radius);
        self.fill_path(argb, path);
    }

    pub fn round_rect_f(&self, argb: u32, width: f32, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        unsafe {
            let mut path = null_mut();
            GdipCreatePath(FillModeAlternate, &mut path);
            Self::add_round_figure_f(path, x, y, w, h, radius);
            let pen = Self::pen(argb, width);
            GdipDrawPath(self.0, pen, path);
            GdipDeletePen(pen);
            GdipDeletePath(path);
        }
    }

    /// Line with round caps at fractional coordinates.
    pub fn line_f(&self, argb: u32, width: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
        unsafe {
            let pen = Self::pen(argb, width);
            GdipSetPenStartCap(pen, LineCapRound);
            GdipSetPenEndCap(pen, LineCapRound);
            GdipDrawLine(self.0, pen, x1, y1, x2, y2);
            GdipDeletePen(pen);
        }
    }

    fn fill_path(&self, argb: u32, path: *mut GpPath) {
        unsafe {
            let mut brush = null_mut();
            GdipCreateSolidFill(argb, &mut brush);
            GdipFillPath(self.0, brush.cast(), path);
            GdipDeleteBrush(brush.cast());
            GdipDeletePath(path);
        }
    }

    pub fn round_rect(&self, argb: u32, width: f32, x: i32, y: i32, w: i32, h: i32, radius: i32) {
        unsafe {
            let mut path = null_mut();
            GdipCreatePath(FillModeAlternate, &mut path);
            Self::add_round_figure(path, x, y, w, h, radius);
            let pen = Self::pen(argb, width);
            GdipDrawPath(self.0, pen, path);
            GdipDeletePen(pen);
            GdipDeletePath(path);
        }
    }

    pub fn fill_round_rect(&self, argb: u32, x: i32, y: i32, w: i32, h: i32, radius: i32) {
        let mut path = null_mut();
        unsafe { GdipCreatePath(FillModeAlternate, &mut path) };
        Self::add_round_figure(path, x, y, w, h, radius);
        self.fill_path(argb, path);
    }

    /// Paints the four corner wedges outside a rounded rectangle, so whatever is
    /// underneath (e.g. an image) appears to have smooth rounded corners.
    pub fn round_corners(&self, argb: u32, x: i32, y: i32, w: i32, h: i32, radius: i32) {
        let mut path = null_mut();
        unsafe {
            GdipCreatePath(FillModeAlternate, &mut path);
            GdipAddPathRectangleI(path, x, y, w, h);
        }
        Self::add_round_figure(path, x, y, w, h, radius);
        self.fill_path(argb, path);
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        unsafe { GdipDeleteGraphics(self.0) };
    }
}

/// Amber rounded square with a dark "!", drawn pixel by pixel so it needs no resource file.
pub fn make_icon() -> HICON {
    const N: i32 = 32;
    let color = Dib::new(N, N);
    let px = color.pixels();
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // signed distance to a rounded square spanning 2..30 with radius 7
            let qx = ((fx - 16.0).abs() - 7.0).max(0.0);
            let qy = ((fy - 16.0).abs() - 7.0).max(0.0);
            let a = (0.5 - ((qx * qx + qy * qy).sqrt() - 7.0)).clamp(0.0, 1.0);
            let bang = (14..18).contains(&x) && ((7..20).contains(&y) || (22..26).contains(&y));
            let rgb = if bang { 0x1E1E1E } else { 0xFFBA00 };
            px[(y * N + x) as usize] = ((a * 255.0) as u32) << 24 | rgb;
        }
    }
    unsafe {
        let mask_bits = [0u8; (N * N / 8) as usize];
        let mask = CreateBitmap(N, N, 1, 1, mask_bits.as_ptr().cast());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color.bmp };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask);
        icon
    }
}

pub fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

pub fn intersect(a: &RECT, b: &RECT) -> Option<RECT> {
    let r = RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

pub fn draw_label(dc: HDC, font: HFONT, text: &str, x: i32, y: i32) {
    unsafe {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let old = SelectObject(dc, font);
        let mut m = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        DrawTextW(dc, wide.as_mut_ptr(), wide.len() as i32, &mut m, DT_CALCRECT | DT_SINGLELINE);
        let (tw, th) = (m.right + m.bottom + 4, m.bottom + 8);
        let mut bx = rect(x, (y - th - 8).max(0), tw, th);
        // rounded pill, like modern snipping tools
        Gfx::new(dc).fill_round_rect(0xEB1C1C1E, bx.left, bx.top, tw, th, th / 2);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00FFFFFF);
        DrawTextW(dc, wide.as_mut_ptr(), wide.len() as i32, &mut bx, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old);
    }
}
