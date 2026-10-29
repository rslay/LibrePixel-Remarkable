//! Shared code for the reMarkable 2 apps in this repo.
//!
//! Platform (any app):
//!   `backend`  the one trait apps draw and read input through: the tablet or the simulator
//!   `qtfb`     AppLoad's wire protocol and the unix-only socket/shm client
//!   `sim`      the simulator: a tiny HTTP server + browser page
//!   `gfx`      rectangles, RGB565 colors, a canvas that draws into a pixel slice
//!   `font`     5x7 bitmap font
//!
//! Editor (the `librepixel` app):
//!   `doc`      sprite, layers, 16-color palette, compositing, grey "view modes" for the panel
//!   `ase`      the .ase/.aseprite file format (docs/ase-format.txt)
//!   `raster`   drawing primitives on an indexed image: brush, line, rect, ellipse, flood fill
//!   `undo`     snapshot undo/redo with a memory cap
//!   `export`   PNG and JPG files, PNG import, and the PDF bytes
//!   `pdf`      a minimal one-page PDF writer with an embedded image
//!   `xochitl`  drop a PDF into the tablet's own document library

pub mod backend;
pub mod font;
pub mod gfx;
pub mod qtfb;
pub mod sim;

pub mod ase;
pub mod doc;
pub mod export;
pub mod pdf;
pub mod ink;
pub mod raster;
pub mod select;
pub mod settings;
pub mod undo;
pub mod xochitl;

pub use qtfb::{RM2_HEIGHT, RM2_WIDTH};
