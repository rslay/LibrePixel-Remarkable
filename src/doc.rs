//! The document model: a sprite is a stack of layers of indexed pixels over a palette of
//! 16 colors. No animation: one frame.
//!
//! Index 0 is "the paper": on a background layer it is painted with the palette color, on
//! every other layer it is transparent. That is exactly Aseprite's transparent-index rule,
//! so files round-trip, and it gives the eraser a single meaning.
//!
//! The panel shows 16 greys, so there are two ways to look at colors (`ViewMode`): by
//! luminance, or by index rank so every palette slot stays distinguishable while editing.
//! Files always keep the true colors.

use crate::gfx::Rect;

pub const PALETTE_SIZE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 255 }
    }
    pub const fn grey(v: u8) -> Rgba {
        Rgba { r: v, g: v, b: v, a: 255 }
    }
    pub fn luma(&self) -> u8 {
        ((self.r as u32 * 299 + self.g as u32 * 587 + self.b as u32 * 114) / 1000) as u8
    }
    pub fn dist2(&self, o: &Rgba) -> u32 {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2) as u32;
        d(self.r, o.r) + d(self.g, o.g) + d(self.b, o.b)
    }
}

/// DawnBringer's 16-color palette (public domain) minus its near-white, with white as slot 0.
pub const DEFAULT_PALETTE: [Rgba; PALETTE_SIZE] = [
    Rgba::rgb(255, 255, 255),
    Rgba::rgb(20, 12, 28),
    Rgba::rgb(68, 36, 52),
    Rgba::rgb(48, 52, 109),
    Rgba::rgb(78, 74, 78),
    Rgba::rgb(133, 76, 48),
    Rgba::rgb(52, 101, 36),
    Rgba::rgb(208, 70, 72),
    Rgba::rgb(117, 113, 97),
    Rgba::rgb(89, 125, 206),
    Rgba::rgb(210, 125, 44),
    Rgba::rgb(133, 149, 161),
    Rgba::rgb(109, 170, 44),
    Rgba::rgb(210, 170, 153),
    Rgba::rgb(109, 194, 202),
    Rgba::rgb(218, 212, 94),
];

