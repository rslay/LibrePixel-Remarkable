//! A minimal PDF writer: one page the size of the reMarkable 2 screen with one RGB image
//! centred on it, zlib-compressed, interpolation off so pixels stay sharp.

/// Page size in points: the rM2 panel (1404x1872 at 226 dpi).
pub const PAGE_W: f64 = 445.7;
pub const PAGE_H: f64 = 594.3;

pub fn make_pdf(rgb: &[u8], w: usize, h: usize) -> Vec<u8> {
    assert_eq!(rgb.len(), w * h * 3);
    let image = miniz_oxide::deflate::compress_to_vec_zlib(rgb, 6);

    // Fit the image inside the page with an 8% margin, keeping its aspect ratio.
    let (box_w, box_h) = (PAGE_W * 0.84, PAGE_H * 0.84);
    let scale = (box_w / w as f64).min(box_h / h as f64);
    let (dw, dh) = (w as f64 * scale, h as f64 * scale);
    let (dx, dy) = ((PAGE_W - dw) / 2.0, (PAGE_H - dh) / 2.0);
    let content = format!("q\n{dw:.2} 0 0 {dh:.2} {dx:.2} {dy:.2} cm\n/Im0 Do\nQ\n");

    let mut out: Vec<u8> = Vec::with_capacity(image.len() + 1024);
    let mut offsets = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, &mut offsets, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        &mut offsets,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W:.1} {PAGE_H:.1}] \
             /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content.as_bytes());
    stream.extend_from_slice(b"endstream");
    obj(&mut out, &mut offsets, &stream);
    let mut stream = format!(
        "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB \
         /BitsPerComponent 8 /Interpolate false /Filter /FlateDecode /Length {} >>\nstream\n",
        image.len()
    )
    .into_bytes();
    stream.extend_from_slice(&image);
    stream.extend_from_slice(b"\nendstream");
    obj(&mut out, &mut offsets, &stream);

    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", offsets.len() + 1).as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(hay: &[u8], needle: &[u8], from: usize) -> usize {
        hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from).unwrap()
    }

    fn rfind(hay: &[u8], needle: &[u8]) -> usize {
        hay.windows(needle.len()).rposition(|w| w == needle).unwrap()
    }

    #[test]
    fn xref_offsets_point_at_objects() {
        let pdf = make_pdf(&[255, 0, 0, 0, 255, 0], 2, 1);
        // startxref must point at the xref keyword (not the one inside "startxref").
        let xref = rfind(&pdf, b"\nxref\n") + 1;
        let sx = rfind(&pdf, b"startxref\n") + 10;
        let end = find(&pdf, b"\n", sx);
        let start: usize = std::str::from_utf8(&pdf[sx..end]).unwrap().trim().parse().unwrap();
        assert_eq!(start, xref);
        // Each 20-byte entry (after the free entry 0) must point at "N 0 obj".
        let first_entry = find(&pdf, b"\n", find(&pdf, b"\n", xref) + 1) + 1;
        for i in 1..=5 {
            let line = &pdf[first_entry + i * 20..first_entry + i * 20 + 20];
            let off: usize = std::str::from_utf8(&line[..10]).unwrap().parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{i} 0 obj").as_bytes()), "object {i}");
            assert_eq!(&line[17..], b"n \n");
        }
    }
}
