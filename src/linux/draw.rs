// Client-side drawing with tiny-skia, plus conversion to the X server's 32-bit BGRX layout.
use tiny_skia::*;

pub const BORDER: i32 = 2;
const IMAGE_RADIUS: f32 = 6.0;
pub const WINDOW_RADIUS: i32 = 8;

/// Note frame colors as (r, g, b): left-to-right snips are green, right-to-left blue.
pub const GREEN: (u8, u8, u8) = (46, 204, 113);
pub const BLUE: (u8, u8, u8) = (52, 152, 255);

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = r * 0.552_284_8; // cubic approximation of a quarter circle
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

/// Close button rect (x, y, size) for a note of the given client size.
pub fn close_rect(w: i32, _h: i32) -> (i32, i32, i32) {
    let (size, margin) = (22, BORDER + 6);
    (w - margin - size, margin, size)
}

pub fn has_room_for_close(w: i32, h: i32) -> bool {
    w > 22 * 2 + 24 && h > 22 + 16
}

/// Renders a note: colored frame, the snip with rounded corners, and the × button when hovered.
pub fn render_note(image: &Pixmap, w: i32, h: i32, edge: (u8, u8, u8), show_close: bool, close_hot: bool) -> Pixmap {
    let mut pm = Pixmap::new(w.max(1) as u32, h.max(1) as u32).expect("note size");
    pm.fill(Color::from_rgba8(edge.0, edge.1, edge.2, 255));
    let (iw, ih) = ((w - 2 * BORDER) as f32, (h - 2 * BORDER) as f32);
    let (sx, sy) = (iw / image.width() as f32, ih / image.height() as f32);
    // the snip, clipped to a rounded rectangle via a pattern fill (anti-aliased edges)
    if let Some(path) = rounded_rect(BORDER as f32, BORDER as f32, iw, ih, IMAGE_RADIUS) {
        let quality = if (sx - 1.0).abs() < 1e-3 && (sy - 1.0).abs() < 1e-3 { FilterQuality::Nearest } else { FilterQuality::Bicubic };
        let shader = Pattern::new(
            image.as_ref(),
            SpreadMode::Pad,
            quality,
            1.0,
            Transform::from_scale(sx, sy).post_translate(BORDER as f32, BORDER as f32),
        );
        let p = Paint { shader, anti_alias: true, ..Default::default() };
        pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    }
    if show_close && has_room_for_close(w, h) {
        let (x, y, s) = close_rect(w, h);
        let (x, y, s) = (x as f32, y as f32, s as f32);
        let bg = if close_hot { paint(229, 72, 77, 245) } else { paint(35, 35, 38, 224) };
        if let Some(p) = rounded_rect(x, y, s, s, 6.0) {
            pm.fill_path(&p, &bg, FillRule::Winding, Transform::identity(), None);
        }
        if let Some(p) = rounded_rect(x + 0.5, y + 0.5, s - 1.0, s - 1.0, 5.5) {
            let ring = if close_hot { paint(255, 255, 255, 0) } else { paint(255, 255, 255, 85) };
            pm.stroke_path(&p, &ring, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
        }
        let (cx, cy, arm) = (x + s / 2.0, y + s / 2.0, s * 0.19);
        let mut pb = PathBuilder::new();
        pb.move_to(cx - arm, cy - arm);
        pb.line_to(cx + arm, cy + arm);
        pb.move_to(cx + arm, cy - arm);
        pb.line_to(cx - arm, cy + arm);
        if let Some(p) = pb.finish() {
            let stroke = Stroke { width: 1.8, line_cap: LineCap::Round, ..Default::default() };
            pm.stroke_path(&p, &paint(255, 255, 255, 255), &stroke, Transform::identity(), None);
        }
    }
    pm
}

/// tiny-skia (premultiplied RGBA, opaque here) -> X11 32bpp BGRX.
pub fn to_bgrx(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for px in pm.data().chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    out
}

/// X11 32bpp BGRX -> tiny-skia RGBA.
pub fn from_bgrx(data: &[u8], w: u32, h: u32) -> Option<Pixmap> {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for px in data.chunks_exact(4).take((w * h) as usize) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    Pixmap::from_vec(rgba, IntSize::from_wh(w, h)?)
}

/// Rows of a rounded rectangle as (x, y, width, height) runs, for the X Shape extension.
pub fn rounded_shape(w: i32, h: i32, r: i32) -> Vec<(i32, i32, i32, i32)> {
    let r = r.min(w / 2).min(h / 2);
    let mut rows = Vec::new();
    for i in 0..r {
        let dy = r as f32 - i as f32 - 0.5;
        let inset = (r as f32 - (r as f32 * r as f32 - dy * dy).max(0.0).sqrt()).round() as i32;
        rows.push((inset, i, w - 2 * inset, 1));
        rows.push((inset, h - 1 - i, w - 2 * inset, 1));
    }
    rows.push((0, r, w, h - 2 * r));
    rows
}
