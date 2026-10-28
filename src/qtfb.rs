#![allow(dead_code)]
//! Minimal client for AppLoad's qtfb framebuffer server.
//!
//! Wire protocol ported from rm-appload/backends/qtfb-clients/cpp/common.h
//! (asivery/rm-appload, GPL-3.0). The structs are `#[repr(C)]` and must match
//! the server compiled into appload.so, which runs inside xochitl on the same
//! device — so the same architecture and ABI.
//!
//! Flow: connect to a SOCK_SEQPACKET unix socket, send INITIALIZE, get back a
//! shm key + size, mmap `/qtfb_<key>` (RGB565, 1404x1872), draw into it, send
//! UPDATE. Input events (touch / pen / buttons) arrive on the same socket.
//!
//! The protocol half of this file (constants, structs, `Event`) builds on every
//! platform so the simulator and the tests can share it. The socket/shm
//! `Client` at the bottom is unix-only.

use std::mem::zeroed;

pub const SOCKET_PATH: &str = "/tmp/qtfb.sock";
pub const DEFAULT_FRAMEBUFFER_KEY: i32 = 245_209_899;

pub const RM2_WIDTH: usize = 1404;
pub const RM2_HEIGHT: usize = 1872;

// client -> server
pub const MESSAGE_INITIALIZE: u8 = 0;
pub const MESSAGE_UPDATE: u8 = 1;
pub const MESSAGE_CUSTOM_INITIALIZE: u8 = 2;
pub const MESSAGE_TERMINATE: u8 = 3;
pub const MESSAGE_SET_REFRESH_MODE: u8 = 5;
pub const MESSAGE_REQUEST_FULL_REFRESH: u8 = 6;
// server -> client
pub const MESSAGE_USERINPUT: u8 = 4;
pub const MESSAGE_DEVICE_STATE_CHANGED: u8 = 7;
pub const MESSAGE_DEVICE_STATE_INIT: u8 = 8;

pub const FBFMT_RM2FB: u8 = 0; // rgb565, 1404x1872

pub const UPDATE_ALL: i32 = 0;
pub const UPDATE_PARTIAL: i32 = 1;

#[allow(dead_code)]
#[repr(i32)]
#[derive(Clone, Copy, Debug)]
pub enum RefreshMode {
    UltraFast = 0,
    Fast = 1,
    Animate = 2,
    Content = 3,
    Ui = 4,
    Sleep = 5,
}

pub const INPUT_TOUCH_PRESS: i32 = 0x10;
pub const INPUT_TOUCH_RELEASE: i32 = 0x11;
pub const INPUT_TOUCH_UPDATE: i32 = 0x12;
pub const INPUT_PEN_PRESS: i32 = 0x20;
pub const INPUT_PEN_RELEASE: i32 = 0x21;
pub const INPUT_PEN_UPDATE: i32 = 0x22;
pub const INPUT_BTN_PRESS: i32 = 0x30;
pub const INPUT_BTN_RELEASE: i32 = 0x31;
pub const INPUT_VKB_PRESS: i32 = 0x40;
pub const INPUT_VKB_RELEASE: i32 = 0x41;

pub const STATE_CHANGED_REASON_ROTATION: i32 = 0;

/// Name of an `INPUT_*` value, for logs.
pub fn input_name(input_type: i32) -> &'static str {
    match input_type {
        INPUT_TOUCH_PRESS => "touch press",
        INPUT_TOUCH_RELEASE => "touch release",
        INPUT_TOUCH_UPDATE => "touch move",
        INPUT_PEN_PRESS => "pen press",
        INPUT_PEN_RELEASE => "pen release",
        INPUT_PEN_UPDATE => "pen move",
        INPUT_BTN_PRESS => "button press",
        INPUT_BTN_RELEASE => "button release",
        INPUT_VKB_PRESS => "keyboard press",
        INPUT_VKB_RELEASE => "keyboard release",
        _ => "unknown input",
    }
}

