//! Native ink: the app's side of the `librepixel-ink` xovi extension (`extension/librepixel-ink.c`),
//! which draws the live pen stroke through xochitl's own pen path while this app arms it.
//!
//! The extension writes `STATUS_FILE` when it is running on a build it knows and refreshes it
//! about once a second; the app treats a fresh file as "native ink available". The app arms it
//! through `ARM_FILE`: `on x y w h size erase radius ox oy zoom brush grid` (screen pixels: the
//! rectangle the pen may ink in, the ink width, whether the tool is the eraser, the eraser
//! radius, where sprite pixel (0,0) sits on screen, the zoom, the brush in sprite pixels, and
//! whether grid lines are drawn) or `off`; a file older than three seconds counts as off, so
//! the app rewrites it every second while armed. With the last five fields the extension fills
//! the sprite's pixel blocks under the pen, the way the editor will; the seven-field form makes
//! it draw a round line instead.
//!
//! The preview itself is driven by the editor over `CMD_FIFO`, a named pipe the extension
//! creates: one `fill x y w h c` line per block the editor just decided to paint (screen
//! pixels; c = 1 for the eraser's white, 0 for black) and `end` when the stroke is over. So
//! the preview is exactly the editor's own pixels, pixel-perfect decisions included, instead of
//! the extension's reading of the pen.
//!
//! `RM2_INK_ARM`, `RM2_INK_STATUS` and `RM2_INK_CMD` override the paths; `RM2_INK_FAKE=1`
//! reports the extension as available without it (for the simulator and tests).

use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const ARM_FILE: &str = "/tmp/librepixel-ink";
pub const STATUS_FILE: &str = "/tmp/librepixel-ink.status";
pub const CMD_FIFO: &str = "/tmp/librepixel-ink.cmd";
/// How old the extension's status file may be and still count.
pub const STATUS_FRESH: Duration = Duration::from_secs(5);
/// How often the app must rewrite the arm file (the extension drops it after 3 s).
pub const HEARTBEAT: Duration = Duration::from_secs(1);

pub fn arm_path() -> PathBuf {
    std::env::var("RM2_INK_ARM").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(ARM_FILE))
}

pub fn status_path() -> PathBuf {
    std::env::var("RM2_INK_STATUS").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(STATUS_FILE))
}

/// The extension is running and knows this xochitl build: its status file is fresh and says
/// "ready" (it writes "off <reason>" when it has to stay out of the way).
pub fn available() -> bool {
    if std::env::var("RM2_INK_FAKE").map_or(false, |v| v != "0") {
        return true;
    }
    let Ok(meta) = std::fs::metadata(status_path()) else { return false };
    let fresh = meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age <= STATUS_FRESH);
    fresh && status_line().is_some_and(|l| l.starts_with("ready"))
}

/// The arm file line for `arm`'s arguments (round-line ink).
pub fn arm_line(x: i32, y: i32, w: i32, h: i32, size: f32, erase: bool, radius: f32) -> String {
    format!("on {x} {y} {w} {h} {size:.1} {} {radius:.1}\n", erase as i32)
}

/// The arm file line for pixel-block ink: `(ox, oy)` is where sprite pixel (0,0) sits on
/// screen, `zoom` the screen pixels per sprite pixel, `brush` the brush in sprite pixels.
#[allow(clippy::too_many_arguments)]
pub fn arm_line_blocks(
    x: i32, y: i32, w: i32, h: i32, size: f32, erase: bool, radius: f32, ox: i32, oy: i32, zoom: i32, brush: i32, grid: bool,
) -> String {
    format!("on {x} {y} {w} {h} {size:.1} {} {radius:.1} {ox} {oy} {zoom} {brush} {}\n", erase as i32, grid as i32)
}

pub fn write_arm(line: &str) -> io::Result<()> {
    std::fs::write(arm_path(), line)
}

/// What the extension last told us (first line of the status file), for the log.
pub fn status_line() -> Option<String> {
    std::fs::read_to_string(status_path()).ok().and_then(|t| t.lines().next().map(str::to_string))
}

/// The pen may ink inside `(x, y, w, h)` (screen pixels) with ink `size` wide; `erase` makes it
/// rub white with `radius`, and the pen's own eraser end always does.
pub fn arm(x: i32, y: i32, w: i32, h: i32, size: f32, erase: bool, radius: f32) -> io::Result<()> {
    write_arm(&arm_line(x, y, w, h, size, erase, radius))
}

pub fn disarm() -> io::Result<()> {
    write_arm("off\n")
}

pub fn cmd_path() -> PathBuf {
    std::env::var("RM2_INK_CMD").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(CMD_FIFO))
}

/// Writer for the extension's command pipe. Opens lazily and without blocking (a pipe with no
/// reader is simply "not there"), retries at most once a second, and drops the handle on any
/// write error so a restarted extension is picked up again.
pub struct Cmd {
    file: Option<File>,
    tried: Option<Instant>,
}

impl Default for Cmd {
    fn default() -> Self {
        Self::new()
    }
}

impl Cmd {
    pub fn new() -> Cmd {
        Cmd { file: None, tried: None }
    }

    fn open(&mut self) -> Option<&mut File> {
        if self.file.is_none() {
            if self.tried.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
                return None;
            }
            self.tried = Some(Instant::now());
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.custom_flags(libc::O_NONBLOCK);
            }
            self.file = opts.open(cmd_path()).ok();
        }
        self.file.as_mut()
    }

    fn send(&mut self, line: &str) {
        let ok = self.open().is_some_and(|f| f.write_all(line.as_bytes()).is_ok());
        if !ok {
            self.file = None;
        }
    }

    /// Show a block the editor just painted: screen rectangle, white for the eraser.
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, white: bool) {
        if w > 0 && h > 0 {
            self.send(&format!("fill {x} {y} {w} {h} {}\n", white as i32));
        }
    }

    /// The stroke is over: the extension may forget its pen-mask bookkeeping for it.
    pub fn end(&mut self) {
        self.send("end\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_file_format() {
        let dir = std::env::temp_dir().join(format!("rm2-ink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("arm");
        std::env::set_var("RM2_INK_ARM", &path);
        arm(265, 316, 1024, 1024, 8.0, false, 8.0).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "on 265 316 1024 1024 8.0 0 8.0\n");
        assert_eq!(
            arm_line_blocks(265, 316, 1024, 1024, 8.0, true, 8.0, 265, 316, 16, 2, true),
            "on 265 316 1024 1024 8.0 1 8.0 265 316 16 2 1\n"
        );
        disarm().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "off\n");
        std::env::remove_var("RM2_INK_ARM");
        // The command "pipe" can be a plain file for tests; lines are appended.
        let cmd = dir.join("cmd");
        std::fs::write(&cmd, "").unwrap();
        std::env::set_var("RM2_INK_CMD", &cmd);
        let mut c = Cmd::new();
        c.fill(10, 20, 16, 16, false);
        c.fill(0, 0, 0, 5, true); // empty: not sent
        c.end();
        assert_eq!(std::fs::read_to_string(&cmd).unwrap(), "fill 10 20 16 16 0\nend\n");
        std::env::remove_var("RM2_INK_CMD");
        let mut missing = Cmd::new();
        std::env::set_var("RM2_INK_CMD", dir.join("nope").join("cmd"));
        missing.fill(1, 1, 1, 1, false); // no pipe: silently dropped
        std::env::remove_var("RM2_INK_CMD");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
