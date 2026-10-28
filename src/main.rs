//! hello — minimal reMarkable 2 app on top of AppLoad's qtfb framebuffer.
//!
//!   1. connect to the qtfb server inside xochitl, map the shared RGB565 buffer
//!   2. draw text and a scaled-up 16x16 sprite into it
//!   3. ask the server to push it to the e-ink panel
//!   4. wait for a finger or pen tap, draw "BYE", exit
//!
//! Exit codes: 0 = user tapped, 1 = could not connect, 2 = server went away.
//!
//! Where it draws is a `backend::Backend`:
//!   --device   the tablet, through AppLoad's qtfb socket (default on Linux)
//!   --sim      the tablet simulated in a browser window on the PC (default elsewhere), see sim.rs
//! `HELLO_MODE=device|sim` does the same.

mod backend;
mod font;
mod qtfb;
mod sim;

use std::sync::atomic::{AtomicBool, Ordering};

use backend::Backend;
use qtfb::{Event, INPUT_PEN_RELEASE, INPUT_TOUCH_RELEASE, RM2_HEIGHT, RM2_WIDTH};

const BLACK: u16 = 0x0000;
const WHITE: u16 = 0xFFFF;
const GREY: u16 = 0x8410; // rgb565 mid grey

// ---------- tiny raster helpers over an RGB565 slice ----------

struct Canvas<'a> {
    px: &'a mut [u16],
    w: usize,
    h: usize,
}

impl Canvas<'_> {
    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u16) {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w).max(0) as usize).min(self.w);
        let y1 = ((y + h).max(0) as usize).min(self.h);
        for row in y0..y1 {
            self.px[row * self.w + x0..row * self.w + x1].fill(color);
        }
    }

    fn text(&mut self, mut x: i32, y: i32, scale: i32, color: u16, s: &str) {
        for ch in s.chars() {
            let g = font::glyph(ch);
            for (r, bits) in g.iter().enumerate() {
                for col in 0..font::W {
                    if bits & (0x10 >> col) != 0 {
                        self.fill_rect(x + col as i32 * scale, y + r as i32 * scale, scale, scale, color);
                    }
                }
            }
            x += (font::W as i32 + 1) * scale;
        }
    }

    fn text_centered(&mut self, y: i32, scale: i32, color: u16, s: &str) {
        let width = s.chars().count() as i32 * (font::W as i32 + 1) * scale - scale;
        self.text((self.w as i32 - width) / 2, y, scale, color, s);
    }

    /// 16x16 1-bit sprite, `scale` screen pixels per sprite pixel.
    fn sprite16(&mut self, x: i32, y: i32, scale: i32, rows: &[u16; 16]) {
        for (r, bits) in rows.iter().enumerate() {
            for col in 0..16 {
                if bits & (0x8000 >> col) != 0 {
                    self.fill_rect(x + col * scale, y + r as i32 * scale, scale, scale, BLACK);
                }
            }
        }
    }
}

const SMILEY: [u16; 16] = [
    0x07E0, 0x1818, 0x2004, 0x4002, 0x4002, 0x8C31, 0x8C31, 0x8001,
    0x8001, 0x9009, 0x8812, 0x47E2, 0x4002, 0x2004, 0x1818, 0x07E0,
];

// ---------- the two frames this app shows ----------

fn draw_hello(cv: &mut Canvas) {
    let (w, h) = (cv.w as i32, cv.h as i32);
    cv.fill_rect(0, 0, w, h, WHITE);
    cv.fill_rect(40, 40, w - 80, h - 80, BLACK); // border
    cv.fill_rect(48, 48, w - 96, h - 96, WHITE);
    cv.text_centered(200, 12, BLACK, "HELLO");
    cv.text_centered(320, 12, BLACK, "REMARKABLE 2");
    cv.sprite16((w - 16 * 24) / 2, 520, 24, &SMILEY);
    cv.text_centered(1000, 6, GREY, "TAP ANYWHERE TO QUIT");
    cv.text_centered(1100, 4, GREY, "QTFB CLIENT - WRITTEN IN RUST");
}

fn draw_bye(cv: &mut Canvas) {
    let (w, h) = (cv.w as i32, cv.h as i32);
    cv.fill_rect(48, 48, w - 96, h - 96, WHITE);
    cv.text_centered(800, 16, BLACK, "BYE!");
}

// ---------- entry points ----------

const USAGE: &str = "usage: hello [--device | --sim]
  --device   draw on the tablet through AppLoad's qtfb socket (Linux only; the default there)
  --sim      simulate the tablet in a browser window (the default on Windows/macOS)
             HELLO_SIM_ADDR=host:port moves the server (default 127.0.0.1:1404)
             HELLO_SIM_OPEN=0 never opens a browser; open the address yourself
  HELLO_TIMEOUT_S=<n>  give up waiting for a tap after n seconds";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Device,
    Sim,
}