// ---- wire structs ---------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InitMessage {
    pub framebuffer_key: i32,
    pub framebuffer_type: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CustomInitMessage {
    pub framebuffer_key: i32,
    pub framebuffer_type: u8,
    pub width: u16,
    pub height: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InitResponse {
    pub shm_key: i32,
    pub shm_size: usize, // size_t: 4 bytes on arm32, 8 on x86-64
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UpdateRegion {
    pub kind: i32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UserInput {
    pub input_type: i32,
    pub dev_id: i32,
    pub x: i32,
    pub y: i32,
    pub d: i32, // pen pressure 0..100
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DeviceState {
    pub reason: i32,
    pub rotation: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ClientPayload {
    pub init: InitMessage,
    pub update: UpdateRegion,
    pub custom_init: CustomInitMessage,
    pub refresh_mode: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClientMessage {
    pub kind: u8,
    pub payload: ClientPayload,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ServerPayload {
    pub init: InitResponse,
    pub user_input: UserInput,
    pub device_state: DeviceState,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ServerMessage {
    pub kind: u8,
    pub payload: ServerPayload,
}

impl ClientMessage {
    fn new(kind: u8) -> Self {
        // SAFETY: all-zero is a valid bit pattern for every union member.
        let mut m: ClientMessage = unsafe { zeroed() };
        m.kind = kind;
        m
    }
}

/// Decoded server event, so callers never touch the union.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    Input(UserInput),
    DeviceState(DeviceState),
    Unknown(u8),
}

/// Reads `QTFB_KEY`, which AppLoad sets when it launches an external app.
pub fn key_from_env() -> Option<i32> {
    std::env::var("QTFB_KEY").ok()?.trim().parse().ok()
}

// ---- client (unix only: unix sockets + POSIX shm) ---------------------------

#[cfg(unix)]
mod client {
    use super::*;
    use std::ffi::CString;
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::unix::io::RawFd;

    pub struct Client {
        sock: RawFd,
        shm_fd: RawFd,
        fb_ptr: *mut u16,
        fb_len: usize, // bytes
        pub width: usize,
        pub height: usize,
    }

    fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
    }

    impl Client {
        pub fn connect(key: i32) -> io::Result<Self> {
            // SAFETY: plain libc calls with valid, fully-initialised arguments.
            unsafe {
                let sock = cvt(libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0))?;
                let path = std::env::var("QTFB_SOCKET").unwrap_or_else(|_| SOCKET_PATH.to_string());
                let mut addr: libc::sockaddr_un = zeroed();
                addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
                for (dst, src) in addr.sun_path.iter_mut().zip(path.as_bytes()) {
                    *dst = *src as libc::c_char;
                }
                if libc::connect(sock, &addr as *const _ as *const libc::sockaddr,
                                 size_of::<libc::sockaddr_un>() as libc::socklen_t) != 0 {
                    let e = io::Error::last_os_error();
                    libc::close(sock);
                    return Err(io::Error::new(e.kind(),
                        format!("connect {path}: {e} (is AppLoad running, and was this launched from it?)")));
                }

                let mut c = Client { sock, shm_fd: -1, fb_ptr: std::ptr::null_mut(), fb_len: 0,
                                     width: RM2_WIDTH, height: RM2_HEIGHT };

                let mut init = ClientMessage::new(MESSAGE_INITIALIZE);
                init.payload.init = InitMessage { framebuffer_key: key, framebuffer_type: FBFMT_RM2FB };
                c.send(&init)?;

                let mut resp: ServerMessage = zeroed();
                let n = libc::recv(sock, &mut resp as *mut _ as *mut libc::c_void, size_of::<ServerMessage>(), 0);
                if n < 1 { return Err(io::Error::new(io::ErrorKind::Other, "no init response from server")); }
                if resp.kind != MESSAGE_INITIALIZE {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("unexpected init response type {}", resp.kind)));
                }
                let shm_key = resp.payload.init.shm_key;
                let shm_size = resp.payload.init.shm_size;

                let name = CString::new(format!("/qtfb_{shm_key}")).unwrap();
                c.shm_fd = cvt(libc::shm_open(name.as_ptr(), libc::O_RDWR, 0))?;
                let p = libc::mmap(std::ptr::null_mut(), shm_size, libc::PROT_READ | libc::PROT_WRITE,
                                   libc::MAP_SHARED, c.shm_fd, 0);
                if p == libc::MAP_FAILED { return Err(io::Error::last_os_error()); }
                if shm_size < RM2_WIDTH * RM2_HEIGHT * 2 {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("shm too small: {shm_size}")));
                }
                c.fb_ptr = p as *mut u16;
                c.fb_len = shm_size;
                Ok(c)
            }
        }

        /// The framebuffer as a mutable slice of RGB565 pixels, row-major, `width` per row.
        pub fn fb(&mut self) -> &mut [u16] {
            // SAFETY: mapped MAP_SHARED, lives until Drop; u16 has no invalid bit patterns.
            unsafe { std::slice::from_raw_parts_mut(self.fb_ptr, self.width * self.height) }
        }

        fn send(&self, m: &ClientMessage) -> io::Result<()> {
            // SAFETY: m is a valid repr(C) struct of the stated size.
            let n = unsafe {
                libc::send(self.sock, m as *const _ as *const libc::c_void, size_of::<ClientMessage>(), libc::MSG_NOSIGNAL)
            };
            if n != size_of::<ClientMessage>() as isize { Err(io::Error::last_os_error()) } else { Ok(()) }
        }

        pub fn update_all(&self) -> io::Result<()> {
            let mut m = ClientMessage::new(MESSAGE_UPDATE);
            m.payload.update = UpdateRegion { kind: UPDATE_ALL, x: 0, y: 0, w: 0, h: 0 };
            self.send(&m)
        }

        pub fn update_rect(&self, x: i32, y: i32, w: i32, h: i32) -> io::Result<()> {
            let mut m = ClientMessage::new(MESSAGE_UPDATE);
            m.payload.update = UpdateRegion { kind: UPDATE_PARTIAL, x, y, w, h };
            self.send(&m)
        }

        pub fn set_refresh_mode(&self, mode: RefreshMode) -> io::Result<()> {
            let mut m = ClientMessage::new(MESSAGE_SET_REFRESH_MODE);
            m.payload.refresh_mode = mode as i32;
            self.send(&m)
        }

        pub fn request_full_refresh(&self) -> io::Result<()> {
            self.send(&ClientMessage::new(MESSAGE_REQUEST_FULL_REFRESH))
        }

        /// Wait up to `timeout_ms` (-1 = forever) for the next server event.
        /// Ok(None) on timeout; Err on disconnect.
        pub fn wait_event(&self, timeout_ms: i32) -> io::Result<Option<Event>> {
            let mut p = libc::pollfd { fd: self.sock, events: libc::POLLIN, revents: 0 };
            // SAFETY: valid pollfd array of length 1.
            let r = unsafe { libc::poll(&mut p, 1, timeout_ms) };
            if r == 0 { return Ok(None); }
            if r < 0 {
                let e = io::Error::last_os_error();
                return if e.kind() == io::ErrorKind::Interrupted { Ok(None) } else { Err(e) };
            }
            if p.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "server hung up"));
            }
            // SAFETY: zeroed is valid for the union; recv fills at most size_of bytes.
            let msg: ServerMessage = unsafe {
                let mut m: ServerMessage = zeroed();
                let n = libc::recv(self.sock, &mut m as *mut _ as *mut libc::c_void, size_of::<ServerMessage>(), 0);
                if n < 1 { return Err(io::Error::new(io::ErrorKind::BrokenPipe, "server closed")); }
                m
            };
            // SAFETY: the tag tells us which union member the server wrote.
            Ok(Some(unsafe {
                match msg.kind {
                    MESSAGE_USERINPUT => Event::Input(msg.payload.user_input),
                    MESSAGE_DEVICE_STATE_INIT | MESSAGE_DEVICE_STATE_CHANGED => Event::DeviceState(msg.payload.device_state),
                    other => Event::Unknown(other),
                }
            }))
        }
    }

    impl Drop for Client {
        fn drop(&mut self) {
            // SAFETY: undoing exactly what connect() set up.
            unsafe {
                if !self.fb_ptr.is_null() { libc::munmap(self.fb_ptr as *mut libc::c_void, self.fb_len); }
                if self.shm_fd >= 0 { libc::close(self.shm_fd); }
                let _ = self.send(&ClientMessage::new(MESSAGE_TERMINATE));
                libc::close(self.sock);
            }
        }
    }
}

#[cfg(unix)]
#[allow(unused_imports)]
pub use client::Client;

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn wire_sizes_match_c() {
        // Verified against the C header with gcc: 24 bytes on both arm32 and x86-64.
        assert_eq!(size_of::<ClientMessage>(), 24);
        // Contains size_t, so 24 on 32-bit, 32 on 64-bit.
        assert_eq!(size_of::<ServerMessage>(), if size_of::<usize>() == 4 { 24 } else { 32 });
    }
}
