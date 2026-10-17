// Easing curves and the per-frame stepping used by note animations.

pub const FRAME_MS: f32 = 15.0;
pub const APPEAR_MS: f32 = 260.0;
pub const CLOSE_MS: f32 = 170.0;
pub const PULSE_MS: f32 = 280.0;
pub const BUTTON_MS: f32 = 130.0;
/// Fraction of the remaining distance a following note covers per frame.
pub const GLIDE: f32 = 0.38;

pub fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_out_back(t: f32) -> f32 {
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// Moves `v` one frame toward `target` over a full-range duration of `ms`. Returns true if it changed.
pub fn step(v: &mut f32, target: f32, ms: f32) -> bool {
    if *v == target {
        return false;
    }
    let d = FRAME_MS / ms;
    *v = if *v < target { (*v + d).min(target) } else { (*v - d).max(target) };
    true
}

/// One glide step from `from` toward `to`, snapping when close.
pub fn glide(from: i32, to: i32) -> i32 {
    let d = (to - from) as f32;
    if d.abs() <= 1.5 { to } else { from + (d * GLIDE).round() as i32 }
}

/// (scale, alpha) of a note from its appear / closing / pulse progress (each 0..=1).
pub fn note_transform(appear: f32, closing: Option<f32>, pulse: f32) -> (f32, f32) {
    let mut scale = 0.8 + 0.2 * ease_out_back(appear);
    let mut alpha = ease_out_cubic((appear * 1.8).min(1.0));
    if let Some(c) = closing {
        scale *= 1.0 - 0.15 * c * c;
        alpha *= 1.0 - ease_out_cubic(c);
    }
    scale *= 1.0 + 0.06 * (pulse * std::f32::consts::PI).sin();
    (scale, alpha)
}

/// Top-left of a stack of notes (width, total height) placed below-right of the cursor,
/// flipped to the other side when it would leave the work area (l, t, r, b).
pub fn stack_origin(cursor: (i32, i32), size: (i32, i32), work: (i32, i32, i32, i32)) -> (i32, i32) {
    let (cx, cy) = cursor;
    let (w, h) = size;
    let (l, t, r, b) = work;
    let mut x = cx + 20;
    let mut y = cy + 20;
    if x + w > r {
        x = cx - 12 - w;
    }
    if y + h > b {
        y = cy - 12 - h;
    }
    (x.max(l), y.max(t))
}
