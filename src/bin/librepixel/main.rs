//! sprite — a pixel-art editor for the reMarkable 2, on AppLoad's qtfb framebuffer.
//!
//! Draws with the pen on a 16-color, layered sprite; saves `.ase` files that Aseprite and
//! LibreSprite open; exports PNG, JPG, and a PDF straight into the tablet's library.
//!
//!   librepixel [--device | --sim] [file.ase | file.png]
//!
//! Files live in `$RM2_SPRITE_DIR`, else `~/sprites` (`/home/root/sprites` on the tablet).

mod app;
mod dialogs;
mod ui;
mod view;

use std::path::PathBuf;

use librepixel::{sim, RM2_HEIGHT, RM2_WIDTH};

const USAGE: &str = "usage: librepixel [--device | --sim] [file.ase | file.png]
  --device   draw on the tablet through AppLoad's qtfb socket (Linux only; the default there)
  --sim      simulate the tablet in a browser window (the default on Windows/macOS)
             HELLO_SIM_ADDR=host:port moves the server, HELLO_SIM_OPEN=0 never opens a browser
  RM2_SPRITE_DIR=<dir>   where sprites are kept (default ~/sprites)";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Device,
    Sim,
}

fn sprites_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("RM2_SPRITE_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    match home {
        Some(h) => h.join("sprites"),
        None if cfg!(unix) => PathBuf::from("/home/root/sprites"),
        None => PathBuf::from("sprites"),
    }
}

fn main() {
    let mut mode = None;
    let mut start = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--device" => mode = Some(Mode::Device),
            "--sim" => mode = Some(Mode::Sim),
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("librepixel: unknown argument '{other}'\n{USAGE}");
                std::process::exit(64);
            }
            file => start = Some(PathBuf::from(file)),
        }
    }
    let mode = mode
        .or_else(|| match std::env::var("HELLO_MODE").as_deref() {
            Ok("device") => Some(Mode::Device),
            Ok("sim") => Some(Mode::Sim),
            _ => None,
        })
        .unwrap_or(if cfg!(unix) { Mode::Device } else { Mode::Sim });

    let dir = sprites_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("librepixel: cannot create {}: {e}", dir.display());
    }
    eprintln!("librepixel: files in {}", dir.display());

    let rc = match mode {
        Mode::Sim => match sim::Sim::start(RM2_WIDTH, RM2_HEIGHT) {
            Ok(s) => app::App::new(s, dir, start).run(),
            Err(e) => {
                eprintln!("librepixel: sim: {e}");
                1
            }
        },
        Mode::Device => run_device(dir, start),
    };
    std::process::exit(rc);
}

#[cfg(not(unix))]
fn run_device(_dir: PathBuf, _start: Option<PathBuf>) -> i32 {
    eprintln!("librepixel: --device needs Linux (the qtfb socket lives inside xochitl); use --sim here");
    1
}

#[cfg(unix)]
fn run_device(dir: PathBuf, start: Option<PathBuf>) -> i32 {
    use librepixel::backend::Backend;
    use librepixel::qtfb;
    use std::sync::atomic::Ordering;

    extern "C" fn on_signal(_: libc::c_int) {
        app::QUIT.store(true, Ordering::SeqCst);
    }
    // SAFETY: installing a trivial async-signal-safe handler.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }

    let key = qtfb::key_from_env().unwrap_or_else(|| {
        eprintln!("librepixel: QTFB_KEY not set, using default framebuffer key");
        qtfb::DEFAULT_FRAMEBUFFER_KEY
    });
    let client = match qtfb::Client::connect(key) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("librepixel: {e}");
            return 1;
        }
    };
    eprintln!("librepixel: connected, {}x{} rgb565", client.width(), client.height());
    app::App::new(client, dir, start).run()
}
