//! Small 2D helpers shared by the apps: rectangles, RGB565 colors, and a `Canvas`
//! that draws into an RGB565 slice (the framebuffer or a back buffer).

use crate::font;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r <= x || b <= y { Rect::default() } else { Rect::new(x, y, r - x, b - y) }
    }
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    pub fn inset(&self, d: i32) -> Rect {
        Rect::new(self.x + d, self.y + d, self.w - 2 * d, self.h - 2 * d)
    }
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
    /// The smallest rect containing both points, inclusive.
    pub fn from_points(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        let (xa, xb) = (x0.min(x1), x0.max(x1));
        let (ya, yb) = (y0.min(y1), y0.max(y1));
        Rect::new(xa, ya, xb - xa + 1, yb - ya + 1)
    }
}

pub const BLACK: u16 = 0x0000;
pub const WHITE: u16 = 0xFFFF;

pub const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

pub const fn grey565(v: u8) -> u16 {
    rgb565(v, v, v)
}

/// One of the panel's 16 grey levels: 0 = black, 15 = white.
pub const fn grey16(level: u8) -> u16 {
    let l = if level > 15 { 15 } else { level };
    grey565(l * 17)
}

/// The grey (0-255) an RGB565 pixel shows on the panel, by its red channel (the UI is grey).
pub const fn grey_of(px: u16) -> u8 {
    (((px >> 11) & 31) * 255 / 31) as u8
}

/// Copy the rows of `r` from one full-size RGB565 buffer to another of the same width.
pub fn copy_rect(dst: &mut [u16], src: &[u16], width: usize, height: usize, r: &Rect) {
    let r = r.intersect(&Rect::new(0, 0, width as i32, height as i32));
    if r.is_empty() {
        return;
    }
    for row in r.y..r.bottom() {
        let a = row as usize * width + r.x as usize;
        let b = a + r.w as usize;
        dst[a..b].copy_from_slice(&src[a..b]);
    }
}

pub struct Canvas<'a> {
    pub px: &'a mut [u16],
    pub w: usize,
    pub h: usize,
}

impl<'a> Canvas<'a> {
    pub fn new(px: &'a mut [u16], w: usize, h: usize) -> Canvas<'a> {
        debug_assert!(px.len() >= w * h);
        Canvas { px, w, h }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w as i32, self.h as i32)
    }

