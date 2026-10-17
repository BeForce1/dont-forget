// Full-screen selection overlay: a frozen screenshot, dimmed, with the selected area bright.
// Both versions live as server-side pixmaps, so dragging only copies the changed region.
use super::{draw, put_image, Conn, Res};
use tiny_skia::{IntRect, Pixmap};
use x11rb::{connection::Connection, protocol::xproto::*};

pub struct Overlay {
    pub win: Window,
    pub start: (i32, i32),
    pub cur: (i32, i32),
    screen: (i32, i32),
    shot: Pixmap, // client copy, for cropping
    bright: x11rb::protocol::xproto::Pixmap,
    dim: x11rb::protocol::xproto::Pixmap,
    gc: Gcontext,
    outline: Gcontext,
    text: Gcontext,
    font: Font,
}

fn rgb(c: (u8, u8, u8)) -> u32 {
    (c.0 as u32) << 16 | (c.1 as u32) << 8 | c.2 as u32
}

impl Overlay {
    pub fn open(conn: &Conn, root: Window, depth: u8, visual: Visualid, screen: (i32, i32), start: (i32, i32)) -> Res<Overlay> {
        let (w, h) = screen;
        let mut data = conn.get_image(ImageFormat::Z_PIXMAP, root, 0, 0, w as u16, h as u16, !0)?.reply()?.data;
        let shot = draw::from_bgrx(&data, w as u32, h as u32).ok_or("screenshot failed")?;

        let gc = conn.generate_id()?;
        conn.create_gc(gc, root, &CreateGCAux::new().graphics_exposures(0))?;
        let bright = conn.generate_id()?;
        conn.create_pixmap(depth, bright, root, w as u16, h as u16)?;
        put_image(conn, bright, gc, depth, w, h, &data)?;
        // dim toward a dark blue-gray (R10 G10 B20) at 120/255
        for px in data.chunks_exact_mut(4) {
            for (c, shade) in px[..3].iter_mut().zip([20u32, 10, 10]) {
                *c = ((*c as u32 * 135 + shade * 120) / 255) as u8;
            }
        }
        let dim = conn.generate_id()?;
        conn.create_pixmap(depth, dim, root, w as u16, h as u16)?;
        put_image(conn, dim, gc, depth, w, h, &data)?;

        let win = conn.generate_id()?;
        conn.create_window(
            depth,
            win,
            root,
            0,
            0,
            w as u16,
            h as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new().override_redirect(1).background_pixmap(dim).event_mask(EventMask::EXPOSURE),
        )?;
        let outline = conn.generate_id()?;
        conn.create_gc(outline, root, &CreateGCAux::new().line_width(2).graphics_exposures(0))?;
        let font = conn.generate_id()?;
        conn.open_font(font, b"fixed")?;
        let text = conn.generate_id()?;
        conn.create_gc(text, root, &CreateGCAux::new().font(font).foreground(0xffffff).background(0x1c1c1e))?;
        conn.map_window(win)?;
        conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        let ov = Overlay { win, start, cur: start, screen, shot, bright, dim, gc, outline, text, font };
        Ok(ov)
    }

    /// Selection in screen coordinates as (x, y, w, h), clipped to the screen.
    pub fn selection(&self) -> (i32, i32, i32, i32) {
        let l = self.start.0.min(self.cur.0).max(0);
        let t = self.start.1.min(self.cur.1).max(0);
        let r = self.start.0.max(self.cur.0).min(self.screen.0);
        let b = self.start.1.max(self.cur.1).min(self.screen.1);
        (l, t, r - l, b - t)
    }

    pub fn crop(&self) -> Option<Pixmap> {
        let (x, y, w, h) = self.selection();
        if w < 4 || h < 4 {
            return None;
        }
        self.shot.clone_rect(IntRect::from_xywh(x, y, w as u32, h as u32)?)
    }

    pub fn track(&mut self, conn: &Conn, p: (i32, i32)) -> Res<()> {
        if p == self.cur {
            return Ok(());
        }
        let (bx, by, bw, bh) = self.selection();
        self.cur = p;
        let (ax, ay, aw, ah) = self.selection();
        // repaint the union of old and new selection, padded for the outline and the label above it
        let l = bx.min(ax) - 4;
        let t = by.min(ay) - 28;
        let r = (bx + bw).max(ax + aw) + 140;
        let b = (by + bh).max(ay + ah) + 4;
        self.paint(conn, l, t, r - l, b - t)
    }

    pub fn redraw_all(&self, conn: &Conn) -> Res<()> {
        self.paint(conn, 0, 0, self.screen.0, self.screen.1)
    }

    fn paint(&self, conn: &Conn, x: i32, y: i32, w: i32, h: i32) -> Res<()> {
        let (x, y) = (x.max(0), y.max(0));
        conn.copy_area(self.dim, self.win, self.gc, x as i16, y as i16, x as i16, y as i16, w.max(0) as u16, h.max(0) as u16)?;
        let (sx, sy, sw, sh) = self.selection();
        if sw <= 0 || sh <= 0 {
            return Ok(());
        }
        conn.copy_area(self.bright, self.win, self.gc, sx as i16, sy as i16, sx as i16, sy as i16, sw as u16, sh as u16)?;
        let color = if self.cur.0 < self.start.0 { draw::BLUE } else { draw::GREEN };
        conn.change_gc(self.outline, &ChangeGCAux::new().foreground(rgb(color)))?;
        let outline = Rectangle { x: (sx - 1) as i16, y: (sy - 1) as i16, width: (sw + 1) as u16, height: (sh + 1) as u16 };
        conn.poly_rectangle(self.win, self.outline, &[outline])?;
        let label = format!(" {sw} x {sh} ");
        conn.image_text8(self.win, self.text, sx as i16, (sy - 8).max(12) as i16, label.as_bytes())?;
        Ok(())
    }

    pub fn close(self, conn: &Conn) -> Res<()> {
        conn.destroy_window(self.win)?;
        conn.free_pixmap(self.bright)?;
        conn.free_pixmap(self.dim)?;
        for gc in [self.gc, self.outline, self.text] {
            conn.free_gc(gc)?;
        }
        conn.close_font(self.font)?;
        Ok(())
    }
}
