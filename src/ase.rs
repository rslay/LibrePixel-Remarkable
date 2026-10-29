//! The `.ase` / `.aseprite` file format (docs/ase-format.txt), restricted to what this editor
//! keeps: one frame, image and group layers, a 16-entry palette, indexed pixels.
//!
//! Reading accepts any file Aseprite or LibreSprite writes: extra frames are dropped (with a
//! warning), RGBA and grayscale pixels are remapped to the palette, indices past slot 15 are
//! remapped to the nearest of the first 16 colors. Writing always produces an indexed,
//! single-frame file that LibreSprite opens unchanged.

use std::io;
use std::path::Path;

use crate::doc::{nearest_index, Image, Layer, LayerKind, Rgba, Sprite, DEFAULT_PALETTE, PALETTE_SIZE};

const HEADER_MAGIC: u16 = 0xA5E0;
const FRAME_MAGIC: u16 = 0xF1FA;
const CHUNK_OLD_PALETTE: u16 = 0x0004;
const CHUNK_OLD_PALETTE_64: u16 = 0x0011;
const CHUNK_LAYER: u16 = 0x2004;
const CHUNK_CEL: u16 = 0x2005;
const CHUNK_PALETTE: u16 = 0x2019;

const LAYER_VISIBLE: u16 = 1;
const LAYER_EDITABLE: u16 = 2;
const LAYER_BACKGROUND: u16 = 8;

pub struct Loaded {
    pub sprite: Sprite,
    pub warnings: Vec<String>,
}

