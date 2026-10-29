//! Export the flattened sprite as PNG or JPG (with nearest-neighbour upscaling, so pixels stay
//! square), produce the PDF bytes, and import a PNG as a new sprite.

use std::io::{self, BufReader, BufWriter};
use std::path::Path;

use crate::doc::{nearest_index, Layer, Rgba, Sprite, DEFAULT_PALETTE, PALETTE_SIZE};

fn other<E: std::error::Error + Send + Sync + 'static>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}

/// The sprite with its true colors, straight alpha.
pub fn flatten(sprite: &Sprite) -> Vec<Rgba> {
    sprite.composite(&sprite.palette)
}

pub fn scale_nearest(src: &[Rgba], w: usize, h: usize, f: usize) -> Vec<Rgba> {
    let f = f.max(1);
    let mut out = Vec::with_capacity(w * h * f * f);
    for y in 0..h * f {
        let row = &src[(y / f) * w..(y / f + 1) * w];
        for x in 0..w * f {
            out.push(row[x / f]);
        }
    }
    out
}

pub fn rgba_bytes(px: &[Rgba]) -> Vec<u8> {
    px.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect()
}

/// RGB with transparency flattened onto white (for JPG and PDF).
pub fn rgb_over_white(px: &[Rgba]) -> Vec<u8> {
    px.iter()
        .flat_map(|c| {
            let a = c.a as u32;
            let ch = |v: u8| ((v as u32 * a + 255 * (255 - a)) / 255) as u8;
            [ch(c.r), ch(c.g), ch(c.b)]
        })
        .collect()
}

pub fn write_png(path: &Path, sprite: &Sprite, scale: usize) -> io::Result<()> {
    let px = scale_nearest(&flatten(sprite), sprite.w, sprite.h, scale);
    let (w, h) = (sprite.w * scale.max(1), sprite.h * scale.max(1));
    let file = BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(other)?;
    writer.write_image_data(&rgba_bytes(&px)).map_err(other)?;
    writer.finish().map_err(other)
}

pub fn write_jpg(path: &Path, sprite: &Sprite, scale: usize, quality: u8) -> io::Result<()> {
    let px = scale_nearest(&flatten(sprite), sprite.w, sprite.h, scale);
    let (w, h) = (sprite.w * scale.max(1), sprite.h * scale.max(1));
    if w > u16::MAX as usize || h > u16::MAX as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "image too large for JPG"));
    }
    let enc = jpeg_encoder::Encoder::new_file(path, quality).map_err(other)?;
    enc.encode(&rgb_over_white(&px), w as u16, h as u16, jpeg_encoder::ColorType::Rgb).map_err(other)
}

/// A one-page PDF of the sprite, upscaled so it stays crisp at the tablet's resolution.
pub fn pdf_bytes(sprite: &Sprite) -> Vec<u8> {
    let long = sprite.w.max(sprite.h).max(1);
    let f = (1120 / long).max(1);
    let px = scale_nearest(&flatten(sprite), sprite.w, sprite.h, f);
    crate::pdf::make_pdf(&rgb_over_white(&px), sprite.w * f, sprite.h * f)
}

pub fn write_pdf(path: &Path, sprite: &Sprite) -> io::Result<()> {
    std::fs::write(path, pdf_bytes(sprite))
}

/// Import a PNG: its 15 most used opaque colors become slots 1-15 (slot 0 stays white), every
/// other color maps to the nearest slot. Transparent pixels make it a transparent layer.
pub fn read_png(path: &Path) -> io::Result<Sprite> {
    let mut decoder = png::Decoder::new(BufReader::new(std::fs::File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(other)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(other)?;
    let (w, h) = (info.width as usize, info.height as usize);
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unsupported image size {w}x{h}")));
    }
    let bytes = &buf[..info.buffer_size()];
    let px: Vec<Rgba> = match info.color_type {
        png::ColorType::Rgba => bytes.chunks_exact(4).map(|p| Rgba { r: p[0], g: p[1], b: p[2], a: p[3] }).collect(),
        png::ColorType::Rgb => bytes.chunks_exact(3).map(|p| Rgba::rgb(p[0], p[1], p[2])).collect(),
        png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).map(|p| Rgba { r: p[0], g: p[0], b: p[0], a: p[1] }).collect(),
        png::ColorType::Grayscale => bytes.iter().map(|&v| Rgba::grey(v)).collect(),
        png::ColorType::Indexed => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "indexed PNG was not expanded"));
        }
    };
    Ok(sprite_from_rgba(&px, w, h))
}

pub fn sprite_from_rgba(px: &[Rgba], w: usize, h: usize) -> Sprite {
    let has_alpha = px.iter().any(|c| c.a < 128);
    // Most used opaque colors first.
    let mut counts: Vec<(Rgba, usize)> = Vec::new();
    for c in px.iter().filter(|c| c.a >= 128) {
        let key = Rgba::rgb(c.r, c.g, c.b);
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some(e) => e.1 += 1,
            None => counts.push((key, 1)),
        }
        if counts.len() > 4096 {
            break;
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    let mut palette = DEFAULT_PALETTE;
    for (slot, (color, _)) in palette.iter_mut().skip(1).zip(counts.iter()) {
        *slot = *color;
    }
    let filled = counts.len().min(PALETTE_SIZE - 1) + 1; // slots that carry the image's colors
    let mut sprite = Sprite::new(w, h);
    sprite.palette = palette;
    sprite.layers[0] = Layer::image(if has_alpha { "Layer 1" } else { "Background" }, w, h, 0);
    sprite.layers[0].background = !has_alpha;
    let from = if has_alpha { 1 } else { 0 };
    for (dst, c) in sprite.layers[0].image.px.iter_mut().zip(px) {
        *dst = if c.a < 128 { 0 } else { nearest_index(&palette[..filled], Rgba::rgb(c.r, c.g, c.b), from) };
    }
    sprite
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_and_flattening() {
        let mut s = Sprite::new(2, 1);
        s.layers[0].image.set(1, 0, 1);
        let px = flatten(&s);
        assert_eq!(px, vec![Rgba::rgb(255, 255, 255), DEFAULT_PALETTE[1]]);
        let big = scale_nearest(&px, 2, 1, 3);
        assert_eq!(big.len(), 18);
        assert_eq!(big[2], px[0]);
        assert_eq!(big[3], px[1]);
        assert_eq!(rgb_over_white(&[Rgba { r: 0, g: 0, b: 0, a: 0 }]), vec![255, 255, 255]);
    }

    #[test]
    fn png_round_trip_through_files() {
        let dir = std::env::temp_dir().join(format!("rm2-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Sprite::new(3, 2);
        s.layers[0].image.set(0, 0, 7);
        s.layers[0].image.set(2, 1, 9);
        let p = dir.join("t.png");
        write_png(&p, &s, 2).unwrap();
        let back = read_png(&p).unwrap();
        assert_eq!((back.w, back.h), (6, 4));
        assert!(back.layers[0].background);
        // Colors survive: the red pixel's slot has the red color.
        let idx = back.layers[0].image.get(0, 0).unwrap();
        assert_eq!(back.palette[idx as usize], DEFAULT_PALETTE[7]);
        write_jpg(&dir.join("t.jpg"), &s, 4, 90).unwrap();
        assert!(std::fs::metadata(dir.join("t.jpg")).unwrap().len() > 100);
        let pdf = pdf_bytes(&s);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
