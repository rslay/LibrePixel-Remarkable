//! Put a PDF into the tablet's own document library so it shows up next to the notebooks.
//!
//! xochitl keeps every document as `<uuid>.pdf` + `<uuid>.metadata` + `<uuid>.content` in
//! `/home/root/.local/share/remarkable/xochitl/`. It scans that directory when it starts, so
//! a new document appears after xochitl restarts (or the tablet reboots).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_DIR: &str = "/home/root/.local/share/remarkable/xochitl";

/// The library directory if this looks like a tablet (or `RM2_XOCHITL_DIR` is set).
pub fn library_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("RM2_XOCHITL_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(DEFAULT_DIR));
    if dir.is_dir() { Some(dir) } else { None }
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    #[cfg(unix)]
    let filled = {
        use std::io::Read;
        std::fs::File::open("/dev/urandom").map(|mut f| f.read_exact(&mut b).is_ok()).unwrap_or(false)
    };
    #[cfg(not(unix))]
    let filled = false;
    if !filled {
        // Not cryptographic, only has to be unique on one device.
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        let mut h = RandomState::new().build_hasher();
        h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let a = h.finish().to_le_bytes();
        let mut h2 = RandomState::new().build_hasher();
        h2.write_u64(std::process::id() as u64);
        let c = h2.finish().to_le_bytes();
        b[..8].copy_from_slice(&a);
        b[8..].copy_from_slice(&c);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// Write the PDF and its sidecar files. Returns the document's uuid.
pub fn add_pdf(dir: &Path, visible_name: &str, pdf: &[u8]) -> io::Result<String> {
    let id = uuid_v4();
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let metadata = format!(
        "{{\"deleted\":false,\"lastModified\":\"{now_ms}\",\"lastOpened\":\"\",\"lastOpenedPage\":0,\
         \"metadatamodified\":false,\"modified\":false,\"parent\":\"\",\"pinned\":false,\"synced\":false,\
         \"type\":\"DocumentType\",\"version\":1,\"visibleName\":{}}}\n",
        json_string(visible_name)
    );
    let content = "{\"extraMetadata\":{},\"fileType\":\"pdf\",\"fontName\":\"\",\"lastOpenedPage\":0,\
                   \"lineHeight\":-1,\"margins\":100,\"orientation\":\"portrait\",\"pageCount\":1,\
                   \"textScale\":1,\"transform\":{\"m11\":1,\"m12\":0,\"m13\":0,\"m21\":0,\"m22\":1,\
                   \"m23\":0,\"m31\":0,\"m32\":0,\"m33\":1}}\n";
    std::fs::write(dir.join(format!("{id}.pdf")), pdf)?;
    std::fs::write(dir.join(format!("{id}.metadata")), metadata)?;
    std::fs::write(dir.join(format!("{id}.content")), content)?;
    std::fs::write(dir.join(format!("{id}.pagedata")), "Blank\n")?;
    Ok(id)
}

/// Restart xochitl in the background. This ends the current app too (AppLoad lives inside
/// xochitl), so call it last and only when the user asked for it.
pub fn restart_xochitl() -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::process::{Command, Stdio};
        let script = "sleep 1; systemctl restart xochitl";
        let direct = Command::new("setsid")
            .args(["sh", "-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if direct.is_ok() {
            return Ok(());
        }
        Command::new("sh").args(["-c", script]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err(io::Error::new(io::ErrorKind::Unsupported, "only on the tablet"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_shape() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert_ne!(u, uuid_v4());
    }

    #[test]
    fn writes_sidecars() {
        let dir = std::env::temp_dir().join(format!("rm2-xochitl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = add_pdf(&dir, "My \"sprite\"", b"%PDF-1.4\n").unwrap();
        let meta = std::fs::read_to_string(dir.join(format!("{id}.metadata"))).unwrap();
        assert!(meta.contains("\"visibleName\":\"My \\\"sprite\\\"\""));
        assert!(dir.join(format!("{id}.content")).is_file());
        assert!(dir.join(format!("{id}.pagedata")).is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