pub fn load(path: &Path) -> io::Result<Loaded> {
    let data = std::fs::read(path)?;
    read(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn save(path: &Path, sprite: &Sprite) -> io::Result<()> {
    std::fs::write(path, write(sprite))
}

// ---- reading ---------------------------------------------------------------

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn need(&self, n: usize) -> Result<(), String> {
        if self.pos + n > self.data.len() { Err("file is truncated".to_string()) } else { Ok(()) }
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        self.need(n)?;
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.bytes(n).map(|_| ())
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }
    fn seek(&mut self, pos: usize) -> Result<(), String> {
        if pos > self.data.len() {
            return Err("chunk runs past the end of the file".to_string());
        }
        self.pos = pos;
        Ok(())
    }
}

struct Cel {
    layer: usize,
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    pixels: Vec<u8>, // in the file's pixel format
}

pub fn read(data: &[u8]) -> Result<Loaded, String> {
    let mut c = Cursor { data, pos: 0 };
    let _file_size = c.u32()?;
    if c.u16()? != HEADER_MAGIC {
        return Err("not an .ase file (bad magic number)".to_string());
    }
    let frames = c.u16()?;
    let w = c.u16()? as usize;
    let h = c.u16()? as usize;
    let depth = c.u16()?;
    let flags = c.u32()?;
    let _speed = c.u16()?;
    c.skip(8)?;
    let transparent = c.u8()?;
    c.skip(3)?;
    let _ncolors = c.u16()?;
    c.skip(94)?;

    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return Err(format!("unsupported sprite size {w}x{h}"));
    }
    let bpp = match depth {
        8 => 1,
        16 => 2,
        32 => 4,
        d => return Err(format!("unsupported color depth {d}")),
    };
    let mut warnings = Vec::new();
    if frames > 1 {
        warnings.push(format!("{frames} frames in file; only the first is kept"));
    }
    if frames == 0 {
        return Err("file has no frames".to_string());
    }

    // First frame only.
    let _frame_bytes = c.u32()?;
    if c.u16()? != FRAME_MAGIC {
        return Err("bad frame header".to_string());
    }
    let old_count = c.u16()?;
    let _duration = c.u16()?;
    c.skip(2)?;
    let new_count = c.u32()?;
    let nchunks = if old_count == 0xFFFF && new_count != 0 { new_count as usize } else { old_count as usize };

    let mut file_palette: Vec<Rgba> = Vec::new();
    let mut new_palette_seen = false;
    let mut layers: Vec<Layer> = Vec::new();
    let mut cels: Vec<Cel> = Vec::new();

    for _ in 0..nchunks {
        let start = c.pos;
        let size = c.u32()? as usize;
        let kind = c.u16()?;
        if size < 6 {
            return Err("bad chunk size".to_string());
        }
        let end = start + size;
        match kind {
            CHUNK_OLD_PALETTE | CHUNK_OLD_PALETTE_64 => {
                if !new_palette_seen {
                    read_old_palette(&mut c, &mut file_palette, kind == CHUNK_OLD_PALETTE_64)?;
                }
            }
            CHUNK_PALETTE => {
                new_palette_seen = true;
                read_palette(&mut c, &mut file_palette)?;
            }
            CHUNK_LAYER => layers.push(read_layer(&mut c, flags & 1 != 0, w, h, transparent, &mut warnings)?),
            CHUNK_CEL => {
                if let Some(cel) = read_cel(&mut c, bpp, end, &mut warnings)? {
                    cels.push(cel);
                }
            }
            _ => {}
        }
        c.seek(end)?;
    }

    // The editor's palette: the file's first 16 colors, defaults for the rest.
    let mut palette = DEFAULT_PALETTE;
    for (slot, color) in palette.iter_mut().zip(&file_palette) {
        *slot = *color;
    }
    let transparent = if (transparent as usize) < PALETTE_SIZE {
        transparent
    } else {
        warnings.push(format!("transparent index {transparent} is past slot 15; using 0"));
        0
    };

    // Pixels to indices.
    let mut remapped = 0usize;
    for cel in &cels {
        let Some(layer) = layers.get_mut(cel.layer) else {
            warnings.push(format!("cel for missing layer {}", cel.layer));
            continue;
        };
        if layer.kind != LayerKind::Image {
            continue;
        }
        for row in 0..cel.h {
            let y = cel.y + row as i32;
            for col in 0..cel.w {
                let x = cel.x + col as i32;
                if !layer.image.in_bounds(x, y) {
                    continue;
                }
                let p = &cel.pixels[(row * cel.w + col) * bpp..];
                let idx = match bpp {
                    1 => {
                        let i = p[0];
                        if (i as usize) < PALETTE_SIZE {
                            i
                        } else {
                            remapped += 1;
                            let c = file_palette.get(i as usize).copied().unwrap_or(Rgba::grey(0));
                            nearest_index(&palette, c, 0)
                        }
                    }
                    2 => {
                        remapped += 1;
                        if p[1] < 128 && !layer.background { transparent } else { nearest_index(&palette, Rgba::grey(p[0]), 0) }
                    }
                    _ => {
                        remapped += 1;
                        if p[3] < 128 && !layer.background {
                            transparent
                        } else {
                            nearest_index(&palette, Rgba::rgb(p[0], p[1], p[2]), 0)
                        }
                    }
                };
                layer.image.set(x, y, idx);
            }
        }
    }
    if remapped > 0 {
        warnings.push(format!("{remapped} pixels were mapped to the nearest of the 16 palette colors"));
    }
    if layers.is_empty() {
        warnings.push("file has no layers; added a background".to_string());
        let mut bg = Layer::image("Background", w, h, 0);
        bg.background = true;
        layers.push(bg);
    }

    Ok(Loaded { sprite: Sprite { w, h, palette, transparent, layers }, warnings })
}

fn read_old_palette(c: &mut Cursor, palette: &mut Vec<Rgba>, six_bit: bool) -> Result<(), String> {
    let packets = c.u16()?;
    let mut index = 0usize;
    for _ in 0..packets {
        index += c.u8()? as usize;
        let count = match c.u8()? {
            0 => 256,
            n => n as usize,
        };
        for _ in 0..count {
            let (mut r, mut g, mut b) = (c.u8()?, c.u8()?, c.u8()?);
            if six_bit {
                r = r.saturating_mul(4);
                g = g.saturating_mul(4);
                b = b.saturating_mul(4);
            }
            if index < 256 {
                if palette.len() <= index {
                    palette.resize(index + 1, Rgba::grey(0));
                }
                palette[index] = Rgba::rgb(r, g, b);
            }
            index += 1;
        }
    }
    Ok(())
}

