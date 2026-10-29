//! The canvas: where the sprite sits on screen, at what zoom, and how it is painted.
//!
//! Painting is clipped to a rectangle so a pen stroke only repaints the pixels it touched
//! (the e-ink panel then only refreshes that area). Transparent pixels show a checkerboard;
//! at 6x and above a grid line separates the pixels.

use librepixel::doc::{Image, Rgba, Sprite, ViewMode};
use librepixel::gfx::{grey16, grey565, Canvas, Rect, BLACK, WHITE};
use librepixel::select::Selection;

use crate::ui::{CANVAS, CANVAS_BG};

pub const ZOOMS: [i32; 12] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64];

/// Things drawn over the pixels: the selection (shifted while it is being moved) and the
/// selection being dragged out right now.
#[derive(Clone, Copy, Default)]
pub struct Marks<'a> {
    pub selection: Option<&'a Selection>,
    pub offset: (i32, i32),
    pub preview: Option<&'a Selection>,
    /// The first corner of a polygon selection being built (sprite pixel): tapping it closes
    /// the polygon, so it is marked with a white disc in a black ring.
    pub start: Option<(i32, i32)>,
}

/// Radius of the polygon start marker, screen pixels.
pub const MARKER_R: i32 = 11;

pub struct View {
    pub zoom_idx: usize,
    /// Screen position of sprite pixel (0,0), relative to the canvas origin.
    pub ox: i32,
    pub oy: i32,
    pub mode: ViewMode,
    pub grid: bool,
    /// A stroke is live: the area being repainted is kept pure black and white (grid included)
    /// so the panel can use its fast waveform; the true greys come back on pen-up.
    pub ink: bool,
    /// The composited sprite in display greys; patched in place as strokes land.
    cache: Option<Vec<Rgba>>,
    /// Reused buffer for shape previews, so dragging a shape allocates nothing.
    scratch: Vec<Rgba>,
}

impl View {
    pub fn new() -> View {
        View { zoom_idx: 3, ox: 0, oy: 0, mode: ViewMode::Index, grid: true, ink: false, cache: None, scratch: Vec::new() }
    }

    pub fn zoom(&self) -> i32 {
        ZOOMS[self.zoom_idx]
    }

    /// Call whenever the sprite or the view mode changed.
    pub fn invalidate(&mut self) {
        self.cache = None;
    }

    /// The sprite changed inside `r` (sprite space): recomposite just that part of the cache.
    /// Without a valid cache this is a no-op and the next `render` rebuilds it.
    pub fn patch(&mut self, sprite: &Sprite, r: &Rect) {
        let colors = sprite.display_palette(self.mode);
        match self.cache.as_mut() {
            Some(c) if c.len() == sprite.w * sprite.h => sprite.composite_into(&colors, None, r, c),
            _ => self.cache = None,
        }
    }