fn pick_mode() -> Result<Mode, String> {
    let mut mode = None;
    for a in std::env::args().skip(1) {
        mode = Some(match a.as_str() {
            "--device" => Mode::Device,
            "--sim" => Mode::Sim,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0)
            }
            other => return Err(format!("unknown argument '{other}'")),
        });
    }
    if mode.is_none() {
        mode = match std::env::var("HELLO_MODE").as_deref() {
            Ok("device") => Some(Mode::Device),
            Ok("sim") => Some(Mode::Sim),
            Ok(other) => return Err(format!("HELLO_MODE={other}: expected device or sim")),
            Err(_) => None,
        };
    }
    Ok(mode.unwrap_or(if cfg!(unix) { Mode::Device } else { Mode::Sim }))
}

/// Set by SIGINT/SIGTERM on the device, so the loop ends and the client still sends TERMINATE.
static QUIT: AtomicBool = AtomicBool::new(false);

fn main() {
    let mode = pick_mode().unwrap_or_else(|e| {
        eprintln!("hello: {e}\n{USAGE}");
        std::process::exit(64);
    });
    let rc = match mode {
        Mode::Sim => match sim::Sim::start(RM2_WIDTH, RM2_HEIGHT) {
            Ok(mut s) => run_app(&mut s),
            Err(e) => {
                eprintln!("hello: sim: {e}");
                1
            }
        },
        Mode::Device => run_device(),
    };
    std::process::exit(rc);
}

/// The app itself, independent of where it draws.
fn run_app(b: &mut impl Backend) -> i32 {
    let (w, h) = (b.width(), b.height());

    // --- frame 1 ---
    draw_hello(&mut Canvas { px: b.fb(), w, h });
    if let Err(e) = b.update_all() {
        eprintln!("hello: update: {e}");
        return 2;
    }

    // --- wait for a tap (finger or pen release) ---
    let timeout_ms: i32 = std::env::var("HELLO_TIMEOUT_S").ok()
        .and_then(|s| s.parse::<i32>().ok()).map(|s| s * 1000).unwrap_or(-1);

    let mut rc = 0;
    while !QUIT.load(Ordering::SeqCst) {
        match b.wait_event(timeout_ms) {
            Err(e) => { eprintln!("hello: {e}"); rc = 2; break; }
            Ok(None) => {
                if !QUIT.load(Ordering::SeqCst) { eprintln!("hello: timeout"); }
                break;
            }
            Ok(Some(Event::Input(i))) => {
                eprintln!("hello: {} dev={} x={} y={} d={}", qtfb::input_name(i.input_type), i.dev_id, i.x, i.y, i.d);
                if i.input_type == INPUT_TOUCH_RELEASE || i.input_type == INPUT_PEN_RELEASE {
                    QUIT.store(true, Ordering::SeqCst);
                }
            }
            Ok(Some(Event::DeviceState(s))) => eprintln!("hello: device state {s:?}"),
            Ok(Some(Event::Unknown(t))) => eprintln!("hello: message type {t}"),
        }
    }

    // --- frame 2: goodbye, give the panel a moment, leave ---
    if rc == 0 {
        draw_bye(&mut Canvas { px: b.fb(), w, h });
        let _ = b.update_all();
        std::thread::sleep(std::time::Duration::from_millis(600));
    }
    eprintln!("hello: exit {rc}");
    rc
}

#[cfg(not(unix))]
fn run_device() -> i32 {
    eprintln!("hello: --device needs Linux (the qtfb socket lives inside xochitl); use --sim here");
    1
}

#[cfg(unix)]
fn run_device() -> i32 {
    use qtfb::Client;

    extern "C" fn on_signal(_: libc::c_int) {
        QUIT.store(true, Ordering::SeqCst);
    }
    // SAFETY: installing a trivial async-signal-safe handler.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }

    let key = qtfb::key_from_env().unwrap_or_else(|| {
        eprintln!("hello: QTFB_KEY not set, using default framebuffer key");
        qtfb::DEFAULT_FRAMEBUFFER_KEY
    });

    let mut client = match Client::connect(key) {
        Ok(c) => c,
        Err(e) => { eprintln!("hello: {e}"); return 1; }
    };
    eprintln!("hello: connected, {}x{} rgb565", client.width, client.height);

    let rc = run_app(&mut client);
    drop(client); // sends TERMINATE, unmaps
    rc
}
