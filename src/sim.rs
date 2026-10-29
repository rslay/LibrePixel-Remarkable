//! Simulator: the tablet in a browser window, so the app can be seen and poked
//! on the PC without pushing it to the device.
//!
//! The app draws into an RGB565 buffer exactly as on the device. Every
//! `update_*` publishes that buffer to a tiny HTTP server (std only, no
//! crates). The page in `sim.html` shows it on a canvas of the panel's size
//! (1404x1872, scaled to fit the window) and turns mouse / touch / stylus
//! input into the same qtfb events AppLoad would send. Rotation, refresh modes
//! and partial updates are accepted and logged, nothing more.
//!
//! Endpoints (loopback only):
//!   GET  /               the page
//!   GET  /frame?after=N  long-poll: the framebuffer once its sequence number
//!                        is > N, as raw little-endian RGB565 with headers
//!                        X-Frame-Seq and X-Frame-Size; 204 after ~20 s
//!   POST /input          events separated by newlines or ';', each
//!                        "type x y d [dev]" with an INPUT_* value from qtfb.rs
//!
//! Env: HELLO_SIM_ADDR=host:port (default 127.0.0.1:1404), HELLO_SIM_OPEN=0 to
//! never launch a browser.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::backend::Backend;
use crate::qtfb::{DeviceState, Event, RefreshMode, UserInput, STATE_CHANGED_REASON_ROTATION};

/// Simulator-only input type for `/input` lines: `256 0 0 1` means the eraser end of the pen
/// is near the screen, `256 0 0 0` the tip. Becomes `Event::Eraser`, like the digitizer's
/// tool report does on the device.
pub const SIM_INPUT_ERASER: i32 = 0x100;
/// `257 0 0 1` / `257 0 0 0`: the pen came within hover range of the screen / left it
/// (`Event::PenNear`, the digitizer's BTN_TOOL_PEN on the device).
pub const SIM_INPUT_PEN_NEAR: i32 = 0x101;

const PAGE: &str = include_str!("sim.html");
const DEFAULT_ADDR: &str = "127.0.0.1:1404";
const POLL_TIMEOUT: Duration = Duration::from_secs(20);

struct Frame {
    seq: u64,
    px: Vec<u16>,
}

/// Shared between the app thread and the HTTP handler threads.
struct Shared {
    w: usize,
    h: usize,
    frame: Mutex<Frame>,
    frame_cv: Condvar,
    events: Mutex<VecDeque<Event>>,
    events_cv: Condvar,
    /// Number of /frame requests seen. A window left open from the previous run
    /// reconnects within a second, in which case no new one is opened.
    pages: AtomicUsize,
}

/// Lock that shrugs off poisoning: a panicking handler thread must not take the app down.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Sim {
    buf: Vec<u16>,
    shared: Arc<Shared>,
}

impl Sim {
    pub fn start(w: usize, h: usize) -> io::Result<Sim> {
        let addr = std::env::var("HELLO_SIM_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
        let listener = TcpListener::bind(&addr).or_else(|e| {
            eprintln!("hello: sim: cannot listen on {addr} ({e}); using a free port instead");
            TcpListener::bind("127.0.0.1:0")
        })?;
        let url = format!("http://{}/", listener.local_addr()?);

        let shared = Arc::new(Shared {
            w,
            h,
            frame: Mutex::new(Frame { seq: 0, px: vec![0xFFFF; w * h] }),
            frame_cv: Condvar::new(),
            events: Mutex::new(VecDeque::new()),
            events_cv: Condvar::new(),
            pages: AtomicUsize::new(0),
        });
        // AppLoad tells a freshly connected client its rotation; do the same.
        lock(&shared.events).push_back(Event::DeviceState(DeviceState {
            reason: STATE_CHANGED_REASON_ROTATION,
            rotation: 0,
        }));

        let sh = shared.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        let sh = sh.clone();
                        std::thread::spawn(move || {
                            let _ = handle(stream, &sh); // a dropped connection is the browser's business
                        });
                    }
                    Err(e) => eprintln!("hello: sim: accept: {e}"),
                }
            }
        });

        eprintln!("hello: simulator at {url}");
        if std::env::var("HELLO_SIM_OPEN").map_or(true, |v| v != "0") {
            let sh = shared.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1500));
                if sh.pages.load(Ordering::Relaxed) == 0 {
                    open_browser(&url);
                }
            });
        }
        Ok(Sim { buf: vec![0xFFFF; w * h], shared })
    }

    fn publish(&mut self) {
        let mut f = lock(&self.shared.frame);
        f.px.copy_from_slice(&self.buf);
        f.seq += 1;
        drop(f);
        self.shared.frame_cv.notify_all();
    }
}

impl Backend for Sim {
    fn width(&self) -> usize { self.shared.w }
    fn height(&self) -> usize { self.shared.h }
    fn fb(&mut self) -> &mut [u16] { &mut self.buf }

    fn update_all(&mut self) -> io::Result<()> {
        self.publish();
        Ok(())
    }

