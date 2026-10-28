//! fake_qtfb_server — PC stand-in for AppLoad's qtfb server.
//!
//!   - accepts one client on a unix SOCK_SEQPACKET socket
//!   - creates the shared-memory framebuffer and answers INITIALIZE
//!   - on every UPDATE, dumps the framebuffer to frame-N.ppm in the output dir
//!   - after the first UPDATE, sends a synthetic finger tap
//!   - exits when the client sends TERMINATE or hangs up
//!
//! usage: fake_qtfb_server <socket path> <output dir>

#![cfg_attr(not(unix), allow(unused))]

#[cfg(unix)]
#[path = "../qtfb.rs"]
#[allow(dead_code)]
mod qtfb;

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::{BufWriter, Write};
#[cfg(unix)]
use std::mem::{size_of, zeroed};

#[cfg(unix)]
use qtfb::*;

#[cfg(not(unix))]
fn main() {
    eprintln!("fake_qtfb_server needs unix sockets + POSIX shm; on Windows use `cargo run -- --headless` instead.");
    std::process::exit(1);
}

#[cfg(unix)]
fn send(fd: i32, m: &ServerMessage) {
    // SAFETY: m is a valid repr(C) struct.
    unsafe { libc::send(fd, m as *const _ as *const libc::c_void, size_of::<ServerMessage>(), libc::MSG_NOSIGNAL); }
}

#[cfg(unix)]
fn dump_ppm(fb: &[u16], w: usize, h: usize, path: &str) {
    let mut f = BufWriter::new(File::create(path).expect("create ppm"));
    write!(f, "P6\n{w} {h}\n255\n").unwrap();
    for &p in fb {
        let r = (((p >> 11) & 0x1F) as u32 * 255 / 31) as u8;
        let g = (((p >> 5) & 0x3F) as u32 * 255 / 63) as u8;
        let b = ((p & 0x1F) as u32 * 255 / 31) as u8;
        f.write_all(&[r, g, b]).unwrap();
    }
    eprintln!("server: wrote {path}");
}

#[cfg(unix)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sock_path = args.get(1).cloned().unwrap_or_else(|| SOCKET_PATH.to_string());
    let out_dir = args.get(2).cloned().unwrap_or_else(|| ".".to_string());

    // SAFETY: straightforward libc socket/shm setup with valid arguments.
    unsafe {
        let ls = libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0);
        let mut addr: libc::sockaddr_un = zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (d, s) in addr.sun_path.iter_mut().zip(sock_path.as_bytes()) { *d = *s as libc::c_char; }
        let _ = std::fs::remove_file(&sock_path);
        assert_eq!(libc::bind(ls, &addr as *const _ as *const libc::sockaddr, size_of::<libc::sockaddr_un>() as u32), 0, "bind");
        assert_eq!(libc::listen(ls, 1), 0, "listen");
        eprintln!("server: listening on {sock_path}");

        let cs = libc::accept(ls, std::ptr::null_mut(), std::ptr::null_mut());
        assert!(cs >= 0, "accept");

        let mut cm: ClientMessage = zeroed();
        let n = libc::recv(cs, &mut cm as *mut _ as *mut libc::c_void, size_of::<ClientMessage>(), 0);
        assert!(n >= 1 && cm.kind == MESSAGE_INITIALIZE, "expected INITIALIZE");
        let init = cm.payload.init;
        eprintln!("server: client key={} fmt={}", init.framebuffer_key, init.framebuffer_type);
        assert_eq!(init.framebuffer_type, FBFMT_RM2FB, "only RM2FB supported");

        let (w, h) = (RM2_WIDTH, RM2_HEIGHT);
        let size = w * h * 2;
        let shm_key = 4242;
        let name = CString::new(format!("/qtfb_{shm_key}")).unwrap();
        libc::shm_unlink(name.as_ptr());
        let fd = libc::shm_open(name.as_ptr(), libc::O_RDWR | libc::O_CREAT, 0o666);
        assert!(fd >= 0 && libc::ftruncate(fd, size as libc::off_t) == 0, "shm");
        let p = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
        assert!(p != libc::MAP_FAILED, "mmap");
        let fb = std::slice::from_raw_parts_mut(p as *mut u16, w * h);
        fb.fill(0xFFFF);

        let mut sm: ServerMessage = zeroed();
        sm.kind = MESSAGE_INITIALIZE;
        sm.payload.init = InitResponse { shm_key, shm_size: size };
        send(cs, &sm);

        // AppLoad sends the rotation state right after init
        let mut sm: ServerMessage = zeroed();
        sm.kind = MESSAGE_DEVICE_STATE_INIT;
        sm.payload.device_state = DeviceState { reason: STATE_CHANGED_REASON_ROTATION, rotation: 0 };
        send(cs, &sm);

        let (mut frame, mut tapped) = (0, false);
        loop {
            let mut cm: ClientMessage = zeroed();
            let n = libc::recv(cs, &mut cm as *mut _ as *mut libc::c_void, size_of::<ClientMessage>(), 0);
            if n < 1 { eprintln!("server: client hung up"); break; }
            match cm.kind {
                MESSAGE_UPDATE => {
                    let u = cm.payload.update;
                    eprintln!("server: UPDATE type={} rect={},{} {}x{}", u.kind, u.x, u.y, u.w, u.h);
                    dump_ppm(fb, w, h, &format!("{out_dir}/frame-{frame}.ppm"));
                    frame += 1;
                    if !tapped {
                        tapped = true;
                        for t in [INPUT_TOUCH_PRESS, INPUT_TOUCH_RELEASE] {
                            let mut sm: ServerMessage = zeroed();
                            sm.kind = MESSAGE_USERINPUT;
                            sm.payload.user_input = UserInput { input_type: t, dev_id: 1, x: 700, y: 900, d: 0 };
                            send(cs, &sm);
                        }
                        eprintln!("server: sent synthetic tap");
                    }
                }
                MESSAGE_TERMINATE => { eprintln!("server: TERMINATE"); break; }
                MESSAGE_SET_REFRESH_MODE => eprintln!("server: refresh mode {}", cm.payload.refresh_mode),
                MESSAGE_REQUEST_FULL_REFRESH => eprintln!("server: full refresh requested"),
                other => eprintln!("server: message {other}"),
            }
        }
        libc::shm_unlink(name.as_ptr());
        let _ = std::fs::remove_file(&sock_path);
    }
}
