//! Drawing primitives on an indexed `Image`. Every operation returns the image-space
//! rectangle it touched so the caller can repaint just that.

use crate::doc::Image;
use crate::gfx::Rect;

fn clip(img: &Image, r: &Rect) -> Rect {
    r.intersect(&Rect::new(0, 0, img.w as i32, img.h as i32))
}

pub fn fill_rect(img: &mut Image, r: &Rect, v: u8) -> Rect {
    let c = clip(img, r);
    for y in c.y..c.bottom() {
        let row = y as usize * img.w;
        img.px[row + c.x as usize..row + c.right() as usize].fill(v);
    }
    c
}

/// A square brush of `size` pixels, the pen point at its centre (upper-left of centre for
/// even sizes).
pub fn stamp(img: &mut Image, x: i32, y: i32, size: i32, v: u8) -> Rect {
    let size = size.max(1);
    let o = (size - 1) / 2;
    fill_rect(img, &Rect::new(x - o, y - o, size, size), v)
}

/// The rectangle `stamp` would touch, without drawing.
pub fn stamp_rect(img: &Image, x: i32, y: i32, size: i32) -> Rect {
    let size = size.max(1);
    let o = (size - 1) / 2;
    clip(img, &Rect::new(x - o, y - o, size, size))
}

/// Copy the pixels of `r` from `src` into `dst` (same size images).
pub fn copy_rect(dst: &mut Image, src: &Image, r: &Rect) {
    if dst.w != src.w || dst.h != src.h {
        return;
    }
    let c = clip(dst, r);
    for y in c.y..c.bottom() {
        let a = y as usize * dst.w + c.x as usize;
        let b = a + c.w as usize;
        dst.px[a..b].copy_from_slice(&src.px[a..b]);
    }
}

/// Every pixel of the Bresenham line from (x0, y0) to (x1, y1), both ends included, in order.
pub fn line_points(x0: i32, y0: i32, x1: i32, y1: i32, mut f: impl FnMut(i32, i32)) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    loop {
        f(x, y);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Bresenham line, stamping the brush at every point.
pub fn line(img: &mut Image, x0: i32, y0: i32, x1: i32, y1: i32, size: i32, v: u8) -> Rect {
    let mut touched = Rect::default();
    line_points(x0, y0, x1, y1, |x, y| touched = touched.union(&stamp(img, x, y, size, v)));
    touched
}

/// Aseprite's "pixel perfect" freehand rule: which points of a pixel-stepped path to draw.
/// A point that forms the corner of an L with its neighbours (orthogonal to both, and the
/// two neighbours diagonal to each other) is skipped, so a hand-drawn diagonal comes out one
/// pixel wide instead of a staircase of doubled corners. The point after a skipped corner is
/// always drawn, exactly as in Aseprite/LibreSprite, so results match files drawn there.
pub fn pixel_perfect_keep(pts: &[(i32, i32)]) -> Vec<bool> {
    let mut keep = vec![true; pts.len()];
    let mut c = 0;
    while c < pts.len() {
        if c > 0 && c + 1 < pts.len() {
            let (p, q, n) = (pts[c - 1], pts[c], pts[c + 1]);
            if (p.0 == q.0 || p.1 == q.1) && (n.0 == q.0 || n.1 == q.1) && p.0 != n.0 && p.1 != n.1 {
                keep[c] = false;
                c += 1;
            }
        }
        c += 1;
    }
    keep
}

pub fn rect_outline(img: &mut Image, x0: i32, y0: i32, x1: i32, y1: i32, size: i32, v: u8) -> Rect {
    let r = Rect::from_points(x0, y0, x1, y1);
    let (xa, ya, xb, yb) = (r.x, r.y, r.right() - 1, r.bottom() - 1);
    let mut t = line(img, xa, ya, xb, ya, size, v);
    t = t.union(&line(img, xb, ya, xb, yb, size, v));
    t = t.union(&line(img, xb, yb, xa, yb, size, v));
    t.union(&line(img, xa, yb, xa, ya, size, v))
}

pub fn rect_filled(img: &mut Image, x0: i32, y0: i32, x1: i32, y1: i32, v: u8) -> Rect {
    fill_rect(img, &Rect::from_points(x0, y0, x1, y1), v)
}

/// Ellipse inscribed in the box spanned by the two points. Outline is one pixel thick
/// (stamped with the brush); filled covers every pixel whose centre is inside.
pub fn ellipse(img: &mut Image, x0: i32, y0: i32, x1: i32, y1: i32, size: i32, v: u8, filled: bool) -> Rect {
    let r = Rect::from_points(x0, y0, x1, y1);
    let (w, h) = (r.w as i64, r.h as i64);
    // Pixel (x, y) is inside when ((2dx)/w)^2 + ((2dy)/h)^2 <= 1 with dx, dy measured from
    // the box centre to the pixel centre, all in half-pixel units to stay in integers.
    let cx2 = 2 * r.x as i64 + w; // 2 * centre x
    let cy2 = 2 * r.y as i64 + h;
    let inside = |x: i32, y: i32| -> bool {
        if !r.contains(x, y) {
            return false;
        }
        let dx = 2 * x as i64 + 1 - cx2;
        let dy = 2 * y as i64 + 1 - cy2;
        dx * dx * h * h + dy * dy * w * w <= w * w * h * h
    };
    let mut touched = Rect::default();
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            if !inside(x, y) {
                continue;
            }
            let edge = !inside(x - 1, y) || !inside(x + 1, y) || !inside(x, y - 1) || !inside(x, y + 1);
            if filled {
                if img.in_bounds(x, y) {
                    img.set(x, y, v);
                    touched = touched.union(&Rect::new(x, y, 1, 1));
                }
            } else if edge {
                touched = touched.union(&stamp(img, x, y, size, v));
            }
        }
    }
    touched
}

