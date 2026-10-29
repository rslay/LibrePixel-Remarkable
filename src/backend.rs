//! What the app draws on. One trait, two implementations:
//!
//!   `qtfb::Client`  the real thing: AppLoad's shared-memory framebuffer (unix only)
//!   `sim::Sim`      the tablet simulated in a browser window on the PC
//!
//! `run_app` in main.rs only talks to this trait, so whatever works in the
//! simulator works on the device and the other way round.

use std::io;

use crate::qtfb::{Event, RefreshMode};

#[allow(dead_code)] // the whole qtfb surface is offered even if this app uses only part of it
pub trait Backend {
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    /// The framebuffer: RGB565, row-major, `width()` pixels per row.
    fn fb(&mut self) -> &mut [u16];
    /// Push the whole framebuffer to the screen.
    fn update_all(&mut self) -> io::Result<()>;
    /// Push one rectangle of it.
    fn update_rect(&mut self, x: i32, y: i32, w: i32, h: i32) -> io::Result<()>;
    /// Expensive on the tablet: AppLoad's server sleeps one second after this message before
    /// it reads the next update from this client, so never send it while the user is
    /// interacting (a stroke that toggles modes shows up a second late). In AppLoad as of
    /// October 2026 the mode is stored but not applied to the display at all.
    fn set_refresh_mode(&mut self, _mode: RefreshMode) -> io::Result<()> { Ok(()) }
    /// Same one-second stall as `set_refresh_mode`; only for an explicit user action.
    fn request_full_refresh(&mut self) -> io::Result<()> { Ok(()) }
    /// Wait up to `timeout_ms` (-1 = forever) for the next event.
    /// Ok(None) on timeout; Err when the display is gone.
    fn wait_event(&mut self, timeout_ms: i32) -> io::Result<Option<Event>>;
}

#[cfg(unix)]
impl Backend for crate::qtfb::Client {
    fn width(&self) -> usize { self.width }
    fn height(&self) -> usize { self.height }
    fn fb(&mut self) -> &mut [u16] { crate::qtfb::Client::fb(self) }
    fn update_all(&mut self) -> io::Result<()> { crate::qtfb::Client::update_all(self) }
    fn update_rect(&mut self, x: i32, y: i32, w: i32, h: i32) -> io::Result<()> {
        crate::qtfb::Client::update_rect(self, x, y, w, h)
    }
    fn set_refresh_mode(&mut self, mode: RefreshMode) -> io::Result<()> {
        crate::qtfb::Client::set_refresh_mode(self, mode)
    }
    fn request_full_refresh(&mut self) -> io::Result<()> { crate::qtfb::Client::request_full_refresh(self) }
    fn wait_event(&mut self, timeout_ms: i32) -> io::Result<Option<Event>> {
        crate::qtfb::Client::wait_event(self, timeout_ms)
    }
}