fn read_palette(c: &mut Cursor, palette: &mut Vec<Rgba>) -> Result<(), String> {
    let size = c.u32()? as usize;
    let from = c.u32()? as usize;
    let to = c.u32()? as usize;
    c.skip(8)?;
    if size > 256 || from > to || to >= 256 {
        return Err("bad palette chunk".to_string());
    }
    if palette.len() < size {
        palette.resize(size, Rgba::grey(0));
    }
    for i in from..=to {
        let flags = c.u16()?;
        let color = Rgba { r: c.u8()?, g: c.u8()?, b: c.u8()?, a: c.u8()? };
        if flags & 1 != 0 {
            c.string()?; // color name
        }
        if palette.len() <= i {
            palette.resize(i + 1, Rgba::grey(0));
        }
        palette[i] = color;
    }
    Ok(())
}

fn read_layer(c: &mut Cursor, opacity_valid: bool, w: usize, h: usize, fill: u8, warnings: &mut Vec<String>) -> Result<Layer, String> {
    let flags = c.u16()?;
    let kind = c.u16()?;
    let child_level = c.u16()?;
    let _default_w = c.u16()?;
    let _default_h = c.u16()?;
    let blend_mode = c.u16()?;
    let opacity = c.u8()?;
    c.skip(3)?;
    if kind == 2 {
        c.u32()?; // tileset index of a tilemap layer
        warnings.push("tilemap layer treated as an empty image layer".to_string());
    }
    let name = c.string()?;
    let kind = if kind == 1 { LayerKind::Group } else { LayerKind::Image };
    Ok(Layer {
        name,
        kind,
        child_level,
        visible: flags & LAYER_VISIBLE != 0,
        locked: flags & LAYER_EDITABLE == 0,
        background: flags & LAYER_BACKGROUND != 0,
        opacity: if opacity_valid { opacity } else { 255 },
        blend_mode,
        image: if kind == LayerKind::Image { Image::new(w, h, fill) } else { Image::new(0, 0, 0) },
    })
}

fn read_cel(c: &mut Cursor, bpp: usize, end: usize, warnings: &mut Vec<String>) -> Result<Option<Cel>, String> {
    let layer = c.u16()? as usize;
    let x = c.i16()? as i32;
    let y = c.i16()? as i32;
    let _opacity = c.u8()?;
    let kind = c.u16()?;
    c.skip(7)?;
    match kind {
        0 => {
            let w = c.u16()? as usize;
            let h = c.u16()? as usize;
            let pixels = c.bytes(w * h * bpp)?.to_vec();
            Ok(Some(Cel { layer, x, y, w, h, pixels }))
        }
        2 => {
            let w = c.u16()? as usize;
            let h = c.u16()? as usize;
            let packed = c.bytes(end.saturating_sub(c.pos))?;
            let pixels = miniz_oxide::inflate::decompress_to_vec_zlib(packed)
                .map_err(|_| "cel data is not valid zlib".to_string())?;
            if pixels.len() < w * h * bpp {
                return Err("cel data is too short".to_string());
            }
            Ok(Some(Cel { layer, x, y, w, h, pixels }))
        }
        1 => {
            warnings.push("linked cel ignored".to_string());
            Ok(None)
        }
        _ => {
            warnings.push(format!("cel type {kind} not supported; skipped"));
            Ok(None)
        }
    }
}

// ---- writing ---------------------------------------------------------------

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_string(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(u16::MAX as usize);
    put_u16(out, n as u16);
    out.extend_from_slice(&b[..n]);
}
fn chunk(out: &mut Vec<u8>, kind: u16, body: &[u8]) {
    put_u32(out, (body.len() + 6) as u32);
    put_u16(out, kind);
    out.extend_from_slice(body);
}