    fn update_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) -> io::Result<()> {
        self.publish(); // the page always takes the whole frame; cheap enough on loopback
        Ok(())
    }

    fn set_refresh_mode(&mut self, mode: RefreshMode) -> io::Result<()> {
        eprintln!("hello: sim: refresh mode {mode:?} (on the tablet this stalls AppLoad's updates for 1 s)");
        Ok(())
    }

    fn request_full_refresh(&mut self) -> io::Result<()> {
        eprintln!("hello: sim: full refresh requested (on the tablet this stalls AppLoad's updates for 1 s)");
        Ok(())
    }

    fn wait_event(&mut self, timeout_ms: i32) -> io::Result<Option<Event>> {
        let deadline = (timeout_ms >= 0).then(|| Instant::now() + Duration::from_millis(timeout_ms as u64));
        let mut q = lock(&self.shared.events);
        loop {
            if let Some(ev) = q.pop_front() {
                return Ok(Some(ev));
            }
            q = match deadline {
                None => self.shared.events_cv.wait(q).unwrap_or_else(|e| e.into_inner()),
                Some(d) => {
                    let left = d.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Ok(None);
                    }
                    self.shared.events_cv.wait_timeout(q, left).unwrap_or_else(|e| e.into_inner()).0
                }
            };
        }
    }
}

/// Open the page: as its own plain window when Chrome or Edge is around ("app mode"),
/// otherwise as a tab in whatever the default browser is.
fn open_browser(url: &str) {
    use std::process::{Command, Stdio};
    let app = format!("--app={url}");
    let mut candidates: Vec<Command> = Vec::new();
    if cfg!(windows) {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        for exe in [
            format!(r"{pf86}\Microsoft\Edge\Application\msedge.exe"),
            format!(r"{pf}\Google\Chrome\Application\chrome.exe"),
            format!(r"{pf86}\Google\Chrome\Application\chrome.exe"),
            format!(r"{local}\Google\Chrome\Application\chrome.exe"),
        ] {
            if std::path::Path::new(&exe).is_file() {
                let mut c = Command::new(exe);
                c.arg(&app).arg("--window-size=800,1000");
                candidates.push(c);
            }
        }
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        candidates.push(c);
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        candidates.push(c);
    } else {
        for exe in ["google-chrome", "chromium", "chromium-browser", "microsoft-edge"] {
            let mut c = Command::new(exe);
            c.arg(&app).arg("--window-size=800,1000");
            candidates.push(c);
        }
        let mut c = Command::new("xdg-open");
        c.arg(url);
        candidates.push(c);
    }
    for mut c in candidates {
        if c.stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
            return;
        }
    }
    eprintln!("hello: sim: could not launch a browser; open {url} yourself");
}

// ---- a very small HTTP/1.1 server ------------------------------------------

struct Request {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn read_request(s: &mut TcpStream) -> io::Result<Request> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 64 * 1024 {
            return Err(bad("request header too large"));
        }
        let n = s.read(&mut tmp)?;
        if n == 0 {
            return Err(bad("connection closed before the request was complete"));
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or("").split_whitespace();
    let method = request_line.next().unwrap_or("").to_string();
    let target = request_line.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let content_length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > 1024 * 1024 {
        return Err(bad("request body too large"));
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < content_length {
        let n = s.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);
    Ok(Request { method, path: path.to_string(), query: query.to_string(), body })
}

fn respond(s: &mut TcpStream, status: &str, ctype: &str, extra: &[(&str, String)], body: &[u8]) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nCache-Control: no-store\r\nConnection: close\r\n");
    if !status.starts_with("204") {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    s.flush()
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

/// Block until the frame's sequence number exceeds `after`, or give up after `POLL_TIMEOUT`.
fn wait_frame(sh: &Shared, after: u64) -> Option<(u64, Vec<u8>)> {
    let deadline = Instant::now() + POLL_TIMEOUT;
    let mut f = lock(&sh.frame);
    while f.seq <= after {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        f = sh.frame_cv.wait_timeout(f, left).unwrap_or_else(|e| e.into_inner()).0;
    }
    let mut bytes = vec![0u8; f.px.len() * 2];
    for (dst, p) in bytes.chunks_exact_mut(2).zip(&f.px) {
        dst.copy_from_slice(&p.to_le_bytes());
    }
    Some((f.seq, bytes))
}

fn handle(mut s: TcpStream, sh: &Shared) -> io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    let req = read_request(&mut s)?;
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            respond(&mut s, "200 OK", "text/html; charset=utf-8", &[], PAGE.as_bytes())
        }
        ("GET", "/frame") => {
            sh.pages.fetch_add(1, Ordering::Relaxed);
            let after = query_param(&req.query, "after").and_then(|v| v.parse().ok()).unwrap_or(0);
            match wait_frame(sh, after) {
                Some((seq, bytes)) => respond(&mut s, "200 OK", "application/octet-stream",
                    &[("X-Frame-Seq", seq.to_string()), ("X-Frame-Size", format!("{}x{}", sh.w, sh.h))], &bytes),
                None => respond(&mut s, "204 No Content", "text/plain", &[], b""),
            }
        }
        ("POST", "/input") => {
            let text = String::from_utf8_lossy(&req.body);
            let mut n = 0;
            let mut q = lock(&sh.events);
            for line in text.split(|c| c == '\n' || c == ';') {
                let v: Vec<i32> = line.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                if (v.len() == 4 || v.len() == 5) && v[0] == SIM_INPUT_ERASER {
                    q.push_back(Event::Eraser(v[3] != 0));
                    n += 1;
                } else if (v.len() == 4 || v.len() == 5) && v[0] == SIM_INPUT_PEN_NEAR {
                    q.push_back(Event::PenNear(v[3] != 0));
                    n += 1;
                } else if v.len() == 4 || v.len() == 5 {
                    q.push_back(Event::Input(UserInput {
                        input_type: v[0],
                        dev_id: v.get(4).copied().unwrap_or(1),
                        x: v[1],
                        y: v[2],
                        d: v[3],
                    }));
                    n += 1;
                }
            }
            drop(q);
            sh.events_cv.notify_all();
            respond(&mut s, "200 OK", "text/plain", &[], format!("{n} events\n").as_bytes())
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", &[], b"not found\n"),
    }
}