    /// `patch`, then show every pixel of `r` that a live stroke changed (where `layer` differs
    /// from `base`) as solid `ink` instead of its real grey.
    pub fn patch_ink(&mut self, sprite: &Sprite, r: &Rect, layer: &Image, base: &Image, ink: Rgba) {
        self.patch(sprite, r);
        let Some(c) = self.cache.as_mut() else { return };
        if layer.px.len() != c.len() || base.px.len() != c.len() {
            return;
        }
        let r = r.intersect(&Rect::new(0, 0, sprite.w as i32, sprite.h as i32));
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let k = y as usize * sprite.w + x as usize;
                if layer.px[k] != base.px[k] {
                    c[k] = ink;
                }
            }
        }
    }

    /// Largest zoom at which the whole sprite fits, centred.
    pub fn fit(&mut self, sw: usize, sh: usize) {
        let mut idx = 0;
        for (i, &z) in ZOOMS.iter().enumerate() {
            if sw as i32 * z <= CANVAS.w - 40 && sh as i32 * z <= CANVAS.h - 40 {
                idx = i;
            }
        }
        self.zoom_idx = idx;
        self.center(sw, sh);
    }

    pub fn center(&mut self, sw: usize, sh: usize) {
        let z = self.zoom();
        self.ox = (CANVAS.w - sw as i32 * z) / 2;
        self.oy = (CANVAS.h - sh as i32 * z) / 2;
    }

    /// Change zoom by `steps` levels, keeping the sprite point under `anchor` (screen) still.
    pub fn zoom_by(&mut self, steps: i32, anchor: (i32, i32)) -> bool {
        let new_idx = (self.zoom_idx as i32 + steps).clamp(0, ZOOMS.len() as i32 - 1) as usize;
        if new_idx == self.zoom_idx {
            return false;
        }
        let (ax, ay) = (anchor.0 - CANVAS.x, anchor.1 - CANVAS.y);
        let (old, new) = (self.zoom(), ZOOMS[new_idx]);
        self.ox = ax - (ax - self.ox) * new / old;
        self.oy = ay - (ay - self.oy) * new / old;
        self.zoom_idx = new_idx;
        true
    }

    /// Keep at least a corner of the sprite on the canvas.
    pub fn clamp_pan(&mut self, sw: usize, sh: usize) {
        let z = self.zoom();
        let (w, h) = (sw as i32 * z, sh as i32 * z);
        let keep = 80;
        self.ox = self.ox.clamp(keep - w, CANVAS.w - keep);
        self.oy = self.oy.clamp(keep - h, CANVAS.h - keep);
    }

    pub fn to_sprite(&self, x: i32, y: i32) -> (i32, i32) {
        let z = self.zoom();
        ((x - CANVAS.x - self.ox).div_euclid(z), (y - CANVAS.y - self.oy).div_euclid(z))
    }

    /// Screen rectangle of a sprite-space rectangle, one pixel wider all round for grid lines.
    pub fn to_screen(&self, r: &Rect) -> Rect {
        let z = self.zoom();
        Rect::new(CANVAS.x + self.ox + r.x * z - 1, CANVAS.y + self.oy + r.y * z - 1, r.w * z + 2, r.h * z + 2)
    }

    pub fn sprite_rect(&self, sw: usize, sh: usize) -> Rect {
        let z = self.zoom();
        Rect::new(CANVAS.x + self.ox, CANVAS.y + self.oy, sw as i32 * z, sh as i32 * z)
    }

    /// Screen centre of a sprite pixel.
    pub fn cell_center(&self, px: i32, py: i32) -> (i32, i32) {
        let z = self.zoom();
        (CANVAS.x + self.ox + px * z + z / 2, CANVAS.y + self.oy + py * z + z / 2)
    }

    /// Screen rectangle covered by the polygon start marker at a sprite pixel.
    pub fn marker_rect(&self, px: i32, py: i32) -> Rect {
        let (cx, cy) = self.cell_center(px, py);
        Rect::new(cx - MARKER_R - 1, cy - MARKER_R - 1, 2 * MARKER_R + 3, 2 * MARKER_R + 3)
    }

    /// Paint the part of the canvas inside `clip`. `preview` swaps one layer's image, for
    /// shapes being dragged out and pixels being moved.
    pub fn render(&mut self, cv: &mut Canvas, sprite: &Sprite, preview: Option<(usize, &Image)>, clip: &Rect, marks: Marks) {
        let area = clip.intersect(&CANVAS);
        if area.is_empty() {
            return;
        }
        cv.fill(&area, CANVAS_BG);

        let z = self.zoom();
        let sr = self.sprite_rect(sprite.w, sprite.h);
        let vis = sr.intersect(&area);
        let grid_color = if self.ink { BLACK } else { grey16(7) };
        if !vis.is_empty() {
            let n = sprite.w * sprite.h;
            let colors = sprite.display_palette(self.mode);
            // The sprite pixels under `vis`, inclusive: all that needs compositing for this clip.
            let sub = Rect::from_points(
                (vis.x - sr.x) / z,
                (vis.y - sr.y) / z,
                (vis.right() - 1 - sr.x) / z,
                (vis.bottom() - 1 - sr.y) / z,
            );
            let comp: &[Rgba] = match preview {
                Some(p) => {
                    if self.scratch.len() != n {
                        self.scratch = vec![Rgba::default(); n];
                    }
                    sprite.composite_into(&colors, Some(p), &sub, &mut self.scratch);
                    &self.scratch
                }
                None => {
                    if self.cache.as_ref().map_or(true, |c| c.len() != n) {
                        self.cache = Some(sprite.composite(&colors));
                    }
                    self.cache.as_deref().unwrap()
                }
            };
            let cell = (z / 2).max(4);
            let grid = self.grid && z >= 6;
            for y in vis.y..vis.bottom() {
                let ly = y - sr.y;
                let row = (ly / z) as usize * sprite.w;
                let on_grid_row = grid && ly % z == 0;
                let out = y as usize * cv.w;
                for x in vis.x..vis.right() {
                    let lx = x - sr.x;
                    let px = if on_grid_row || (grid && lx % z == 0) {
                        grid_color
                    } else {
                        let c = comp[row + (lx / z) as usize];
                        if c.a == 255 {
                            grey565(c.r)
                        } else {
                            let base: u32 = if ((lx / cell) + (ly / cell)) % 2 == 0 { 255 } else { 221 };
                            let a = c.a as u32;
                            grey565(((c.r as u32 * a + base * (255 - a)) / 255) as u8)
                        }
                    };
                    cv.px[out + x as usize] = px;
                }
            }
        }

        // A 2 px frame just outside the sprite.
        let f = Rect::new(sr.x - 2, sr.y - 2, sr.w + 4, sr.h + 4);
        for strip in [
            Rect::new(f.x, f.y, f.w, 2),
            Rect::new(f.x, f.bottom() - 2, f.w, 2),
            Rect::new(f.x, f.y, 2, f.h),
            Rect::new(f.right() - 2, f.y, 2, f.h),
        ] {
            cv.fill(&strip.intersect(&area), BLACK);
        }

        if let Some(s) = marks.selection {
            self.outline(cv, s, marks.offset, &sr, &area);
        }
        if let Some(s) = marks.preview {
            self.outline(cv, s, (0, 0), &sr, &area);
        }
        if let Some((px, py)) = marks.start {
            let (cx, cy) = self.cell_center(px, py);
            let r = self.marker_rect(px, py).intersect(&area);
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    let d2 = (x - cx).pow(2) + (y - cy).pow(2);
                    if d2 <= MARKER_R * MARKER_R {
                        cv.px[y as usize * cv.w + x as usize] = if d2 <= (MARKER_R - 3).pow(2) { WHITE } else { BLACK };
                    }
                }
            }
        }
    }

    /// A dashed black-and-white border along the edge of the selected pixels, drawn just
    /// inside them, clipped to `area`. Static dashes: marching ants would never stop
    /// refreshing an e-ink panel.
    fn outline(&self, cv: &mut Canvas, sel: &Selection, offset: (i32, i32), sr: &Rect, area: &Rect) {
        if sel.is_empty() {
            return;
        }
        let z = self.zoom();
        let t = if z >= 4 { 2 } else { 1 };
        let b = sel.bounds;
        // Pixels whose cells can touch `area`, in the selection's own coordinates.
        let (ox, oy) = offset;
        let sx0 = (((area.x - sr.x).div_euclid(z)) - ox).max(b.x);
        let sy0 = (((area.y - sr.y).div_euclid(z)) - oy).max(b.y);
        let sx1 = (((area.right() - 1 - sr.x).div_euclid(z)) - ox).min(b.right() - 1);
        let sy1 = (((area.bottom() - 1 - sr.y).div_euclid(z)) - oy).min(b.bottom() - 1);
        let dash = |k: i32| if (k / 4) % 2 == 0 { BLACK } else { WHITE };
        for py in sy0..=sy1 {
            for px in sx0..=sx1 {
                if !sel.contains(px, py) {
                    continue;
                }
                let (cx, cy) = (sr.x + (px + ox) * z, sr.y + (py + oy) * z);
                let edges = [
                    (!sel.contains(px, py - 1), Rect::new(cx, cy, z, t), true),
                    (!sel.contains(px, py + 1), Rect::new(cx, cy + z - t, z, t), true),
                    (!sel.contains(px - 1, py), Rect::new(cx, cy, t, z), false),
                    (!sel.contains(px + 1, py), Rect::new(cx + z - t, cy, t, z), false),
                ];
                for (open, r, horizontal) in edges {
                    if !open {
                        continue;
                    }
                    let r = r.intersect(area);
                    for y in r.y..r.bottom() {
                        for x in r.x..r.right() {
                            cv.px[y as usize * cv.w + x as usize] = dash(if horizontal { x } else { y });
                        }
                    }
                }
            }
        }
    }
}