pub fn write(sprite: &Sprite) -> Vec<u8> {
    let mut out = Vec::new();
    // Header (128 bytes).
    put_u32(&mut out, 0); // file size, patched below
    put_u16(&mut out, HEADER_MAGIC);
    put_u16(&mut out, 1); // frames
    put_u16(&mut out, sprite.w as u16);
    put_u16(&mut out, sprite.h as u16);
    put_u16(&mut out, 8); // indexed
    put_u32(&mut out, 1); // layer opacity is valid
    put_u16(&mut out, 100); // speed (deprecated)
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    out.push(sprite.transparent);
    out.extend_from_slice(&[0; 3]);
    put_u16(&mut out, PALETTE_SIZE as u16);
    out.extend_from_slice(&[0; 94]);
    debug_assert_eq!(out.len(), 128);

    let image_layers = sprite.layers.iter().filter(|l| l.kind == LayerKind::Image).count();
    let nchunks = 2 + sprite.layers.len() + image_layers;

    // Frame header (16 bytes).
    let frame_start = out.len();
    put_u32(&mut out, 0); // frame bytes, patched below
    put_u16(&mut out, FRAME_MAGIC);
    put_u16(&mut out, nchunks.min(0xFFFF) as u16);
    put_u16(&mut out, 100); // duration
    out.extend_from_slice(&[0; 2]);
    put_u32(&mut out, nchunks as u32);

    // Palettes: the old chunk for old readers, the new one for everyone else.
    let mut body = Vec::new();
    put_u16(&mut body, 1); // packets
    body.push(0); // skip
    body.push(PALETTE_SIZE as u8);
    for c in &sprite.palette {
        body.extend_from_slice(&[c.r, c.g, c.b]);
    }
    chunk(&mut out, CHUNK_OLD_PALETTE, &body);

    let mut body = Vec::new();
    put_u32(&mut body, PALETTE_SIZE as u32);
    put_u32(&mut body, 0);
    put_u32(&mut body, PALETTE_SIZE as u32 - 1);
    body.extend_from_slice(&[0; 8]);
    for c in &sprite.palette {
        put_u16(&mut body, 0);
        body.extend_from_slice(&[c.r, c.g, c.b, c.a]);
    }
    chunk(&mut out, CHUNK_PALETTE, &body);

    // Layers, bottom to top.
    for l in &sprite.layers {
        let mut body = Vec::new();
        let mut flags = if l.locked { 0 } else { LAYER_EDITABLE };
        if l.visible {
            flags |= LAYER_VISIBLE;
        }
        if l.background {
            flags |= LAYER_BACKGROUND;
        }
        put_u16(&mut body, flags);
        put_u16(&mut body, if l.kind == LayerKind::Group { 1 } else { 0 });
        put_u16(&mut body, l.child_level);
        put_u16(&mut body, sprite.w as u16);
        put_u16(&mut body, sprite.h as u16);
        put_u16(&mut body, if l.kind == LayerKind::Group { 0 } else { l.blend_mode });
        body.push(l.opacity);
        body.extend_from_slice(&[0; 3]);
        put_string(&mut body, &l.name);
        chunk(&mut out, CHUNK_LAYER, &body);
    }

    // One compressed cel per image layer, covering the whole sprite.
    for (i, l) in sprite.layers.iter().enumerate() {
        if l.kind != LayerKind::Image {
            continue;
        }
        let mut body = Vec::new();
        put_u16(&mut body, i as u16);
        put_u16(&mut body, 0); // x
        put_u16(&mut body, 0); // y
        body.push(255); // opacity
        put_u16(&mut body, 2); // compressed
        body.extend_from_slice(&[0; 7]);
        put_u16(&mut body, sprite.w as u16);
        put_u16(&mut body, sprite.h as u16);
        body.extend_from_slice(&miniz_oxide::deflate::compress_to_vec_zlib(&l.image.px, 6));
        chunk(&mut out, CHUNK_CEL, &body);
    }

    let frame_len = (out.len() - frame_start) as u32;
    out[frame_start..frame_start + 4].copy_from_slice(&frame_len.to_le_bytes());
    let total = out.len() as u32;
    out[0..4].copy_from_slice(&total.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::ViewMode;

    fn sample() -> Sprite {
        let mut s = Sprite::new(5, 4);
        s.layers[0].image.set(1, 1, 3);
        let top = s.insert_layer_above(0, "Ink");
        s.layers[top].image.set(2, 2, 9);
        s.layers[top].opacity = 200;
        s.layers[top].visible = false;
        s.layers[top].locked = true;
        s.palette[5] = Rgba::rgb(1, 2, 3);
        s
    }

    #[test]
    fn round_trip() {
        let s = sample();
        let bytes = write(&s);
        assert_eq!(&bytes[4..6], &[0xE0, 0xA5]);
        assert_eq!(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize, bytes.len());
        let back = read(&bytes).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back.sprite, s);
        // The view mode does not depend on the file, but the palette must survive exactly.
        assert_eq!(back.sprite.display_palette(ViewMode::Index), s.display_palette(ViewMode::Index));
    }

    #[test]
    fn rejects_garbage() {
        assert!(read(b"not a sprite").is_err());
        let mut bytes = write(&Sprite::new(2, 2));
        bytes[4] = 0;
        assert!(read(&bytes).is_err());
    }

    #[test]
    fn rgba_file_is_remapped() {
        // Build a 2x1 RGBA, one-frame file by hand: header, frame, layer, raw cel.
        let mut out = Vec::new();
        put_u32(&mut out, 0);
        put_u16(&mut out, HEADER_MAGIC);
        put_u16(&mut out, 1);
        put_u16(&mut out, 2);
        put_u16(&mut out, 1);
        put_u16(&mut out, 32);
        put_u32(&mut out, 1);
        put_u16(&mut out, 100);
        put_u32(&mut out, 0);
        put_u32(&mut out, 0);
        out.push(0);
        out.extend_from_slice(&[0; 3]);
        put_u16(&mut out, 0);
        out.extend_from_slice(&[0; 94]);
        let frame_start = out.len();
        put_u32(&mut out, 0);
        put_u16(&mut out, FRAME_MAGIC);
        put_u16(&mut out, 2);
        put_u16(&mut out, 100);
        out.extend_from_slice(&[0; 6]);
        let mut body = Vec::new();
        put_u16(&mut body, LAYER_VISIBLE | LAYER_EDITABLE);
        put_u16(&mut body, 0);
        put_u16(&mut body, 0);
        put_u16(&mut body, 2);
        put_u16(&mut body, 1);
        put_u16(&mut body, 0);
        body.push(255);
        body.extend_from_slice(&[0; 3]);
        put_string(&mut body, "L");
        chunk(&mut out, CHUNK_LAYER, &body);
        let mut body = Vec::new();
        put_u16(&mut body, 0);
        put_u16(&mut body, 0);
        put_u16(&mut body, 0);
        body.push(255);
        put_u16(&mut body, 0);
        body.extend_from_slice(&[0; 7]);
        put_u16(&mut body, 2);
        put_u16(&mut body, 1);
        body.extend_from_slice(&[208, 70, 72, 255, 0, 0, 0, 0]); // DB16 red, then transparent
        chunk(&mut out, CHUNK_CEL, &body);
        let flen = (out.len() - frame_start) as u32;
        out[frame_start..frame_start + 4].copy_from_slice(&flen.to_le_bytes());
        let total = out.len() as u32;
        out[0..4].copy_from_slice(&total.to_le_bytes());

        let loaded = read(&out).unwrap();
        assert_eq!(loaded.sprite.layers[0].image.px, vec![7, 0]);
        assert!(!loaded.warnings.is_empty());
    }
}
