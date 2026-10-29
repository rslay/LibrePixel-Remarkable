//! hello — minimal reMarkable 2 app on top of AppLoad's qtfb framebuffer, and the smoke test
//! for the build + deploy pipeline.
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
//!   --sim      the tablet simulated in a browser window on the PC (default elsewhere)
//! `HELLO_MODE=device|sim` does the same.

use std::sync::atomic::{AtomicBool, Ordering};

use librepixel::backend::Backend;
use librepixel::gfx::{Canvas, BLACK, WHITE};
use librepixel::qtfb::{self, Event, INPUT_PEN_RELEASE, INPUT_TOUCH_RELEASE, RM2_HEIGHT, RM2_WIDTH};
use librepixel::sim;

const GREY: u16 = 0x8410; // rgb565 mid grey

const SMILEY: [u16; 16] = [
    0x07E0, 0x1818, 0x2004, 0x4002, 0x4002, 0x8C31, 0x8C31, 0x8001,
    0x8001, 0x9009, 0x8812, 0x47E2, 0x4002, 0x2004, 0x1818, 0x07E0,
];

fn draw_hello(cv: &mut Canvas) {
    let (w, h) = (cv.w as i32, cv.h as i32);
    cv.fill_rect(0, 0, w, h, WHITE);
    cv.fill_rect(40, 40, w - 80, h - 80, BLACK); // border
    cv.fill_rect(48, 48, w - 96, h - 96, WHITE);
    cv.text_centered(200, 24, BLACK, "HELLO");
    cv.text_centered(320, 24, BLACK, "REMARKABLE 2");
    cv.sprite16((w - 16 * 24) / 2, 520, 24, &SMILEY, BLACK);
    cv.text_centered(1000, 12, GREY, "TAP ANYWHERE TO QUIT"); // scales are half pixels: 12 = 6x
    cv.text_centered(1100, 8, GREY, "QTFB CLIENT - WRITTEN IN RUST");
}

fn draw_bye(cv: &mut Canvas) {
    let (w, h) = (cv.w as i32, cv.h as i32);
    cv.fill_rect(48, 48, w - 96, h - 96, WHITE);
    cv.text_centered(800, 32, BLACK, "BYE!");
}

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

    draw_hello(&mut Canvas::new(b.fb(), w, h));
    if let Err(e) = b.update_all() {
        eprintln!("hello: update: {e}");
        return 2;
    }

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
            Ok(Some(Event::Eraser(e))) => eprintln!("hello: pen end: {}", if e { "eraser" } else { "tip" }),
            Ok(Some(Event::PenNear(n))) => eprintln!("hello: pen {}", if n { "near" } else { "away" }),
            Ok(Some(Event::TouchSize { slot, major })) => eprintln!("hello: touch slot {slot} size {major}"),
            Ok(Some(Event::Unknown(t))) => eprintln!("hello: message type {t}"),
        }
    }

    if rc == 0 {
        draw_bye(&mut Canvas::new(b.fb(), w, h));
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
