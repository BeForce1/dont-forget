// Client-side drawing straight into the X server's 32-bit BGRX layout. Shapes are anti-aliased
// with signed distance functions, which covers everything a note needs without a 2D library.

pub const BORDER: i32 = 2;
const IMAGE_RADIUS: f32 = 6.0;
pub const WINDOW_RADIUS: i32 = 8;

/// Note frame colors as (r, g, b): left-to-right snips are green, right-to-left blue.
pub const GREEN: (u8, u8, u8) = (46, 204, 113);
pub const BLUE: (u8, u8, u8) = (52, 152, 255);

/// An opaque image, one u32 per pixel as 0x00RRGGBB (BGRX bytes on little-endian).
#[derive(Clone)]
pub struct Image {
    w: u32,
    h: u32,
    px: Vec<u32>,
}

impl Image {
    pub fn new(w: u32, h: u32, color: u32) -> Image {
        Image { w, h, px: vec![color; (w * h) as usize] }
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn crop(&self, x: i32, y: i32, w: u32, h: u32) -> Option<Image> {
        if x < 0 || y < 0 || x as u32 + w > self.w || y as u32 + h > self.h || w == 0 || h == 0 {
            return None;
        }
        let mut px = Vec::with_capacity((w * h) as usize);
        for row in y as u32..y as u32 + h {
            let start = (row * self.w + x as u32) as usize;
            px.extend_from_slice(&self.px[start..start + w as usize]);
        }
        Some(Image { w, h, px })
    }

    /// Bilinear sample at a point in image space (pixel centers at +0.5).
    fn sample(&self, x: f32, y: f32) -> [f32; 3] {
        let (fx, fy) = ((x - 0.5).clamp(0.0, (self.w - 1) as f32), (y - 0.5).clamp(0.0, (self.h - 1) as f32));
        let (x0, y0) = (fx as u32, fy as u32);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: u32, y: u32| self.px[(y * self.w + x) as usize];
        let mut out = [0.0; 3];
        for (i, shift) in [16, 8, 0].into_iter().enumerate() {
            let c = |p: u32| ((p >> shift) & 255) as f32;
            let top = c(at(x0, y0)) * (1.0 - tx) + c(at(x1, y0)) * tx;
            let bottom = c(at(x0, y1)) * (1.0 - tx) + c(at(x1, y1)) * tx;
            out[i] = top * (1.0 - ty) + bottom * ty;
        }
        out
    }

    /// Blends `color` (r, g, b) over the pixel with coverage `a` (0..=1).
    fn blend(&mut self, x: i32, y: i32, color: [f32; 3], a: f32) {
        if a <= 0.0 {
            return;
        }
        let p = &mut self.px[(y as u32 * self.w + x as u32) as usize];
        let mut out = 0;
        for (c, shift) in color.into_iter().zip([16, 8, 0]) {
            let under = ((*p >> shift) & 255) as f32;
            out |= ((under + (c - under) * a.min(1.0)).round() as u32) << shift;
        }
        *p = out;
    }
}

fn rgb(c: (u8, u8, u8)) -> [f32; 3] {
    [c.0 as f32, c.1 as f32, c.2 as f32]
}

/// Signed distance from (px, py) to a rounded rectangle; negative inside.
fn rounded_rect_dist(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.0).min(h / 2.0);
    let qx = (px - (x + w / 2.0)).abs() - w / 2.0 + r;
    let qy = (py - (y + h / 2.0)).abs() - h / 2.0 + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Distance from a point to a line segment.
fn segment_dist(px: f32, py: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - ax - t * dx).hypot(py - ay - t * dy)
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
pub fn render_note(image: &Image, w: i32, h: i32, edge: (u8, u8, u8), show_close: bool, close_hot: bool) -> Image {
    let (w, h) = (w.max(1), h.max(1));
    let mut out = Image::new(w as u32, h as u32, (edge.0 as u32) << 16 | (edge.1 as u32) << 8 | edge.2 as u32);
    let (b, iw, ih) = (BORDER as f32, (w - 2 * BORDER) as f32, (h - 2 * BORDER) as f32);
    let (sx, sy) = (image.w as f32 / iw, image.h as f32 / ih);
    let exact = image.w as f32 == iw && image.h as f32 == ih;
    for y in BORDER..h - BORDER {
        for x in BORDER..w - BORDER {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cover = 0.5 - rounded_rect_dist(cx, cy, b, b, iw, ih, IMAGE_RADIUS);
            if cover <= 0.0 {
                continue;
            }
            let color = if exact {
                let p = image.px[((y - BORDER) as u32 * image.w + (x - BORDER) as u32) as usize];
                [(p >> 16 & 255) as f32, (p >> 8 & 255) as f32, (p & 255) as f32]
            } else {
                image.sample((cx - b) * sx, (cy - b) * sy)
            };
            out.blend(x, y, color, cover);
        }
    }
    if show_close && has_room_for_close(w, h) {
        let (x0, y0, s) = close_rect(w, h);
        let (fx, fy, fs) = (x0 as f32, y0 as f32, s as f32);
        let (bg, bg_alpha) = if close_hot { ((229, 72, 77), 245.0) } else { ((35, 35, 38), 224.0) };
        let ring_alpha = if close_hot { 0.0 } else { 85.0 / 255.0 };
        let (mx, my, arm) = (fx + fs / 2.0, fy + fs / 2.0, fs * 0.19);
        let (a1, b1) = ((mx - arm, my - arm), (mx + arm, my + arm));
        let (a2, b2) = ((mx + arm, my - arm), (mx - arm, my + arm));
        for y in y0..y0 + s {
            for x in x0..x0 + s {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = rounded_rect_dist(cx, cy, fx, fy, fs, fs, 6.0);
                out.blend(x, y, rgb(bg), (0.5 - d).clamp(0.0, 1.0) * bg_alpha / 255.0);
                // 1px ring just inside the edge
                out.blend(x, y, [255.0; 3], (1.0 - (d + 1.0).abs()).clamp(0.0, 1.0) * ring_alpha);
                // the ×: two round-capped strokes, 1.8px wide
                let cross = segment_dist(cx, cy, a1, b1).min(segment_dist(cx, cy, a2, b2));
                out.blend(x, y, [255.0; 3], (0.9 + 0.5 - cross).clamp(0.0, 1.0));
            }
        }
    }
    out
}

/// Pixels as the X server's 32bpp BGRX bytes.
pub fn to_bgrx(img: &Image) -> Vec<u8> {
    img.px.iter().flat_map(|p| p.to_le_bytes()).collect()
}

/// X11 32bpp BGRX bytes -> Image.
pub fn from_bgrx(data: &[u8], w: u32, h: u32) -> Option<Image> {
    let px: Vec<u32> = data
        .chunks_exact(4)
        .take((w * h) as usize)
        .map(|p| u32::from_le_bytes([p[0], p[1], p[2], 0]))
        .collect();
    (px.len() == (w * h) as usize).then_some(Image { w, h, px })
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
