//! A selection: a mask over the sprite made with the SELECT (rectangle) or LASSO (freehand)
//! tool. Painting is clipped to it, and its pixels can be dragged to a new place.

use crate::doc::Image;
use crate::gfx::Rect;
use crate::raster::line_points;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub w: usize,
    pub h: usize,
    pub mask: Vec<bool>,
    /// Bounding box of the selected pixels (sprite space); empty when nothing is selected.
    pub bounds: Rect,
}

impl Selection {
    fn blank(w: usize, h: usize) -> Selection {
        Selection { w, h, mask: vec![false; w * h], bounds: Rect::default() }
    }

    fn image_rect(w: usize, h: usize) -> Rect {
        Rect::new(0, 0, w as i32, h as i32)
    }

    /// The pixels of `r`, clipped to the image.
    pub fn from_rect(w: usize, h: usize, r: &Rect) -> Selection {
        let mut s = Selection::blank(w, h);
        let c = r.intersect(&Self::image_rect(w, h));
        for y in c.y..c.bottom() {
            let row = y as usize * w;
            s.mask[row + c.x as usize..row + c.right() as usize].fill(true);
        }
        s.bounds = c;
        s
    }

    /// The pixels a pen path passes over (used to show a lasso while it is drawn).
    pub fn from_path(w: usize, h: usize, pts: &[(i32, i32)]) -> Selection {
        let mut s = Selection::blank(w, h);
        for pair in pts.windows(2) {
            line_points(pair[0].0, pair[0].1, pair[1].0, pair[1].1, |x, y| s.set(x, y));
        }
        if let Some(&(x, y)) = pts.first() {
            s.set(x, y);
        }
        s.recompute_bounds();
        s
    }

    /// Everything inside the closed polygon through `pts` (even-odd rule at pixel centres)
    /// plus the pixels on the path itself, so a thin lasso still selects what it circled.
    pub fn from_polygon(w: usize, h: usize, pts: &[(i32, i32)]) -> Selection {
        let mut s = Selection::from_path(w, h, pts);
        if pts.len() >= 3 {
            let (a, b) = (pts[pts.len() - 1], pts[0]);
            line_points(a.0, a.1, b.0, b.1, |x, y| s.set(x, y));
            let n = pts.len();
            let mut xs: Vec<f64> = Vec::new();
            for y in 0..h as i32 {
                let yc = y as f64 + 0.5;
                xs.clear();
                for i in 0..n {
                    let (p, q) = (pts[i], pts[(i + 1) % n]);
                    let (y0, y1) = (p.1 as f64, q.1 as f64);
                    if (y0 <= yc) != (y1 <= yc) {
                        xs.push(p.0 as f64 + (yc - y0) * (q.0 - p.0) as f64 / (y1 - y0));
                    }
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                for pair in xs.chunks(2) {
                    if pair.len() < 2 {
                        break;
                    }
                    // Pixel x is inside when its centre x + 0.5 lies between the crossings.
                    let (xa, xb) = (((pair[0] - 0.5).ceil() as i32).max(0), ((pair[1] - 0.5).floor() as i32).min(w as i32 - 1));
                    for x in xa..=xb {
                        s.set(x, y);
                    }
                }
            }
            s.recompute_bounds();
        }
        s
    }

    /// Magic wand: the pixels of `img` with the same index as the one at (x, y); only the
    /// 4-connected patch around it when `contiguous`. Empty when (x, y) is outside.
    pub fn wand(img: &Image, x: i32, y: i32, contiguous: bool) -> Selection {
        let mut s = Selection::blank(img.w, img.h);
        let Some(v) = img.get(x, y) else { return s };
        if contiguous {
            let mut stack = vec![(x, y)];
            while let Some((px, py)) = stack.pop() {
                if img.get(px, py) != Some(v) || s.contains(px, py) {
                    continue;
                }
                s.set(px, py);
                stack.extend([(px + 1, py), (px - 1, py), (px, py + 1), (px, py - 1)]);
            }
        } else {
            for (k, &p) in img.px.iter().enumerate() {
                if p == v {
                    s.mask[k] = true;
                }
            }
        }
        s.recompute_bounds();
        s
    }

    fn set(&mut self, x: i32, y: i32) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.mask[y as usize * self.w + x as usize] = true;
        }
    }

    fn recompute_bounds(&mut self) {
        let mut b: Option<Rect> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.mask[y * self.w + x] {
                    let px = Rect::new(x as i32, y as i32, 1, 1);
                    b = Some(b.map_or(px, |r| r.union(&px)));
                }
            }
        }
        self.bounds = b.unwrap_or_default();
    }

    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.mask[y as usize * self.w + x as usize]
    }

    /// The same selection moved by (dx, dy); whatever leaves the image is dropped.
    pub fn translated(&self, dx: i32, dy: i32) -> Selection {
        let mut s = Selection::blank(self.w, self.h);
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                if self.contains(x, y) {
                    s.set(x + dx, y + dy);
                }
            }
        }
        s.recompute_bounds();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_selection_is_clipped() {
        let s = Selection::from_rect(4, 4, &Rect::new(2, 2, 10, 10));
        assert_eq!(s.bounds, Rect::new(2, 2, 2, 2));
        assert!(s.contains(3, 3) && !s.contains(1, 3) && !s.contains(4, 4));
        assert!(Selection::from_rect(4, 4, &Rect::new(9, 9, 1, 1)).is_empty());
    }

    #[test]
    fn polygon_fills_inside_and_keeps_the_path() {
        // A diamond around the centre of an 11x11 image.
        let pts = [(5, 0), (10, 5), (5, 10), (0, 5)];
        let s = Selection::from_polygon(11, 11, &pts);
        assert!(s.contains(5, 5), "centre is inside");
        assert!(s.contains(5, 0) && s.contains(0, 5), "path pixels count");
        assert!(!s.contains(0, 0) && !s.contains(10, 10), "corners are outside");
        assert_eq!(s.bounds, Rect::new(0, 0, 11, 11));
        let inside = s.mask.iter().filter(|&&m| m).count();
        assert!(inside > 50 && inside < 80, "roughly half the square: {inside}");
    }

    #[test]
    fn wand_contiguous_and_global() {
        // Two separate patches of 1 on a field of 0, touching only at a corner.
        let mut img = Image::new(6, 6, 0);
        for (x, y) in [(1, 1), (2, 1), (1, 2), (4, 4), (4, 5)] {
            img.set(x, y, 1);
        }
        let patch = Selection::wand(&img, 1, 1, true);
        assert_eq!(patch.mask.iter().filter(|&&m| m).count(), 3);
        assert_eq!(patch.bounds, Rect::new(1, 1, 2, 2));
        let all = Selection::wand(&img, 1, 1, false);
        assert_eq!(all.mask.iter().filter(|&&m| m).count(), 5);
        let paper = Selection::wand(&img, 0, 0, true);
        assert_eq!(paper.mask.iter().filter(|&&m| m).count(), 31, "the background is one patch");
        assert!(Selection::wand(&img, 9, 9, true).is_empty());
    }

    #[test]
    fn open_path_and_translation() {
        let p = Selection::from_path(8, 8, &[(0, 0), (3, 0)]);
        assert_eq!(p.bounds, Rect::new(0, 0, 4, 1));
        let t = Selection::from_rect(8, 8, &Rect::new(6, 6, 2, 2)).translated(1, -1);
        assert_eq!(t.bounds, Rect::new(7, 5, 1, 2));
        assert!(Selection::from_rect(8, 8, &Rect::new(0, 0, 2, 2)).translated(-5, 0).is_empty());
    }
}