    pub fn put(&mut self, x: i32, y: i32, color: u16) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = color;
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u16) {
        self.fill(&Rect::new(x, y, w, h), color);
    }

    pub fn fill(&mut self, r: &Rect, color: u16) {
        let r = r.intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        for row in r.y..r.bottom() {
            let a = row as usize * self.w + r.x as usize;
            self.px[a..a + r.w as usize].fill(color);
        }
    }

    /// A border of thickness `t` just inside `r`.
    pub fn frame(&mut self, r: &Rect, t: i32, color: u16) {
        self.fill(&Rect::new(r.x, r.y, r.w, t), color);
        self.fill(&Rect::new(r.x, r.bottom() - t, r.w, t), color);
        self.fill(&Rect::new(r.x, r.y, t, r.h), color);
        self.fill(&Rect::new(r.right() - t, r.y, t, r.h), color);
    }

    /// Checkerboard, `cell` pixels per square, anchored at the rect's origin.
    pub fn checker(&mut self, r: &Rect, cell: i32, c1: u16, c2: u16) {
        let clip = r.intersect(&self.bounds());
        if clip.is_empty() || cell <= 0 {
            return;
        }
        for y in clip.y..clip.bottom() {
            let cy = (y - r.y) / cell;
            let row = y as usize * self.w;
            for x in clip.x..clip.right() {
                let cx = (x - r.x) / cell;
                self.px[row + x as usize] = if (cx + cy) % 2 == 0 { c1 } else { c2 };
            }
        }
    }

    /// Draw `s` with the 5x7 font. `scale` is in half pixels: 4 = 2x, 5 = 2.5x, 6 = 3x. Even
    /// scales are plain blocks; odd ones are rendered 2x2-supersampled with grey edges, so a
    /// 2.5x stroke looks even instead of alternating 2 and 3 pixels wide.
    pub fn text(&mut self, mut x: i32, y: i32, scale: i32, color: u16, s: &str) {
        let advance = (font::W as i32 + 1) * scale / 2;
        for ch in s.chars() {
            let g = font::glyph(ch);
            if scale % 2 == 0 {
                let px = scale / 2;
                for (r, bits) in g.iter().enumerate() {
                    for col in 0..font::W {
                        if bits & (0x10 >> col) != 0 {
                            self.fill_rect(x + col as i32 * px, y + r as i32 * px, px, px, color);
                        }
                    }
                }
            } else {
                let (gw, gh) = ((font::W as i32 * scale + 1) / 2, (font::H as i32 * scale + 1) / 2);
                let ink = grey_of(color) as i32;
                for oy in 0..gh {
                    for ox in 0..gw {
                        let mut cov = 0;
                        for (sx, sy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let (c, r) = (((2 * ox + sx) / scale) as usize, ((2 * oy + sy) / scale) as usize);
                            if c < font::W && r < font::H && g[r] & (0x10 >> c) != 0 {
                                cov += 1;
                            }
                        }
                        if cov > 0 {
                            let (px, py) = (x + ox, y + oy);
                            if px >= 0 && py >= 0 && (px as usize) < self.w && (py as usize) < self.h {
                                let k = py as usize * self.w + px as usize;
                                let under = grey_of(self.px[k]) as i32;
                                self.px[k] = grey565(((under * (4 - cov) + ink * cov) / 4) as u8);
                            }
                        }
                    }
                }
            }
            x += advance;
        }
    }

    /// Width of `s` at a half-pixel `scale` (see `text`).
    pub fn text_width(scale: i32, s: &str) -> i32 {
        let n = s.chars().count() as i32;
        if n == 0 { 0 } else { n * (font::W as i32 + 1) * scale / 2 - scale / 2 }
    }

    pub fn text_height(scale: i32) -> i32 {
        (font::H as i32 * scale + 1) / 2
    }

    /// Horizontally centered in the canvas.
    pub fn text_centered(&mut self, y: i32, scale: i32, color: u16, s: &str) {
        let x = (self.w as i32 - Self::text_width(scale, s)) / 2;
        self.text(x, y, scale, color, s);
    }

    /// Centered both ways inside `r`.
    pub fn text_in(&mut self, r: &Rect, scale: i32, color: u16, s: &str) {
        let x = r.x + (r.w - Self::text_width(scale, s)) / 2;
        let y = r.y + (r.h - Self::text_height(scale)) / 2;
        self.text(x, y, scale, color, s);
    }

    /// Left-aligned with `pad`, vertically centered inside `r`.
    pub fn text_left(&mut self, r: &Rect, pad: i32, scale: i32, color: u16, s: &str) {
        let y = r.y + (r.h - Self::text_height(scale)) / 2;
        self.text(r.x + pad, y, scale, color, s);
    }

    /// 16x16 1-bit sprite, `scale` screen pixels per sprite pixel.
    pub fn sprite16(&mut self, x: i32, y: i32, scale: i32, rows: &[u16; 16], color: u16) {
        for (r, bits) in rows.iter().enumerate() {
            for col in 0..16 {
                if bits & (0x8000 >> col) != 0 {
                    self.fill_rect(x + col * scale, y + r as i32 * scale, scale, scale, color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_math() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Rect::new(5, 5, 5, 5));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert!(Rect::new(20, 20, 1, 1).intersect(&a).is_empty());
        assert_eq!(Rect::from_points(3, 4, 1, 2), Rect::new(1, 2, 3, 3));
        assert!(a.contains(9, 9) && !a.contains(10, 9));
    }

    #[test]
    fn canvas_clips() {
        let mut px = vec![WHITE; 4 * 4];
        let mut c = Canvas::new(&mut px, 4, 4);
        c.fill_rect(-2, -2, 4, 4, BLACK); // only (0,0)-(1,1) inside
        assert_eq!(px.iter().filter(|&&p| p == BLACK).count(), 4);
        assert_eq!(Canvas::text_width(2, "AB"), 11); // 1x: two 5-wide glyphs and a gap
        assert_eq!(Canvas::text_width(5, "AB"), 28); // 2.5x: 15 px advance, 2 px trailing gap
        assert_eq!(Canvas::text_height(5), 18);
        assert_eq!(Canvas::text_width(2, ""), 0);
    }

    #[test]
    fn half_scale_text_is_antialiased_and_sized() {
        let mut px = vec![WHITE; 40 * 20];
        let mut c = Canvas::new(&mut px, 40, 20);
        c.text(0, 0, 5, BLACK, "I");
        let dark = px.iter().filter(|&&p| grey_of(p) < 255).count();
        assert!(dark > 0, "something was drawn");
        assert!(px.iter().any(|&p| grey_of(p) > 0 && grey_of(p) < 255), "edges are grey");
        assert!(px.iter().enumerate().all(|(i, &p)| grey_of(p) == 255 || (i % 40) < 13 && (i / 40) < 18), "stays in its 13x18 box");
    }
}