/// 4-connected flood fill of the region containing (x, y).
pub fn flood_fill(img: &mut Image, x: i32, y: i32, v: u8) -> Rect {
    let Some(target) = img.get(x, y) else { return Rect::default() };
    if target == v {
        return Rect::default();
    }
    let mut touched = Rect::new(x, y, 1, 1);
    let mut stack = vec![(x, y)];
    while let Some((px, py)) = stack.pop() {
        if img.get(px, py) != Some(target) {
            continue;
        }
        // Walk the whole horizontal run, then seed the rows above and below.
        let mut x0 = px;
        while img.get(x0 - 1, py) == Some(target) {
            x0 -= 1;
        }
        let mut x1 = px;
        while img.get(x1 + 1, py) == Some(target) {
            x1 += 1;
        }
        for xx in x0..=x1 {
            img.set(xx, py, v);
        }
        touched = touched.union(&Rect::new(x0, py, x1 - x0 + 1, 1));
        for ny in [py - 1, py + 1] {
            let mut xx = x0;
            while xx <= x1 {
                if img.get(xx, ny) == Some(target) {
                    stack.push((xx, ny));
                    while xx <= x1 && img.get(xx, ny) == Some(target) {
                        xx += 1;
                    }
                }
                xx += 1;
            }
        }
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(img: &Image, v: u8) -> usize {
        img.px.iter().filter(|&&p| p == v).count()
    }

    #[test]
    fn stamps_and_lines() {
        let mut img = Image::new(8, 8, 0);
        assert_eq!(stamp(&mut img, 0, 0, 1, 1), Rect::new(0, 0, 1, 1));
        assert_eq!(stamp(&mut img, 7, 7, 3, 1), Rect::new(6, 6, 2, 2)); // clipped
        let t = line(&mut img, 0, 7, 7, 0, 1, 2);
        assert_eq!(t, Rect::new(0, 0, 8, 8));
        assert_eq!(count(&img, 2), 8);
    }

    #[test]
    fn pixel_perfect_drops_l_corners() {
        // A staircase becomes a clean diagonal; the point after each dropped corner is kept
        // without being examined (Aseprite's rule), otherwise the whole line would vanish.
        let stairs = [(0, 0), (1, 0), (1, 1), (2, 1), (2, 2), (3, 2), (3, 3)];
        assert_eq!(pixel_perfect_keep(&stairs), vec![true, false, true, false, true, false, true]);
        assert!(pixel_perfect_keep(&[(0, 0), (1, 0), (2, 0), (3, 0)]).iter().all(|&k| k));
        assert!(pixel_perfect_keep(&[(0, 0), (1, 1), (2, 2)]).iter().all(|&k| k));
        assert_eq!(pixel_perfect_keep(&[(5, 5)]), vec![true]);
        assert!(pixel_perfect_keep(&[]).is_empty());
    }

    #[test]
    fn line_points_copy_rect_and_stamp_rect() {
        let mut pts = Vec::new();
        line_points(0, 0, 3, 1, |x, y| pts.push((x, y)));
        assert_eq!((pts.first(), pts.last(), pts.len()), (Some(&(0, 0)), Some(&(3, 1)), 4));
        let mut a = Image::new(4, 4, 0);
        let b = Image::new(4, 4, 9);
        copy_rect(&mut a, &b, &Rect::new(1, 1, 2, 2));
        assert_eq!(count(&a, 9), 4);
        copy_rect(&mut a, &Image::new(2, 2, 7), &Rect::new(0, 0, 4, 4)); // size mismatch: ignored
        assert_eq!(count(&a, 7), 0);
        assert_eq!(stamp_rect(&a, 0, 0, 3), Rect::new(0, 0, 2, 2));
        assert!(stamp_rect(&a, 10, 10, 1).is_empty());
    }

    #[test]
    fn rects() {
        let mut img = Image::new(6, 6, 0);
        rect_outline(&mut img, 1, 1, 4, 4, 1, 1);
        assert_eq!(count(&img, 1), 12);
        rect_filled(&mut img, 1, 1, 4, 4, 2);
        assert_eq!(count(&img, 2), 16);
    }

    #[test]
    fn ellipses() {
        let mut img = Image::new(9, 9, 0);
        let t = ellipse(&mut img, 0, 0, 8, 8, 1, 1, true);
        assert_eq!(t, Rect::new(0, 0, 9, 9));
        assert_eq!(img.get(4, 4), Some(1));
        assert_eq!(img.get(0, 0), Some(0)); // corners stay out
        assert_eq!(img.get(4, 0), Some(1)); // the box is touched at the middle of each edge
        let mut img = Image::new(9, 9, 0);
        ellipse(&mut img, 0, 0, 8, 8, 1, 1, false);
        assert_eq!(img.get(4, 4), Some(0));
        assert_eq!(img.get(4, 0), Some(1));
    }

    #[test]
    fn flood() {
        let mut img = Image::new(5, 5, 0);
        rect_outline(&mut img, 1, 1, 3, 3, 1, 1);
        let t = flood_fill(&mut img, 2, 2, 2);
        assert_eq!(t, Rect::new(2, 2, 1, 1));
        assert_eq!(count(&img, 2), 1);
        let t = flood_fill(&mut img, 0, 0, 3);
        assert_eq!(t, Rect::new(0, 0, 5, 5));
        assert_eq!(count(&img, 3), 16);
        assert!(flood_fill(&mut img, 0, 0, 3).is_empty()); // same color: nothing to do
        assert!(flood_fill(&mut img, -1, 0, 3).is_empty());
    }
}