/// Index of the palette color (searching from `from`) closest to `c`, by RGB distance.
pub fn nearest_index(colors: &[Rgba], c: Rgba, from: usize) -> u8 {
    let mut best = from.min(colors.len().saturating_sub(1));
    let mut best_d = u32::MAX;
    for (i, p) in colors.iter().enumerate().skip(from) {
        let d = p.dist2(&c);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best as u8
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Image {
    pub fn new(w: usize, h: usize, fill: u8) -> Image {
        Image { w, h, px: vec![fill; w * h] }
    }
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }
    pub fn get(&self, x: i32, y: i32) -> Option<u8> {
        if self.in_bounds(x, y) { Some(self.px[y as usize * self.w + x as usize]) } else { None }
    }
    pub fn set(&mut self, x: i32, y: i32, v: u8) {
        if self.in_bounds(x, y) {
            self.px[y as usize * self.w + x as usize] = v;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerKind {
    Image,
    Group,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Layer {
    pub name: String,
    pub kind: LayerKind,
    /// Nesting depth as in the file: a layer with level n is a child of the nearest layer
    /// below it (earlier in the list) with level n-1.
    pub child_level: u16,
    pub visible: bool,
    /// Painting on it is refused (Aseprite's "editable" flag, inverted). A hidden layer is
    /// treated as locked as well.
    pub locked: bool,
    pub background: bool,
    pub opacity: u8,
    /// Kept from the file and written back; rendering treats every mode as normal.
    pub blend_mode: u16,
    /// Sprite-sized for image layers, 0x0 for groups.
    pub image: Image,
}

impl Layer {
    pub fn image(name: &str, w: usize, h: usize, fill: u8) -> Layer {
        Layer {
            name: name.to_string(),
            kind: LayerKind::Image,
            child_level: 0,
            visible: true,
            locked: false,
            background: false,
            opacity: 255,
            blend_mode: 0,
            image: Image::new(w, h, fill),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    /// Each palette slot gets its own grey, ordered by the slot's luminance.
    Index,
    /// Each slot shown at its luminance, quantised to the panel's 16 greys.
    Luma,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sprite {
    pub w: usize,
    pub h: usize,
    pub palette: [Rgba; PALETTE_SIZE],
    /// The index that is transparent on non-background layers (Aseprite's "transparent color").
    pub transparent: u8,
    /// Bottom first.
    pub layers: Vec<Layer>,
}

impl Sprite {
    /// A new sprite: one opaque background layer filled with slot 0 (white).
    pub fn new(w: usize, h: usize) -> Sprite {
        let mut bg = Layer::image("Background", w, h, 0);
        bg.background = true;
        Sprite { w, h, palette: DEFAULT_PALETTE, transparent: 0, layers: vec![bg] }
    }

    pub fn parent_of(&self, i: usize) -> Option<usize> {
        let level = self.layers[i].child_level;
        if level == 0 {
            return None;
        }
        (0..i).rev().find(|&k| self.layers[k].child_level == level - 1)
    }

    /// Visible, and every group it sits in is visible.
    pub fn effective_visible(&self, i: usize) -> bool {
        let mut j = i;
        loop {
            if !self.layers[j].visible {
                return false;
            }
            match self.parent_of(j) {
                Some(p) => j = p,
                None => return true,
            }
        }
    }

    /// Insert a new transparent image layer right above layer `above`, at its nesting level.
    pub fn insert_layer_above(&mut self, above: usize, name: &str) -> usize {
        let mut l = Layer::image(name, self.w, self.h, self.transparent);
        l.child_level = self.layers.get(above).map_or(0, |a| a.child_level);
        let at = (above + 1).min(self.layers.len());
        self.layers.insert(at, l);
        at
    }

    pub fn remove_layer(&mut self, i: usize) {
        if self.layers.len() > 1 && i < self.layers.len() {
            self.layers.remove(i);
        }
    }

    /// Swap layer `i` with its neighbour. Returns the new index.
    pub fn move_layer(&mut self, i: usize, up: bool) -> usize {
        let j = if up { i + 1 } else { i.wrapping_sub(1) };
        if i < self.layers.len() && j < self.layers.len() {
            self.layers.swap(i, j);
            j
        } else {
            i
        }
    }

    pub fn next_layer_name(&self) -> String {
        let mut n = self.layers.len() + 1;
        loop {
            let name = format!("Layer {n}");
            if !self.layers.iter().any(|l| l.name == name) {
                return name;
            }
            n += 1;
        }
    }

    /// Flatten all visible layers, bottom to top, with the given colors per slot.
    /// Straight alpha; fully transparent where nothing was painted.
    pub fn composite(&self, colors: &[Rgba; PALETTE_SIZE]) -> Vec<Rgba> {
        self.composite_with(colors, None)
    }

    /// `composite` with one layer's image swapped for another (tool previews).
    pub fn composite_with(&self, colors: &[Rgba; PALETTE_SIZE], replace: Option<(usize, &Image)>) -> Vec<Rgba> {
        let mut out = vec![Rgba::default(); self.w * self.h];
        self.composite_into(colors, replace, &Rect::new(0, 0, self.w as i32, self.h as i32), &mut out);
        out
    }

    /// Recomposite only the sprite-space rectangle `r` into `out`, a full `w*h` composite
    /// buffer (anything outside `r` is left alone). This is what keeps a pen stroke cheap:
    /// the few pixels a segment touched are recomputed, not the whole sprite.
    pub fn composite_into(&self, colors: &[Rgba; PALETTE_SIZE], replace: Option<(usize, &Image)>, r: &Rect, out: &mut [Rgba]) {
        let r = r.intersect(&Rect::new(0, 0, self.w as i32, self.h as i32));
        if r.is_empty() || out.len() != self.w * self.h {
            return;
        }
        for y in r.y..r.bottom() {
            let row = y as usize * self.w;
            out[row + r.x as usize..row + r.right() as usize].fill(Rgba::default());
        }
        for (i, layer) in self.layers.iter().enumerate() {
            if layer.kind != LayerKind::Image || !self.effective_visible(i) {
                continue;
            }
            let image = match replace {
                Some((k, img)) if k == i => img,
                _ => &layer.image,
            };
            if image.px.len() != out.len() {
                continue;
            }
            let op = layer.opacity as u32;
            for y in r.y..r.bottom() {
                let row = y as usize * self.w;
                for k in row + r.x as usize..row + r.right() as usize {
                    let idx = image.px[k];
                    if !layer.background && idx == self.transparent {
                        continue;
                    }
                    let c = colors[idx as usize % PALETTE_SIZE];
                    let a = c.a as u32 * op / 255;
                    if a == 0 {
                        continue;
                    }
                    out[k] = over(out[k], c, a);
                }
            }
        }
    }

    /// The greys the panel shows for each slot under a view mode (alpha kept from the palette).
    pub fn display_palette(&self, mode: ViewMode) -> [Rgba; PALETTE_SIZE] {
        let mut out = self.palette;
        match mode {
            ViewMode::Luma => {
                for c in out.iter_mut() {
                    let q = ((c.luma() as u32 + 8) / 17).min(15) as u8;
                    *c = Rgba { r: q * 17, g: q * 17, b: q * 17, a: c.a };
                }
            }
            ViewMode::Index => {
                let mut order: Vec<usize> = (0..PALETTE_SIZE).collect();
                order.sort_by_key(|&i| (self.palette[i].luma(), i));
                for (rank, &i) in order.iter().enumerate() {
                    let v = (rank * 17) as u8;
                    out[i] = Rgba { r: v, g: v, b: v, a: self.palette[i].a };
                }
            }
        }
        out
    }
}

/// `src` with alpha `a` (0-255) over `dst`, straight alpha.
fn over(dst: Rgba, src: Rgba, a: u32) -> Rgba {
    let da = dst.a as u32;
    let rest = da * (255 - a) / 255;
    let out_a = a + rest;
    if out_a == 0 {
        return Rgba::default();
    }
    let ch = |s: u8, d: u8| ((s as u32 * a + d as u32 * rest) / out_a) as u8;
    Rgba { r: ch(src.r, dst.r), g: ch(src.g, dst.g), b: ch(src.b, dst.b), a: out_a as u8 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sprite_is_white_paper() {
        let s = Sprite::new(4, 3);
        let px = s.composite(&s.palette);
        assert_eq!(px.len(), 12);
        assert!(px.iter().all(|&c| c == Rgba::rgb(255, 255, 255)));
    }

    #[test]
    fn transparent_index_on_upper_layer() {
        let mut s = Sprite::new(2, 1);
        let at = s.insert_layer_above(0, "top");
        s.layers[at].image.set(0, 0, 7);
        let px = s.composite(&s.palette);
        assert_eq!(px[0], DEFAULT_PALETTE[7]);
        assert_eq!(px[1], Rgba::rgb(255, 255, 255)); // index 0 on the top layer shows the paper
    }

    #[test]
    fn composite_into_patches_only_the_rect() {
        let mut s = Sprite::new(4, 3);
        let at = s.insert_layer_above(0, "top");
        for x in 0..4 {
            s.layers[at].image.set(x, 1, 3);
        }
        let full = s.composite(&s.palette);
        // Start from a stale buffer, patch the middle two columns of every row.
        let mut out = vec![Rgba::rgb(1, 2, 3); 12];
        s.composite_into(&s.palette, None, &Rect::new(1, 0, 2, 3), &mut out);
        for y in 0..3 {
            for x in 0..4 {
                let k = y * 4 + x;
                if (1..3).contains(&x) {
                    assert_eq!(out[k], full[k], "patched pixel {x},{y}");
                } else {
                    assert_eq!(out[k], Rgba::rgb(1, 2, 3), "untouched pixel {x},{y}");
                }
            }
        }
        // Out-of-range rects and wrong-sized buffers are ignored, never panic.
        s.composite_into(&s.palette, None, &Rect::new(-5, -5, 100, 100), &mut out);
        assert_eq!(out, full);
        let mut short = vec![Rgba::default(); 5];
        s.composite_into(&s.palette, None, &Rect::new(0, 0, 4, 3), &mut short);
        assert!(short.iter().all(|&c| c == Rgba::default()));
    }

    #[test]
    fn groups_hide_children() {
        let mut s = Sprite::new(1, 1);
        let mut g = Layer::image("g", 0, 0, 0);
        g.kind = LayerKind::Group;
        s.layers.push(g);
        let mut child = Layer::image("c", 1, 1, 5);
        child.child_level = 1;
        s.layers.push(child);
        assert_eq!(s.parent_of(2), Some(1));
        assert!(s.effective_visible(2));
        s.layers[1].visible = false;
        assert!(!s.effective_visible(2));
    }

    #[test]
    fn index_view_spreads_greys() {
        let s = Sprite::new(1, 1);
        let d = s.display_palette(ViewMode::Index);
        let mut greys: Vec<u8> = d.iter().map(|c| c.r).collect();
        greys.sort();
        greys.dedup();
        assert_eq!(greys.len(), 16);
        assert_eq!(d[0].r, 255); // white stays white
    }

    #[test]
    fn layer_ops() {
        let mut s = Sprite::new(1, 1);
        let a = s.insert_layer_above(0, "a");
        let b = s.insert_layer_above(a, "b");
        assert_eq!((a, b), (1, 2));
        assert_eq!(s.move_layer(b, false), 1);
        assert_eq!(s.layers[1].name, "b");
        s.remove_layer(1);
        assert_eq!(s.layers.len(), 2);
        assert_eq!(s.next_layer_name(), "Layer 3");
    }
}
