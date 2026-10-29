//! The editor: state, input handling, panels, file operations.
//!
//! Input model: the pen draws with the active tool; one finger pans the canvas, two fingers
//! zoom; any press-and-release on a button activates it. Everything is painted into a back
//! buffer and only the dirty rectangles are copied to the display and refreshed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use librepixel::ase;
use librepixel::backend::Backend;
use librepixel::doc::{Image, LayerKind, Rgba, Sprite, ViewMode, PALETTE_SIZE};
use librepixel::export;
use librepixel::gfx::{copy_rect, grey565, Canvas, Rect, BLACK, WHITE};
use librepixel::qtfb::{
    Event, UserInput, INPUT_PEN_PRESS, INPUT_PEN_RELEASE, INPUT_PEN_UPDATE, INPUT_TOUCH_PRESS, INPUT_TOUCH_RELEASE,
    INPUT_TOUCH_UPDATE,
};
use librepixel::ink;
use librepixel::raster;
use librepixel::select::Selection;
use librepixel::settings::{self, Settings};
use librepixel::undo::Undo;
use librepixel::xochitl;

use crate::dialogs::{slider_rect, Dialog, Purpose, NAME_MAX, OPEN_ROWS, SCALES};
use crate::ui::{
    draw_button, draw_buttons, hit, row, wrap, Icon, Id, Layout, Tool, Widget, BG, BRUSH_H, BRUSH_LABEL_Y, BRUSH_PITCH,
    BRUSH_Y, CANVAS, CELL_H, CELL_PITCH_X, CELL_PITCH_Y, CELL_W, CELL_X, CONTIGUOUS_H, CONTIGUOUS_LABEL_Y, CONTIGUOUS_Y,
    DISABLED, LAYER_BUTTONS_Y, LAYER_HEAD_Y, LAYER_ICON_W, LAYER_LOCK_X, LAYER_NAME_X, LAYER_PANEL, LAYER_ROWS_Y,
    LAYER_ROW_H, LAYER_ROW_PITCH, LAYER_VIS_X, PALETTE_PANEL, PIXEL_PERFECT_H, PIXEL_PERFECT_LABEL_Y, PIXEL_PERFECT_Y,
    SCREEN, SCREEN_W, SELECTION_LABEL_Y, SELECTION_Y, SHAPES_LABEL_Y, SHAPES_Y, SMALL, TEXT, TITLE, TITLE_RECT,
    TOOLBAR, TOOL_H, TOOL_PITCH, TOOL_W, TOOL_X, TOOL_Y, TOPBAR,
};
use crate::view::{Marks, View};

/// Set from a signal handler on the device so the loop ends cleanly.
pub static QUIT: AtomicBool = AtomicBool::new(false);

const UNDO_BUDGET: usize = 48 << 20;
const LAYER_ROWS: usize = 5;
/// How long after the eraser end lifts before PEN comes back.
const RESTORE_DELAY: Duration = Duration::from_millis(150);
/// A finger must be down this long before it may pan or zoom, so a palm that lands just before
/// the pen never moves the canvas (the pen usually comes into range within this time)...
const TOUCH_SETTLE: Duration = Duration::from_millis(150);
/// ...and must have moved this far from where it landed (a resting palm wobbles less).
const PAN_THRESHOLD: i32 = 30;
/// After the pen lifts, new touches may still tap a button but not pan or zoom for this long,
/// and a "tap" that stays down longer than TAP_MAX in that time is a resting hand.
const PEN_COOLDOWN: Duration = Duration::from_millis(1200);
const TAP_MAX: Duration = Duration::from_millis(400);
/// The two fingers of a pinch land together; a contact that joins one already down for
/// longer than this is the hand arriving in pieces.
const PAIR_WINDOW: Duration = Duration::from_millis(250);
/// A contact that moves this far between two reports is not a finger sliding.
const TOUCH_JUMP: i32 = 120;
/// A pan or pinch that is still going, or began less than this long ago, when the pen
/// arrives was the hand coming down: the view goes back to where it was.
const PAN_GRACE: Duration = Duration::from_millis(600);
/// A locked layer's lock (or a hidden layer's eye) blinks this often, this many times.
const FLASH_STEP: Duration = Duration::from_millis(160);
const FLASH_PHASES: u32 = 4;
/// Tapping within this many screen pixels of a polygon's first corner closes it.
const POLY_CLOSE_PX: i32 = 24;
/// Touch panel contact slots tracked for contact sizes.
const TOUCH_SLOTS: usize = 16;
/// Shortest gap between two panel updates while the pen is down, per ink mode.
const LIVE_FLUSH: Duration = Duration::from_millis(30);
const BATCHED_FLUSH_DEFAULT_MS: i64 = 100;
const NATIVE_FLUSH: Duration = Duration::from_millis(1000);
/// Bumped when a stored default changes; `apply_settings` migrates older files.
const SETTINGS_VERSION: i64 = 3;
/// Batch intervals earlier versions wrote as their default (v1: 300, v2: 200).
const OLD_BATCHED_DEFAULTS_MS: [i64; 2] = [300, 200];

/// How a stroke reaches the panel. Cycled with the INK button, kept in settings.json.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StrokeMode {
    /// Many small updates (every 30 ms); the live stroke in black and white, greys on pen-up.
    Live,
    /// Fewer, larger updates (one every `batched_flush_ms`, 100 by default), so they never
    /// queue up behind each other on the panel.
    Batched,
    /// The librepixel-ink extension draws the live stroke through xochitl's own pen path; the
    /// editor only settles the real pixels about once a second and on pen-up.
    Native,
}

impl StrokeMode {
    fn key(self) -> &'static str {
        match self {
            StrokeMode::Live => "live",
            StrokeMode::Batched => "batched",
            StrokeMode::Native => "native",
        }
    }
    fn from_key(k: &str) -> Option<StrokeMode> {
        [StrokeMode::Live, StrokeMode::Batched, StrokeMode::Native].into_iter().find(|m| m.key() == k)
    }
    fn label(self) -> &'static str {
        match self {
            StrokeMode::Live => "INK:LIVE",
            StrokeMode::Batched => "INK:BATCH",
            StrokeMode::Native => "INK:NATIVE",
        }
    }
    fn describe(self) -> &'static str {
        match self {
            StrokeMode::Live => "INK: LIVE GHOSTING, MANY SMALL UPDATES",
            StrokeMode::Batched => "INK: BATCHED, FEWER AND LARGER UPDATES",
            StrokeMode::Native => "INK: NATIVE, XOCHITL'S OWN PEN PATH",
        }
    }
}

#[derive(Clone, Copy)]
enum Drag {
    None,
    /// PEN or ERASE; the stroke itself is `App::stroke`.
    Stroke,
    Shape { start: (i32, i32), cur: (i32, i32), shown: Rect },
    Pan { dev: i32, last: (i32, i32) },
    /// Dragging out a rectangle selection; `shown` is where its preview was last painted.
    SelRect { start: (i32, i32), shown: Rect },
    /// Drawing a lasso; the path is `App::lasso`.
    Lasso { shown: Rect },
    /// Moving the selected pixels, lifted into `App::float`.
    Move { start: (i32, i32), cur: (i32, i32), shown: Rect },
}

/// One touch contact. `palm` ones are ignored until they lift; `quick` ones (landed in the
/// cooldown after the pen lifted) may only be a short tap; `canvas` ones landed on the canvas
/// (not a button) and may become a pan once settled.
#[derive(Clone, Copy)]
struct Touch {
    dev: i32,
    x: i32,
    y: i32,
    start: (i32, i32),
    since: Instant,
    palm: bool,
    quick: bool,
    canvas: bool,
}

/// The clipboard: the selected pixels of one layer, kept with their selection so PASTE puts
/// them back where they were and selects them again.
struct Clip {
    sel: Selection,
    px: Image,
}

/// A widget blinking to say "this is in the way" (a locked layer's lock, a hidden layer's eye).
struct Flash {
    id: Id,
    until: Instant,
    next: Instant,
    /// Drawn inverted right now.
    on: bool,
}

/// The view before a finger gesture (pan or pinch) began, so it can be put back when the
/// gesture turns out to have been the hand coming down with the pen.
#[derive(Clone, Copy)]
struct Gesture {
    zoom_idx: usize,
    ox: i32,
    oy: i32,
    since: Instant,
    /// A finger is still down.
    active: bool,
}

/// A pen stroke in progress. Strokes are kept as a path so the pixel-perfect rule can take a
/// corner back once the next point shows it was one.
struct Stroke {
    /// The layer before the stroke started.
    base: Image,
    /// Every pixel the pen passed, in order.
    pts: Vec<(i32, i32)>,
    /// Which of them are drawn (false = a corner the pixel-perfect rule dropped).
    keep: Vec<bool>,
    /// Sprite-space area stamped so far.
    bbox: Rect,
    v: u8,
    brush: i32,
    pixel_perfect: bool,
    /// How many path points the native-ink preview has been given.
    sent: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    New,
    Open,
    Quit,
}

pub struct App<B: Backend> {
    b: B,
    fb: Vec<u16>,
    dirty: Vec<Rect>,
    sprite: Sprite,
    path: Option<PathBuf>,
    modified: bool,
    undo: Undo,
    tool: Tool,
    brush: i32,
    color: u8,
    view: View,
    layer: usize,
    layer_scroll: usize,
    drag: Drag,
    stroke: Option<Stroke>,
    /// Pen path of a lasso being drawn, sprite space.
    lasso: Vec<(i32, i32)>,
    /// Corners of a POLY SEL polygon being built, sprite space; empty when none is.
    poly: Vec<(i32, i32)>,
    /// The selected pixels lifted off the layer while they are being moved.
    float: Option<Image>,
    selection: Option<Selection>,
    /// The selection being dragged out, shown before it is final.
    sel_preview: Option<Selection>,
    clip: Option<Clip>,
    pixel_perfect: bool,
    /// WAND picks only the connected patch (true) or every pixel of that color.
    contiguous: bool,
    /// The top bar's title lines as last drawn (`sync_widgets` repaints them when they change).
    title_shown: (String, String),
    flash: Option<Flash>,
    gesture: Option<Gesture>,
    /// The eraser end of the pen is near the screen (digitizer on the device, page in the sim).
    eraser_near: bool,
    /// The tool that was active when the eraser end switched to ERASE by itself.
    auto_erase_from: Option<Tool>,
    /// When to switch back to PEN after the eraser end lifted.
    restore_at: Option<Instant>,
    /// Last time updates went to the panel; strokes flush at most every `flush_interval`.
    last_flush: Instant,
    mode: StrokeMode,
    /// The librepixel-ink extension is running on this build (checked at start).
    native_ok: bool,
    settings: Settings,
    settings_path: PathBuf,
    /// What the arm file last said (None = off), and when it must be rewritten.
    arm_state: Option<String>,
    arm_due: Instant,
    preview: Option<Image>,
    touches: Vec<Touch>,
    pinch: Option<i32>, // finger distance the next zoom step is measured against
    /// Palm rejection inputs: the pen is touching / in range, when it last lifted, and the
    /// touch panel's contact sizes (a contact at or above `palm_major` is a palm; 0 = off).
    pen_touching: bool,
    pen_near: bool,
    pen_up_at: Option<Instant>,
    touch_majors: [i32; TOUCH_SLOTS],
    big_touch: bool,
    palm_major: i32,
    /// Native ink: the extension's command pipe, fed the blocks of the live stroke.
    ink_cmd: ink::Cmd,
    pressed: Option<Id>,
    /// `pressed` was set by the pen (a finger's pending press is dropped when the palm lands).
    pressed_pen: bool,
    dialog: Option<Dialog>,
    pending: Pending,
    quit: bool,
    dir: PathBuf,
    status: String,
    widgets: Vec<Widget>,
    /// `RM2_TRACE=1`: log every rectangle sent to the panel (readable in the tablet's journal).
    trace: bool,
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "untitled".to_string())
}

impl<B: Backend> App<B> {
    pub fn new(b: B, dir: PathBuf, start: Option<PathBuf>) -> App<B> {
        let (w, h) = (b.width(), b.height());
        let settings_path = settings::default_path();
        let settings = Settings::load(&settings_path);
        let mut app = App {
            b,
            mode: StrokeMode::Batched,
            native_ok: false,
            settings,
            settings_path,
            arm_state: None,
            arm_due: Instant::now(),
            fb: vec![WHITE; w * h],
            dirty: vec![SCREEN],
            sprite: Sprite::new(64, 64),
            path: None,
            modified: false,
            undo: Undo::new(UNDO_BUDGET),
            tool: Tool::Pencil,
            brush: 1,
            color: 1,
            view: View::new(),
            layer: 0,
            layer_scroll: 0,
            drag: Drag::None,
            stroke: None,
            lasso: Vec::new(),
            poly: Vec::new(),
            float: None,
            selection: None,
            sel_preview: None,
            clip: None,
            pixel_perfect: false,
            contiguous: true,
            title_shown: (String::new(), String::new()),
            flash: None,
            gesture: None,
            eraser_near: false,
            auto_erase_from: None,
            restore_at: None,
            last_flush: Instant::now(),
            preview: None,
            touches: Vec::new(),
            pinch: None,
            pen_touching: false,
            pen_near: false,
            pen_up_at: None,
            touch_majors: [0; TOUCH_SLOTS],
            big_touch: false,
            palm_major: 0,
            ink_cmd: ink::Cmd::new(),
            pressed: None,
            pressed_pen: false,
            dialog: None,
            pending: Pending::None,
            quit: false,
            dir,
            status: String::new(),
            widgets: Vec::new(),
            trace: std::env::var("RM2_TRACE").map_or(false, |v| v != "0"),
        };
        app.view.fit(64, 64);
        app.apply_settings();
        if let Some(p) = start {
            app.open_file(&p);
        }
        app
    }

    /// Last session's settings, the ink mode this tablet build allows, and a note if the
    /// tablet's OS changed since the editor last ran.
    fn apply_settings(&mut self) {
        // Older files carry the batch interval that was the default when they were written; one
        // still holding exactly such a value gets the current default, a hand-tuned value is kept.
        if self.settings.int("settings_version").unwrap_or(1) < SETTINGS_VERSION
            && self.settings.int("batched_flush_ms").is_some_and(|v| OLD_BATCHED_DEFAULTS_MS.contains(&v))
        {
            self.settings.set_int("batched_flush_ms", BATCHED_FLUSH_DEFAULT_MS);
        }
        self.settings.set_int("settings_version", SETTINGS_VERSION);
        self.native_ok = ink::available();
        let os = settings::os_version();
        let prev_os = self.settings.str("last_os_version").map(str::to_string);
        self.pixel_perfect = self.settings.bool("pixel_perfect").unwrap_or(false);
        self.contiguous = self.settings.bool("contiguous").unwrap_or(true);
        self.palm_major = self.settings.int("palm_touch_major").unwrap_or(0).clamp(0, 1 << 20) as i32;
        self.view.grid = self.settings.bool("grid").unwrap_or(true);
        self.view.mode = match self.settings.str("view") {
            Some("luma") => ViewMode::Luma,
            _ => ViewMode::Index,
        };
        self.view.invalidate();
        let saved = self.settings.str("stroke_mode").and_then(StrokeMode::from_key);
        // The first time the extension is seen, native ink is the default.
        let first_native = self.native_ok && !self.settings.bool("native_seen").unwrap_or(false);
        self.mode = match saved {
            _ if first_native => StrokeMode::Native,
            Some(m) => m,
            None => StrokeMode::Batched,
        };
        if self.native_ok {
            self.settings.set_bool("native_seen", true);
        }
        if self.mode == StrokeMode::Native && !self.native_ok {
            self.mode = StrokeMode::Batched;
            self.set_status("NATIVE INK OFF: SPRITE-INK EXTENSION NOT RUNNING ON THIS BUILD");
        } else if prev_os.is_some() && prev_os.as_deref() != Some(os.as_str()) {
            self.set_status(&format!("TABLET NOW ON {os}"));
        }
        if let Some(line) = ink::status_line() {
            eprintln!("librepixel: librepixel-ink says: {line}");
        }
        eprintln!("librepixel: ink mode {} (native ink {})", self.mode.key(), if self.native_ok { "available" } else { "unavailable" });
        self.settings.set_str("last_os_version", &os);
        self.save_settings();
    }

    fn save_settings(&mut self) {
        self.settings.set_str("stroke_mode", self.mode.key());
        self.settings.set_bool("pixel_perfect", self.pixel_perfect);
        self.settings.set_bool("contiguous", self.contiguous);
        self.settings.set_bool("grid", self.view.grid);
        self.settings.set_str("view", match self.view.mode {
            ViewMode::Luma => "luma",
            ViewMode::Index => "index",
        });
        if self.settings.int("batched_flush_ms").is_none() {
            self.settings.set_int("batched_flush_ms", BATCHED_FLUSH_DEFAULT_MS);
        }
        if self.settings.int("palm_touch_major").is_none() {
            self.settings.set_int("palm_touch_major", 0); // see README: contact size above which a touch is a palm
        }
        if let Err(e) = self.settings.save(&self.settings_path) {
            eprintln!("librepixel: settings {}: {e}", self.settings_path.display());
        }
    }

    fn flush_interval(&self) -> Duration {
        match self.mode {
            StrokeMode::Live => LIVE_FLUSH,
            StrokeMode::Batched => {
                Duration::from_millis(self.settings.int("batched_flush_ms").unwrap_or(BATCHED_FLUSH_DEFAULT_MS).clamp(30, 5000) as u64)
            }
            StrokeMode::Native => NATIVE_FLUSH,
        }
    }

    /// Tell the librepixel-ink extension what the pen may ink right now (native mode: the sprite's
    /// screen rectangle, the ink width, the eraser radius) or that it must stay off. Rewritten
    /// every second while armed, since the extension treats a stale file as off.
    fn update_arm(&mut self) {
        let want = if self.mode == StrokeMode::Native
            && self.native_ok
            && self.dialog.is_none()
            && matches!(self.tool, Tool::Pencil | Tool::Eraser)
            && self.layer_blocker().is_none()
        {
            let r = self.view.sprite_rect(self.sprite.w, self.sprite.h).intersect(&CANVAS);
            (!r.is_empty()).then(|| {
                let z = self.view.zoom();
                let half = (self.brush as f32 * z as f32 * 0.5).max(2.0);
                // The extension fills whole pixel blocks, so it needs the grid the view uses.
                let (ox, oy) = (CANVAS.x + self.view.ox, CANVAS.y + self.view.oy);
                let grid = self.view.grid && z >= 6;
                ink::arm_line_blocks(r.x, r.y, r.w, r.h, half, self.tool == Tool::Eraser, half, ox, oy, z, self.brush, grid)
            })
        } else {
            None
        };
        let now = Instant::now();
        match (&want, &self.arm_state) {
            (Some(line), prev) if prev.as_deref() != Some(line.as_str()) || now >= self.arm_due => {
                let _ = ink::write_arm(line);
                self.arm_due = now + ink::HEARTBEAT;
            }
            (None, Some(_)) => {
                let _ = ink::disarm();
            }
            _ => {}
        }
        self.arm_state = want;
    }

    pub fn run(mut self) -> i32 {
        self.sync_widgets();
        self.render();
        while !self.quit && !QUIT.load(Ordering::SeqCst) {
            // Wake up for the eraser's switch-back timer, a blinking icon, the next stroke
            // flush or the next native-ink heartbeat; else sleep.
            let mut deadline = self.restore_at;
            if let Some(f) = &self.flash {
                let t = f.next.min(f.until);
                deadline = Some(deadline.map_or(t, |d| d.min(t)));
            }
            if self.pen_busy() && !self.dirty.is_empty() {
                let flush = self.last_flush + self.flush_interval();
                deadline = Some(deadline.map_or(flush, |d| d.min(flush)));
            }
            if self.arm_state.is_some() {
                deadline = Some(deadline.map_or(self.arm_due, |d| d.min(self.arm_due)));
            }
            let timeout = deadline.map_or(-1, |d| d.saturating_duration_since(Instant::now()).as_millis() as i32 + 1);
            match self.b.wait_event(timeout) {
                Err(e) => {
                    eprintln!("librepixel: {e}");
                    return 2;
                }
                Ok(None) => {}
                Ok(Some(ev)) => self.handle(ev),
            }
            // Take everything that is already queued (a pen stroke arrives as many events).
            while let Ok(Some(ev)) = self.b.wait_event(0) {
                self.handle(ev);
            }
            self.tick();
            self.update_arm();
            self.sync_widgets();
            // While the pen is down, updates go to the panel at most every flush interval: the
            // e-ink controller falls behind when it is handed one per pen sample, and each
            // update waits for any overlapping one still running.
            if self.pen_busy() && Instant::now() < self.last_flush + self.flush_interval() {
                continue;
            }
            if !self.dirty.is_empty() {
                self.render();
                self.last_flush = Instant::now();
            }
        }
        if self.arm_state.take().is_some() {
            let _ = ink::disarm();
        }
        eprintln!("librepixel: exit");
        0
    }

    /// The pen is dragging something on the canvas.
    fn pen_busy(&self) -> bool {
        !matches!(self.drag, Drag::None | Drag::Pan { .. })
    }

    /// Timers: the eraser end's switch back to PEN, and the blinking icon.
    fn tick(&mut self) {
        if self.restore_at.is_some_and(|t| Instant::now() >= t) {
            self.restore_pencil();
        }
        if let Some(f) = &mut self.flash {
            let now = Instant::now();
            let id = f.id;
            if now >= f.until {
                self.flash = None;
                self.dirty_widget(id);
            } else if now >= f.next {
                f.on = !f.on;
                f.next = now + FLASH_STEP;
                self.dirty_widget(id);
            }
        }
    }

    /// Start blinking a widget (inverted / normal, FLASH_PHASES times).
    fn flash_widget(&mut self, id: Id) {
        let now = Instant::now();
        self.flash = Some(Flash { id, until: now + FLASH_STEP * FLASH_PHASES, next: now + FLASH_STEP, on: true });
        self.dirty_widget(id);
    }

    fn dirty_widget(&mut self, id: Id) {
        if let Some(w) = self.widgets.iter().find(|w| w.id == id) {
            self.dirty.push(w.rect);
        }
    }

    /// Build the widget list afresh and mark for repaint every button whose look changed since
    /// the last build (appeared, vanished, moved, enabled, pressed in), plus the title lines of
    /// the top bar. This is what makes CUT/COPY/PASTE, UNDO, the modified mark and the rest
    /// follow the state at once instead of at the next full repaint of their panel.
    fn sync_widgets(&mut self) {
        let new = self.build_widgets();
        for w in &new {
            match self.widgets.iter().find(|o| o.id == w.id) {
                Some(o) if o == w => {}
                Some(o) => {
                    self.dirty.push(o.rect);
                    self.dirty.push(w.rect);
                }
                None => self.dirty.push(w.rect),
            }
        }
        for o in &self.widgets {
            if !new.iter().any(|w| w.id == o.id) {
                self.dirty.push(o.rect);
            }
        }
        self.widgets = new;
        let title = self.title_lines();
        if title != self.title_shown {
            self.title_shown = title;
            self.dirty.push(TITLE_RECT);
        }
    }

    /// The top bar's file name (with its modified mark) and the size/zoom line.
    fn title_lines(&self) -> (String, String) {
        let name = self
            .path
            .as_deref()
            .map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            .unwrap_or_else(|| "UNTITLED".to_string());
        let name: String = name.to_uppercase().chars().take(16).collect();
        let title = format!("{name}{}", if self.modified { "*" } else { "" });
        (title, format!("{}X{}  ZOOM {}X", self.sprite.w, self.sprite.h, self.view.zoom()))
    }

    fn restore_pencil(&mut self) {
        self.restore_at = None;
        self.auto_erase_from = None;
        if self.tool == Tool::Eraser {
            self.tool = Tool::Pencil;
            self.dirty.push(TOOLBAR);
        }
    }

    // ---- events --------------------------------------------------------------

    fn handle(&mut self, ev: Event) {
        let i = match ev {
            Event::Input(i) => i,
            Event::Eraser(near) => {
                self.eraser_near = near;
                return;
            }
            Event::PenNear(near) => {
                self.pen_near = near;
                if near {
                    self.reject_touches();
                    self.revert_gesture("pen in range");
                }
                return;
            }
            Event::TouchSize { slot, major } => {
                if let Some(m) = self.touch_majors.get_mut(slot.max(0) as usize) {
                    *m = major;
                }
                if self.trace {
                    eprintln!("librepixel: touch slot {slot} size {major}");
                }
                self.big_touch = self.palm_major > 0 && self.touch_majors.iter().any(|&m| m >= self.palm_major);
                if self.big_touch {
                    self.reject_touches();
                }
                return;
            }
            _ => return,
        };
        let UserInput { input_type, dev_id, x, y, .. } = i;
        match input_type {
            INPUT_PEN_PRESS => {
                self.pen_touching = true;
                self.reject_touches(); // whatever is touching now is the hand holding the pen
                self.revert_gesture("pen down");
                self.press(x, y, true, dev_id);
            }
            INPUT_PEN_UPDATE => self.moved(x, y, true, dev_id),
            INPUT_PEN_RELEASE => {
                self.release(x, y, true, dev_id);
                self.pen_touching = false;
                self.pen_up_at = Some(Instant::now());
            }
            INPUT_TOUCH_PRESS => {
                // A contact joining one that has been down a while, or joining a pan, is not the
                // second finger of a pinch (those land together): it is the hand.
                let late = matches!(self.drag, Drag::Pan { .. })
                    || self.touches.iter().any(|t| t.dev != dev_id && !t.palm && t.since.elapsed() > PAIR_WINDOW);
                let palm = self.hand_down() || self.touches.iter().any(|t| t.palm) || late;
                let quick = !palm && self.pen_up_at.is_some_and(|t| t.elapsed() < PEN_COOLDOWN);
                self.touches.retain(|t| t.dev != dev_id);
                self.touches.push(Touch { dev: dev_id, x, y, start: (x, y), since: Instant::now(), palm, quick, canvas: false });
                if late || self.touches.len() >= 3 {
                    self.reject_touches(); // a palm shows up as several contacts
                    if self.trace {
                        eprintln!("librepixel: touch {dev_id} rejected ({})", if late { "late contact" } else { "three contacts" });
                    }
                    return;
                }
                if palm {
                    return;
                }
                let on_canvas = self.press(x, y, false, dev_id);
                if let Some(t) = self.touches.iter_mut().find(|t| t.dev == dev_id) {
                    t.canvas = on_canvas;
                }
            }
            INPUT_TOUCH_UPDATE => {
                let Some(t) = self.touches.iter_mut().find(|t| t.dev == dev_id) else { return };
                let jump = (x - t.x).abs().max((y - t.y).abs());
                t.x = x;
                t.y = y;
                if t.palm {
                    return;
                }
                if jump > TOUCH_JUMP {
                    // No finger slides that far between two reports: the contact is a palm
                    // whose centre wanders, or several contacts reported as one.
                    self.reject_touches();
                    if self.trace {
                        eprintln!("librepixel: touch {dev_id} rejected (jumped {jump} px)");
                    }
                    return;
                }
                self.moved(x, y, false, dev_id);
            }
            INPUT_TOUCH_RELEASE => {
                let t = self.touches.iter().find(|t| t.dev == dev_id).copied();
                let rested = t.is_some_and(|t| t.quick && t.since.elapsed() > TAP_MAX);
                match t {
                    Some(t) if !t.palm && !rested => self.release(x, y, false, dev_id),
                    _ => {
                        if !self.pressed_pen {
                            self.pressed = None; // a hand resting on a button is not a tap
                        }
                    }
                }
                self.touches.retain(|t| t.dev != dev_id);
                if self.active_touches() < 2 {
                    self.pinch = None;
                }
                if self.active_touches() == 0 {
                    if let Some(g) = &mut self.gesture {
                        g.active = false;
                    }
                }
            }
            _ => {}
        }
    }

    /// A pan or pinch is starting: remember the view so it can be put back.
    fn begin_gesture(&mut self) {
        if !self.gesture.is_some_and(|g| g.active) {
            self.gesture = Some(Gesture {
                zoom_idx: self.view.zoom_idx,
                ox: self.view.ox,
                oy: self.view.oy,
                since: Instant::now(),
                active: true,
            });
        }
    }

    /// The pen arrived: a finger gesture still going, or one that began a moment ago, was the
    /// hand coming down with it, so the view goes back to where it was before the gesture.
    fn revert_gesture(&mut self, why: &str) {
        let Some(g) = self.gesture.take() else { return };
        if !(g.active || g.since.elapsed() < PAN_GRACE) {
            return;
        }
        if (self.view.zoom_idx, self.view.ox, self.view.oy) != (g.zoom_idx, g.ox, g.oy) {
            self.view.zoom_idx = g.zoom_idx;
            self.view.ox = g.ox;
            self.view.oy = g.oy;
            self.dirty.push(CANVAS);
            if self.trace {
                eprintln!("librepixel: finger gesture undone ({why})");
            }
        }
    }

    /// Touches landing now are the hand holding the pen: the pen is on or near the screen, or
    /// the touch panel reports a contact too big for a fingertip.
    fn hand_down(&self) -> bool {
        self.pen_touching || self.pen_near || self.big_touch
    }

    /// Every current touch is the palm: no pan, no zoom, no button, until each one lifts.
    fn reject_touches(&mut self) {
        for t in &mut self.touches {
            t.palm = true;
        }
        if matches!(self.drag, Drag::Pan { .. }) {
            self.drag = Drag::None;
        }
        self.pinch = None;
        if !self.pressed_pen {
            self.pressed = None;
        }
    }

    /// Touches that may pan or zoom: not the palm, not a cooldown tap.
    fn active_touches(&self) -> usize {
        self.touches.iter().filter(|t| !t.palm && !t.quick).count()
    }

    fn fingers(&self) -> (Option<Touch>, Option<Touch>) {
        let mut it = self.touches.iter().filter(|t| !t.palm && !t.quick).copied();
        (it.next(), it.next())
    }

    fn current_widgets(&self) -> Vec<Widget> {
        match &self.dialog {
            Some(d) => d.widgets(),
            None => self.widgets.clone(),
        }
    }

    /// Returns whether the press landed on the canvas (as opposed to a widget or nothing).
    fn press(&mut self, x: i32, y: i32, pen: bool, _dev: i32) -> bool {
        let widgets = self.current_widgets();
        if let Some(id) = hit(&widgets, x, y) {
            if let Id::Slider(ch) = id {
                self.slider_to(ch, x);
                return false;
            }
            self.pressed = Some(id);
            self.pressed_pen = pen;
            return false;
        }
        if self.dialog.is_some() || !CANVAS.contains(x, y) {
            return false;
        }
        if pen {
            self.pen_down(x, y);
        } else if self.active_touches() >= 2 {
            self.drag = Drag::None;
            self.pinch = Some(self.touch_distance());
            self.begin_gesture();
        }
        // A lone finger starts panning in `moved`, once it has settled and actually moved.
        true
    }

    fn moved(&mut self, x: i32, y: i32, pen: bool, dev: i32) {
        if let Some(Dialog::EditColor { drag: Some(ch), .. }) = &self.dialog {
            let ch = *ch;
            self.slider_to(ch, x);
            return;
        }
        if pen {
            self.pen_move(x, y);
        } else if let Some(reference) = self.pinch {
            let settled = self.touches.iter().filter(|t| !t.palm && !t.quick).all(|t| t.since.elapsed() >= TOUCH_SETTLE);
            if settled && self.active_touches() >= 2 {
                let d = self.touch_distance().max(1);
                let (ax, ay) = self.touch_midpoint();
                // One zoom level per 30% change of finger distance, then measure from there.
                let steps = if d * 10 > reference * 13 { 1 } else if d * 13 < reference * 10 { -1 } else { 0 };
                if steps != 0 {
                    if self.view.zoom_by(steps, (ax, ay)) {
                        self.view.clamp_pan(self.sprite.w, self.sprite.h);
                        self.dirty.push(CANVAS);
                        self.dirty.push(TOPBAR);
                    }
                    self.pinch = Some(d);
                }
            }
        } else if let Drag::Pan { dev: d, last } = self.drag {
            if d == dev {
                self.view.ox += x - last.0;
                self.view.oy += y - last.1;
                self.view.clamp_pan(self.sprite.w, self.sprite.h);
                self.drag = Drag::Pan { dev, last: (x, y) };
                self.dirty.push(CANVAS);
            }
        } else if let Some(t) = self.touches.iter().find(|t| t.dev == dev && !t.palm && !t.quick && t.canvas).copied() {
            // A lone finger that has settled and clearly moved starts a pan from here.
            let moved = (x - t.start.0).abs().max((y - t.start.1).abs());
            if self.dialog.is_none() && self.active_touches() == 1 && t.since.elapsed() >= TOUCH_SETTLE && moved >= PAN_THRESHOLD {
                self.drag = Drag::Pan { dev, last: (x, y) };
                self.begin_gesture();
                if self.trace {
                    eprintln!("librepixel: touch {dev} pans");
                }
            }
        }
    }

    fn release(&mut self, x: i32, y: i32, pen: bool, _dev: i32) {
        if let Some(Dialog::EditColor { drag, .. }) = &mut self.dialog {
            *drag = None;
        }
        if let Some(id) = self.pressed.take() {
            let widgets = self.current_widgets();
            if hit(&widgets, x, y) == Some(id) {
                self.activate(id);
            }
            return;
        }
        if pen {
            self.pen_up(x, y);
        } else if matches!(self.drag, Drag::Pan { .. }) {
            self.drag = Drag::None;
        }
    }

    fn touch_distance(&self) -> i32 {
        match self.fingers() {
            (Some(a), Some(b)) => (((a.x - b.x).pow(2) + (a.y - b.y).pow(2)) as f64).sqrt() as i32,
            _ => 0,
        }
    }

    fn touch_midpoint(&self) -> (i32, i32) {
        match self.fingers() {
            (Some(a), Some(b)) => ((a.x + b.x) / 2, (a.y + b.y) / 2),
            _ => CANVAS.center(),
        }
    }

    // ---- the pen on the canvas ---------------------------------------------------

    fn layer_is_image(&mut self) -> bool {
        if self.sprite.layers[self.layer].kind == LayerKind::Image {
            true
        } else {
            self.set_status("SELECT AN IMAGE LAYER, NOT A GROUP");
            false
        }
    }

    /// Why the current layer cannot be painted on right now: the widget to blink and the
    /// message, or None when it can.
    fn layer_blocker(&self) -> Option<(Id, String)> {
        let i = self.layer;
        let l = &self.sprite.layers[i];
        if l.kind != LayerKind::Image {
            return Some((Id::Layer(i), "SELECT AN IMAGE LAYER, NOT A GROUP".to_string()));
        }
        let name = l.name.to_uppercase();
        if !self.sprite.effective_visible(i) {
            return Some((Id::LayerVis(i), format!("{name} IS HIDDEN: SHOW IT TO EDIT IT")));
        }
        if l.locked {
            return Some((Id::LayerLock(i), format!("{name} IS LOCKED")));
        }
        None
    }

    /// The current layer can be edited: an image layer, shown, not locked. Otherwise the
    /// status line says why and the eye or lock of that layer blinks.
    fn layer_editable(&mut self) -> bool {
        match self.layer_blocker() {
            None => true,
            Some((id, msg)) => {
                self.set_status(&msg);
                if !matches!(id, Id::Layer(_)) {
                    self.flash_widget(id);
                }
                false
            }
        }
    }

    fn paint_index(&self) -> u8 {
        if self.tool == Tool::Eraser { self.sprite.transparent } else { self.color }
    }

    fn pen_down(&mut self, x: i32, y: i32) {
        let (sx, sy) = self.view.to_sprite(x, y);
        // The eraser end erases whatever tool is chosen; the tip gets PEN back (see tick).
        if self.eraser_near {
            if self.tool != Tool::Eraser {
                self.auto_erase_from = Some(self.tool);
                self.tool = Tool::Eraser;
                self.dirty.push(TOOLBAR);
            }
            self.restore_at = None;
        } else if self.auto_erase_from == Some(Tool::Pencil) {
            self.restore_pencil();
        }
        match self.tool {
            Tool::Pencil | Tool::Eraser => {
                if !self.layer_editable() {
                    return;
                }
                self.undo.record(&self.sprite);
                let v = self.paint_index();
                let (brush, layer) = (self.brush, self.layer);
                let base = self.sprite.layers[layer].image.clone();
                let r = raster::stamp(&mut self.sprite.layers[layer].image, sx, sy, brush, v);
                clip_to_selection(&mut self.sprite.layers[layer].image, &base, &r, self.selection.as_ref());
                let pixel_perfect = self.pixel_perfect && self.tool == Tool::Pencil;
                self.stroke = Some(Stroke { base, pts: vec![(sx, sy)], keep: vec![true], bbox: r, v, brush, pixel_perfect, sent: 0 });
                self.drag = Drag::Stroke;
                self.view.ink = self.mode == StrokeMode::Live;
                self.stroke_changed(r);
                self.send_ink(false);
            }
            Tool::Fill => {
                if !self.layer_editable() {
                    return;
                }
                self.undo.record(&self.sprite);
                let v = self.paint_index();
                let layer = self.layer;
                let base = self.sprite.layers[layer].image.clone();
                let r = raster::flood_fill(&mut self.sprite.layers[layer].image, sx, sy, v);
                clip_to_selection(&mut self.sprite.layers[layer].image, &base, &r, self.selection.as_ref());
                self.sprite_changed(r);
            }
            Tool::Poly if !self.poly.is_empty() => self.poly_corner(x, y, sx, sy),
            t if t.is_select() => {
                let inside = self.selection.as_ref().is_some_and(|s| s.contains(sx, sy));
                if inside {
                    // Pressed inside the selection: lift its pixels off the layer and drag them.
                    if !self.layer_editable() {
                        return;
                    }
                    self.undo.record(&self.sprite);
                    let layer = self.layer;
                    let t_idx = self.sprite.transparent;
                    let img = &mut self.sprite.layers[layer].image;
                    let sel = self.selection.as_ref().unwrap();
                    let mut float = Image::new(img.w, img.h, t_idx);
                    let b = sel.bounds;
                    for py in b.y..b.bottom() {
                        for px in b.x..b.right() {
                            if sel.contains(px, py) {
                                let k = py as usize * img.w + px as usize;
                                float.px[k] = img.px[k];
                                img.px[k] = t_idx;
                            }
                        }
                    }
                    self.float = Some(float);
                    self.drag = Drag::Move { start: (sx, sy), cur: (sx, sy), shown: Rect::default() };
                    self.update_move_preview();
                } else if t == Tool::Select {
                    self.drag = Drag::SelRect { start: (sx, sy), shown: Rect::default() };
                    self.update_sel_preview((sx, sy));
                } else if t == Tool::Wand {
                    self.wand_select(sx, sy);
                } else if t == Tool::Poly {
                    self.poly_corner(x, y, sx, sy);
                } else {
                    self.lasso = vec![(sx, sy)];
                    self.drag = Drag::Lasso { shown: Rect::default() };
                    self.update_sel_preview((sx, sy));
                }
            }
            Tool::Pick => {
                if let Some(idx) = self.pick(sx, sy) {
                    self.color = idx;
                    self.dirty.push(PALETTE_PANEL);
                }
            }
            t if t.is_shape() => {
                if !self.layer_editable() {
                    return;
                }
                self.drag = Drag::Shape { start: (sx, sy), cur: (sx, sy), shown: Rect::default() };
                self.update_preview();
            }
            _ => {}
        }
    }

    /// POLY SEL: each tap adds a corner; a tap on the first corner (its marker) closes the
    /// polygon and selects what is inside. Fewer than three corners when it closes: nothing.
    fn poly_corner(&mut self, x: i32, y: i32, sx: i32, sy: i32) {
        let (w, h) = (self.sprite.w, self.sprite.h);
        let shown = self.poly_shown();
        if let Some(&first) = self.poly.first() {
            let (cx, cy) = self.view.cell_center(first.0, first.1);
            let on_start = first == (sx, sy) || (x - cx).abs().max((y - cy).abs()) <= POLY_CLOSE_PX;
            if on_start {
                let pts = std::mem::take(&mut self.poly);
                self.sel_preview = None;
                let sel = (pts.len() >= 3).then(|| Selection::from_polygon(w, h, &pts));
                let n = sel.as_ref().map_or(0, |s| s.mask.iter().filter(|&&m| m).count());
                self.set_selection(sel.filter(|s| !s.is_empty()), shown);
                self.set_status(&if n > 0 { format!("POLYGON: {n} PIXELS SELECTED") } else { "POLYGON CANCELLED".to_string() });
                return;
            }
            if self.poly.last() == Some(&(sx, sy)) {
                return;
            }
        }
        self.poly.push((sx, sy));
        self.sel_preview = Some(Selection::from_path(w, h, &self.poly));
        let now = self.poly_shown();
        self.dirty.push(shown.union(&now));
        if self.poly.len() == 1 {
            self.set_status("POLYGON: TAP MORE CORNERS, TAP THE FIRST ONE TO CLOSE");
        }
    }

    /// Screen area of the polygon in progress: its edges so far and the start marker.
    fn poly_shown(&self) -> Rect {
        let Some(&first) = self.poly.first() else { return Rect::default() };
        let mut r = self.view.marker_rect(first.0, first.1);
        if let Some(p) = &self.sel_preview {
            r = r.union(&self.view.to_screen(&p.bounds));
        }
        r.intersect(&CANVAS)
    }

    /// Drop a polygon in progress (tool change, DESELECT, new file).
    fn cancel_poly(&mut self) {
        if self.poly.is_empty() {
            return;
        }
        let shown = self.poly_shown();
        self.poly.clear();
        self.sel_preview = None;
        self.dirty.push(shown);
    }

    fn pen_move(&mut self, x: i32, y: i32) {
        let (sx, sy) = self.view.to_sprite(x, y);
        match self.drag {
            Drag::Stroke => {
                let r = self.extend_stroke(sx, sy);
                self.stroke_changed(r);
                self.send_ink(false);
            }
            Drag::Shape { start, cur, shown } => {
                if cur == (sx, sy) {
                    return;
                }
                self.drag = Drag::Shape { start, cur: (sx, sy), shown };
                self.update_preview();
            }
            Drag::SelRect { .. } | Drag::Lasso { .. } => self.update_sel_preview((sx, sy)),
            Drag::Move { start, cur, shown } => {
                if cur == (sx, sy) {
                    return;
                }
                self.drag = Drag::Move { start, cur: (sx, sy), shown };
                self.update_move_preview();
            }
            _ => {}
        }
    }

    fn pen_up(&mut self, x: i32, y: i32) {
        let (sx, sy) = self.view.to_sprite(x, y);
        match self.drag {
            Drag::Stroke => {
                self.drag = Drag::None;
                self.send_ink(true);
                if self.mode == StrokeMode::Native && self.native_ok {
                    self.ink_cmd.end();
                }
                if let Some(st) = self.stroke.take() {
                    let mut r = st.bbox;
                    if self.mode == StrokeMode::Native {
                        // xochitl's live ink is a round line: repaint the cells its edges reach.
                        r = r.inset(-(st.brush / 2 + 2));
                    }
                    if self.view.ink {
                        // The live stroke was solid black or white; now show its real grey.
                        self.view.patch(&self.sprite, &st.bbox);
                    }
                    if self.view.ink || self.mode == StrokeMode::Native {
                        self.dirty.push(self.view.to_screen(&r).intersect(&CANVAS));
                    }
                }
                self.view.ink = false;
                if self.tool == Tool::Eraser {
                    match self.auto_erase_from {
                        Some(Tool::Pencil) => self.restore_at = Some(Instant::now() + RESTORE_DELAY),
                        Some(_) => self.auto_erase_from = None, // other tools stay on ERASE
                        None => {}
                    }
                }
            }
            Drag::Shape { start, shown, .. } => {
                self.drag = Drag::None;
                self.preview = None;
                self.undo.record(&self.sprite);
                let v = self.paint_index();
                let (tool, brush, layer) = (self.tool, self.brush, self.layer);
                let base = self.sprite.layers[layer].image.clone();
                let r = apply_shape(&mut self.sprite.layers[layer].image, tool, start, (sx, sy), brush, v);
                clip_to_selection(&mut self.sprite.layers[layer].image, &base, &r, self.selection.as_ref());
                self.sprite_changed(r);
                self.dirty.push(shown);
            }
            Drag::SelRect { start, shown } => {
                self.drag = Drag::None;
                self.sel_preview = None;
                let (w, h) = (self.sprite.w, self.sprite.h);
                // A tap without a drag deselects.
                let sel = (start != (sx, sy)).then(|| Selection::from_rect(w, h, &Rect::from_points(start.0, start.1, sx, sy)));
                self.set_selection(sel.filter(|s| !s.is_empty()), shown);
            }
            Drag::Lasso { shown } => {
                self.drag = Drag::None;
                self.sel_preview = None;
                let (w, h) = (self.sprite.w, self.sprite.h);
                let path = std::mem::take(&mut self.lasso);
                let sel = (path.len() >= 3).then(|| Selection::from_polygon(w, h, &path));
                self.set_selection(sel.filter(|s| !s.is_empty()), shown);
            }
            Drag::Move { start, shown, .. } => {
                self.drag = Drag::Move { start, cur: (sx, sy), shown };
                self.update_move_preview();
                let Drag::Move { shown, .. } = self.drag else { return };
                self.drag = Drag::None;
                let (dx, dy) = (sx - start.0, sy - start.1);
                let layer = self.layer;
                if let (Some(img), Some(sel)) = (self.preview.take(), self.selection.take()) {
                    let old = sel.bounds;
                    self.sprite.layers[layer].image = img;
                    let moved = sel.translated(dx, dy);
                    let r = old.union(&moved.bounds);
                    self.selection = (!moved.is_empty()).then_some(moved);
                    self.sprite_changed(r);
                }
                self.float = None;
                self.dirty.push(shown);
                self.dirty.push(TOPBAR);
                if (dx, dy) == (0, 0) && self.tool == Tool::Wand {
                    // A tap inside the selection with the wand picks anew from that pixel.
                    self.wand_select(sx, sy);
                }
            }
            _ => {}
        }
    }

    /// WAND: select the pixels of the current layer that match the one under the pen.
    fn wand_select(&mut self, sx: i32, sy: i32) {
        if !self.layer_is_image() {
            return;
        }
        let sel = Selection::wand(&self.sprite.layers[self.layer].image, sx, sy, self.contiguous);
        let n = sel.mask.iter().filter(|&&m| m).count();
        self.set_selection((!sel.is_empty()).then_some(sel), Rect::default());
        if n > 0 {
            self.set_status(&format!("WAND: {n} PIXELS SELECTED"));
        }
    }

    /// COPY / CUT: the selected pixels of the current layer go to the clipboard.
    fn copy_selection(&mut self, cut: bool) {
        let Some(sel) = self.selection.clone() else { return };
        if !self.layer_is_image() || (cut && !self.layer_editable()) {
            return;
        }
        let layer = self.layer;
        let n = sel.mask.iter().filter(|&&m| m).count();
        self.clip = Some(Clip { sel: sel.clone(), px: self.sprite.layers[layer].image.clone() });
        if cut {
            self.undo.record(&self.sprite);
            let t = self.sprite.transparent;
            let img = &mut self.sprite.layers[layer].image;
            let b = sel.bounds;
            for y in b.y..b.bottom() {
                for x in b.x..b.right() {
                    if sel.contains(x, y) {
                        img.px[y as usize * img.w + x as usize] = t;
                    }
                }
            }
            self.sprite_changed(b);
            self.set_selection(None, Rect::default());
            self.set_status(&format!("CUT {n} PIXELS"));
        } else {
            self.set_status(&format!("COPIED {n} PIXELS"));
        }
    }

    /// PASTE: the clipboard's pixels go back to where they came from, on the current layer,
    /// and become the selection so they can be moved.
    fn paste_clip(&mut self) {
        let Some((sel, src)) = self.clip.as_ref().map(|c| (c.sel.clone(), c.px.clone())) else { return };
        if !self.layer_editable() {
            return;
        }
        if sel.w != self.sprite.w || sel.h != self.sprite.h {
            self.set_status("CLIPBOARD IS FROM A SPRITE OF ANOTHER SIZE");
            return;
        }
        self.undo.record(&self.sprite);
        let layer = self.layer;
        let b = sel.bounds;
        {
            let img = &mut self.sprite.layers[layer].image;
            for y in b.y..b.bottom() {
                for x in b.x..b.right() {
                    if sel.contains(x, y) {
                        let k = y as usize * img.w + x as usize;
                        img.px[k] = src.px[k];
                    }
                }
            }
        }
        let n = sel.mask.iter().filter(|&&m| m).count();
        self.sprite_changed(b);
        self.set_selection(Some(sel), Rect::default());
        self.set_status(&format!("PASTED {n} PIXELS"));
    }

    /// Continue the stroke to (sx, sy): walk the pixels in between, apply the pixel-perfect
    /// rule, draw what is new, and take back a corner the rule just rejected. Returns the
    /// sprite-space rectangle that changed.
    fn extend_stroke(&mut self, sx: i32, sy: i32) -> Rect {
        let layer = self.layer;
        let sel = self.selection.as_ref();
        let Some(st) = self.stroke.as_mut() else { return Rect::default() };
        let last = *st.pts.last().unwrap();
        if last == (sx, sy) {
            return Rect::default();
        }
        raster::line_points(last.0, last.1, sx, sy, |x, y| {
            if (x, y) != last {
                st.pts.push((x, y));
            }
        });
        let old = st.keep.len();
        let keep = if st.pixel_perfect { raster::pixel_perfect_keep(&st.pts) } else { vec![true; st.pts.len()] };
        let img = &mut self.sprite.layers[layer].image;
        let mut changed = Rect::default();
        let flipped: Vec<usize> = (0..old).filter(|&i| keep[i] != st.keep[i]).collect();
        if flipped.is_empty() {
            for i in old..st.pts.len() {
                if keep[i] {
                    changed = changed.union(&raster::stamp(img, st.pts[i].0, st.pts[i].1, st.brush, st.v));
                }
            }
        } else {
            // Start the stroke over from the untouched layer, without the rejected corners.
            raster::copy_rect(img, &st.base, &st.bbox);
            for (i, &(x, y)) in st.pts.iter().enumerate() {
                if keep[i] {
                    let r = raster::stamp(img, x, y, st.brush, st.v);
                    if i >= old || flipped.contains(&i) {
                        changed = changed.union(&r);
                    }
                }
            }
            for &i in &flipped {
                changed = changed.union(&raster::stamp_rect(img, st.pts[i].0, st.pts[i].1, st.brush));
            }
        }
        st.keep = keep;
        st.bbox = st.bbox.union(&changed);
        clip_to_selection(img, &st.base, &st.bbox, sel);
        changed
    }

    /// Native ink: hand the extension the blocks of this stroke whose fate is settled. With
    /// pixel-perfect on, the newest point may still turn out to be a corner, so it waits for
    /// the next one (or the pen lifting, `all`). The blocks are the editor's own stamps, so the
    /// preview and the final pixels agree.
    fn send_ink(&mut self, all: bool) {
        if self.mode != StrokeMode::Native || !self.native_ok {
            return;
        }
        let Some(st) = self.stroke.as_mut() else { return };
        let upto = if all || !st.pixel_perfect { st.pts.len() } else { st.pts.len().saturating_sub(1).max(1) };
        let (z, ox, oy) = (self.view.zoom(), CANVAS.x + self.view.ox, CANVAS.y + self.view.oy);
        let o = (st.brush - 1) / 2;
        let white = st.v == self.sprite.transparent;
        let image = Rect::new(0, 0, self.sprite.w as i32, self.sprite.h as i32);
        for i in st.sent..upto.min(st.pts.len()) {
            if st.keep[i] {
                let (px, py) = st.pts[i];
                let r = Rect::new(px - o, py - o, st.brush, st.brush).intersect(&image);
                if !r.is_empty() {
                    self.ink_cmd.fill(ox + r.x * z, oy + r.y * z, r.w * z, r.h * z, white);
                }
            }
        }
        st.sent = st.sent.max(upto);
    }

    /// A live stroke changed `r`: repaint only that (in solid ink in live mode).
    fn stroke_changed(&mut self, r: Rect) {
        if !r.is_empty() {
            let ink = if self.tool == Tool::Eraser { Rgba::rgb(255, 255, 255) } else { Rgba::rgb(0, 0, 0) };
            match &self.stroke {
                Some(st) if self.view.ink => {
                    self.view.patch_ink(&self.sprite, &r, &self.sprite.layers[self.layer].image, &st.base, ink)
                }
                _ => self.view.patch(&self.sprite, &r),
            }
            self.dirty.push(self.view.to_screen(&r).intersect(&CANVAS));
        }
        self.modified = true; // the mark in the top bar follows through sync_widgets
    }

    /// Replace the selection, repainting where the old and new outlines are.
    fn set_selection(&mut self, sel: Option<Selection>, shown: Rect) {
        let mut r = shown;
        if let Some(old) = &self.selection {
            r = r.union(&self.view.to_screen(&old.bounds));
        }
        if let Some(new) = &sel {
            r = r.union(&self.view.to_screen(&new.bounds));
        }
        self.selection = sel;
        self.dirty.push(r.intersect(&CANVAS)); // CLEAR / DESELECT follow through sync_widgets
    }

    /// The rectangle or lasso being dragged out, redrawn where it was and where it is.
    fn update_sel_preview(&mut self, cur: (i32, i32)) {
        let (w, h) = (self.sprite.w, self.sprite.h);
        let (sel, old) = match self.drag {
            Drag::SelRect { start, shown } => {
                (Selection::from_rect(w, h, &Rect::from_points(start.0, start.1, cur.0, cur.1)), shown)
            }
            Drag::Lasso { shown } => {
                if self.lasso.last() != Some(&cur) {
                    self.lasso.push(cur);
                }
                (Selection::from_path(w, h, &self.lasso), shown)
            }
            _ => return,
        };
        let now = self.view.to_screen(&sel.bounds).intersect(&CANVAS);
        self.dirty.push(old.union(&now));
        self.sel_preview = Some(sel);
        match &mut self.drag {
            Drag::SelRect { shown, .. } | Drag::Lasso { shown } => *shown = now,
            _ => {}
        }
    }

    /// The lifted pixels drawn at the current drag offset.
    fn update_move_preview(&mut self) {
        let Drag::Move { start, cur, shown } = self.drag else { return };
        let (Some(float), Some(sel)) = (self.float.as_ref(), self.selection.as_ref()) else { return };
        let (dx, dy) = (cur.0 - start.0, cur.1 - start.1);
        let mut img = self.sprite.layers[self.layer].image.clone();
        let b = sel.bounds;
        for y in b.y..b.bottom() {
            for x in b.x..b.right() {
                if sel.contains(x, y) && img.in_bounds(x + dx, y + dy) {
                    img.set(x + dx, y + dy, float.px[y as usize * img.w + x as usize]);
                }
            }
        }
        self.preview = Some(img);
        let moved = Rect::new(b.x + dx, b.y + dy, b.w, b.h);
        let now = self.view.to_screen(&moved).intersect(&CANVAS);
        let mut r = shown.union(&now);
        if shown.is_empty() {
            r = r.union(&self.view.to_screen(&b).intersect(&CANVAS)); // the hole left behind
        }
        self.dirty.push(r);
        self.drag = Drag::Move { start, cur, shown: now };
    }

    /// Redraw the shape being dragged: the old and new extents of the preview.
    fn update_preview(&mut self) {
        let Drag::Shape { start, cur, shown } = self.drag else { return };
        let base = &self.sprite.layers[self.layer].image;
        let mut img = base.clone();
        let v = self.paint_index();
        let r = apply_shape(&mut img, self.tool, start, cur, self.brush, v);
        clip_to_selection(&mut img, base, &r, self.selection.as_ref());
        self.preview = Some(img);
        let now = self.view.to_screen(&r).intersect(&CANVAS);
        self.dirty.push(shown.union(&now));
        self.drag = Drag::Shape { start, cur, shown: now };
    }

    /// The palette index under a sprite pixel: the current layer's, else the topmost visible.
    fn pick(&self, sx: i32, sy: i32) -> Option<u8> {
        let s = &self.sprite;
        let cur = &s.layers[self.layer];
        if cur.kind == LayerKind::Image {
            if let Some(i) = cur.image.get(sx, sy) {
                if cur.background || i != s.transparent {
                    return Some(i);
                }
            }
        }
        for (k, l) in s.layers.iter().enumerate().rev() {
            if l.kind != LayerKind::Image || !s.effective_visible(k) {
                continue;
            }
            if let Some(i) = l.image.get(sx, sy) {
                if l.background || i != s.transparent {
                    return Some(i);
                }
            }
        }
        None
    }

    /// An edit touched `touched` (sprite space): patch the composite and repaint just that.
    /// During a stroke this is the only thing that reaches the panel, so it stays small.
    fn sprite_changed(&mut self, touched: Rect) {
        if !touched.is_empty() {
            self.view.patch(&self.sprite, &touched);
            self.dirty.push(self.view.to_screen(&touched).intersect(&CANVAS));
        }
        self.modified = true;
    }

    fn set_status(&mut self, s: &str) {
        self.status = s.to_string();
        self.dirty.push(PALETTE_PANEL); // the status line lives under the palette
    }

    // ---- buttons -----------------------------------------------------------------

    fn activate(&mut self, id: Id) {
        if self.dialog.is_some() {
            self.dialog_activate(id);
            return;
        }
        match id {
            Id::New => self.guard(Pending::New),
            Id::Open => self.guard(Pending::Open),
            Id::Quit => self.guard(Pending::Quit),
            Id::Save => self.save(),
            Id::SaveAs => self.save_as(),
            Id::Export => {
                let name = self.path.as_deref().map(stem).unwrap_or_else(|| "untitled".to_string());
                self.show(Dialog::Export { format: 0, scale: 2, to_library: xochitl::library_dir().is_some(), name });
            }
            Id::Undo => {
                if let Some(s) = self.undo.undo(&self.sprite) {
                    self.replace_sprite(s);
                }
            }
            Id::Redo => {
                if let Some(s) = self.undo.redo(&self.sprite) {
                    self.replace_sprite(s);
                }
            }
            Id::ZoomOut | Id::ZoomIn => {
                let (cx, cy) = CANVAS.center();
                if self.view.zoom_by(if id == Id::ZoomIn { 1 } else { -1 }, (cx, cy)) {
                    self.view.clamp_pan(self.sprite.w, self.sprite.h);
                    self.dirty.push(CANVAS);
                    self.dirty.push(TOPBAR);
                }
            }
            Id::Fit => {
                self.view.fit(self.sprite.w, self.sprite.h);
                self.dirty.push(CANVAS);
                self.dirty.push(TOPBAR);
            }
            Id::View => {
                self.view.mode = match self.view.mode {
                    ViewMode::Index => ViewMode::Luma,
                    ViewMode::Luma => ViewMode::Index,
                };
                self.view.invalidate();
                self.dirty.push(CANVAS);
                self.dirty.push(TOPBAR);
                self.dirty.push(PALETTE_PANEL);
                self.save_settings();
            }
            Id::Grid => {
                self.view.grid = !self.view.grid;
                self.dirty.push(CANVAS);
                self.dirty.push(TOPBAR);
                self.save_settings();
            }
            Id::InkMode => {
                self.mode = match self.mode {
                    StrokeMode::Live => StrokeMode::Batched,
                    StrokeMode::Batched if self.native_ok => StrokeMode::Native,
                    StrokeMode::Batched | StrokeMode::Native => StrokeMode::Live,
                };
                let msg = if self.mode == StrokeMode::Live && !self.native_ok {
                    "INK: LIVE GHOSTING. NATIVE NEEDS THE SPRITE-INK EXTENSION (README)"
                } else {
                    self.mode.describe()
                };
                self.set_status(msg);
                self.save_settings();
                self.dirty.push(TOPBAR);
            }
            Id::Refresh => {
                let _ = self.b.request_full_refresh();
                self.dirty.push(SCREEN);
            }
            Id::Tool(t) => {
                if t != Tool::Poly {
                    self.cancel_poly();
                }
                self.tool = t;
                self.auto_erase_from = None;
                self.restore_at = None;
                self.dirty.push(TOOLBAR);
            }
            Id::Brush(n) => {
                self.brush = n;
                self.dirty.push(TOOLBAR);
            }
            Id::PixelPerfect => {
                self.pixel_perfect = !self.pixel_perfect;
                self.dirty.push(TOOLBAR);
                self.save_settings();
            }
            Id::Contiguous => {
                self.contiguous = !self.contiguous;
                self.dirty.push(TOOLBAR);
                self.save_settings();
            }
            Id::SelCut => self.copy_selection(true),
            Id::SelCopy => self.copy_selection(false),
            Id::SelPaste => self.paste_clip(),
            Id::SelClear => {
                if let Some(sel) = self.selection.clone() {
                    if self.layer_editable() {
                        self.undo.record(&self.sprite);
                        let layer = self.layer;
                        let t = self.sprite.transparent;
                        let img = &mut self.sprite.layers[layer].image;
                        let b = sel.bounds;
                        for y in b.y..b.bottom() {
                            for x in b.x..b.right() {
                                if sel.contains(x, y) {
                                    img.px[y as usize * img.w + x as usize] = t;
                                }
                            }
                        }
                        self.sprite_changed(b);
                    }
                }
            }
            Id::SelNone => {
                self.cancel_poly();
                self.set_selection(None, Rect::default());
            }
            Id::Color(i) => {
                self.color = i;
                self.dirty.push(PALETTE_PANEL);
            }
            Id::EditColor => {
                let c = self.sprite.palette[self.color as usize];
                self.show(Dialog::EditColor { index: self.color, rgb: [c.r, c.g, c.b], drag: None });
            }
            Id::LayerAdd => {
                self.undo.record(&self.sprite);
                let name = self.sprite.next_layer_name();
                self.layer = self.sprite.insert_layer_above(self.layer, &name);
                self.layers_changed();
            }
            Id::LayerDel => {
                if self.sprite.layers.len() > 1 {
                    self.undo.record(&self.sprite);
                    self.sprite.remove_layer(self.layer);
                    self.layer = self.layer.min(self.sprite.layers.len() - 1);
                    self.layers_changed();
                }
            }
            Id::LayerUp | Id::LayerDown => {
                let up = id == Id::LayerUp;
                let target = if up { self.layer + 1 } else { self.layer.wrapping_sub(1) };
                if target < self.sprite.layers.len() {
                    self.undo.record(&self.sprite);
                    self.layer = self.sprite.move_layer(self.layer, up);
                    self.layers_changed();
                }
            }
            Id::LayerRename => {
                let name = self.sprite.layers[self.layer].name.to_uppercase();
                self.show(Dialog::Text { title: "LAYER NAME".to_string(), name, purpose: Purpose::RenameLayer });
            }
            Id::Layer(i) => {
                if i < self.sprite.layers.len() {
                    self.layer = i;
                    self.dirty.push(LAYER_PANEL);
                }
            }
            Id::LayerVis(i) => {
                if i < self.sprite.layers.len() {
                    self.undo.record(&self.sprite);
                    self.sprite.layers[i].visible = !self.sprite.layers[i].visible;
                    self.view.invalidate();
                    self.modified = true;
                    self.dirty.push(CANVAS);
                    self.dirty.push(LAYER_PANEL);
                }
            }
            Id::LayerLock(i) => {
                if i < self.sprite.layers.len() {
                    self.undo.record(&self.sprite);
                    self.sprite.layers[i].locked = !self.sprite.layers[i].locked;
                    self.modified = true;
                    self.dirty.push(LAYER_PANEL);
                }
            }
            Id::LayerScrollUp => {
                self.layer_scroll = self.layer_scroll.saturating_sub(1);
                self.dirty.push(LAYER_PANEL);
            }
            Id::LayerScrollDown => {
                if self.layer_scroll + LAYER_ROWS < self.sprite.layers.len() {
                    self.layer_scroll += 1;
                }
                self.dirty.push(LAYER_PANEL);
            }
            _ => {}
        }
    }

    fn layers_changed(&mut self) {
        self.view.invalidate();
        self.modified = true;
        self.ensure_layer_visible();
        self.dirty.push(CANVAS);
        self.dirty.push(LAYER_PANEL);
        self.dirty.push(TOPBAR);
    }

    fn ensure_layer_visible(&mut self) {
        // Rows show the top layer first; row r holds layer n-1-r.
        let n = self.sprite.layers.len();
        let r = n - 1 - self.layer.min(n - 1);
        if r < self.layer_scroll {
            self.layer_scroll = r;
        } else if r >= self.layer_scroll + LAYER_ROWS {
            self.layer_scroll = r + 1 - LAYER_ROWS;
        }
        self.layer_scroll = self.layer_scroll.min(n.saturating_sub(LAYER_ROWS));
    }

    fn replace_sprite(&mut self, s: Sprite) {
        self.sprite = s;
        self.layer = self.layer.min(self.sprite.layers.len() - 1);
        self.view.invalidate();
        self.modified = true;
        self.ensure_layer_visible();
        self.dirty.push(CANVAS);
        self.dirty.push(LAYER_PANEL);
        self.dirty.push(PALETTE_PANEL);
        self.dirty.push(TOPBAR);
    }

    /// Ask about unsaved changes before New / Open / Quit.
    fn guard(&mut self, what: Pending) {
        if self.modified {
            self.pending = what;
            self.show(Dialog::message_with(
                "UNSAVED CHANGES",
                "This sprite has changes that are not saved.",
                &[("CANCEL", Id::Cancel), ("DISCARD", Id::Discard), ("SAVE", Id::SaveFirst)],
            ));
        } else {
            self.pending = what;
            self.proceed();
        }
    }

    fn proceed(&mut self) {
        match std::mem::replace(&mut self.pending, Pending::None) {
            Pending::None => {}
            Pending::New => self.show(Dialog::NewSprite { w: self.sprite.w as i32, h: self.sprite.h as i32 }),
            Pending::Open => {
                let files = self.list_files();
                self.show(Dialog::Open { files, selected: None, page: 0 });
            }
            Pending::Quit => self.quit = true,
        }
    }

    // ---- dialogs -----------------------------------------------------------------

    fn show(&mut self, d: Dialog) {
        self.dirty.push(d.rect());
        self.dialog = Some(d);
    }

    fn close_dialog(&mut self) {
        if let Some(d) = self.dialog.take() {
            self.dirty.push(d.rect());
        }
    }

    fn slider_to(&mut self, ch: usize, x: i32) {
        let Some(Dialog::EditColor { rgb, drag, .. }) = &mut self.dialog else { return };
        let r = slider_rect(&Dialog::EditColor { index: 0, rgb: *rgb, drag: None }.rect(), ch);
        rgb[ch] = ((x - r.x).clamp(0, r.w) * 255 / r.w) as u8;
        *drag = Some(ch);
        let d = self.dialog.as_ref().unwrap().rect();
        self.dirty.push(d);
    }

    fn dialog_activate(&mut self, id: Id) {
        let Some(dialog) = self.dialog.as_mut() else { return };
        let rect = dialog.rect();
        match dialog {
            Dialog::Message { .. } => match id {
                Id::Ok => self.close_dialog(),
                Id::Cancel => {
                    self.pending = Pending::None;
                    self.close_dialog();
                }
                Id::Discard => {
                    self.close_dialog();
                    self.modified = false;
                    self.proceed();
                }
                Id::SaveFirst => {
                    self.close_dialog();
                    if self.path.is_some() {
                        self.save();
                        if !self.modified {
                            self.proceed();
                        }
                    } else {
                        self.save_as(); // the pending action continues after the name is given
                    }
                }
                Id::Restart => {
                    match xochitl::restart_xochitl() {
                        Ok(()) => self.quit = true,
                        Err(e) => self.set_status(&format!("RESTART FAILED: {e}")),
                    }
                    self.close_dialog();
                }
                _ => {}
            },
            Dialog::NewSprite { w, h } => match id {
                Id::SizeW(d) => *w = (*w + d).clamp(1, 1024),
                Id::SizeH(d) => *h = (*h + d).clamp(1, 1024),
                Id::Preset(s) => {
                    *w = s;
                    *h = s;
                }
                Id::Ok => {
                    let (w, h) = (*w as usize, *h as usize);
                    self.close_dialog();
                    self.load_sprite(Sprite::new(w, h), None);
                    self.set_status("NEW SPRITE");
                }
                Id::Cancel => self.close_dialog(),
                _ => {}
            },
            Dialog::Open { files, selected, page } => match id {
                Id::Item(i) => *selected = Some(i),
                Id::PageUp => *page = page.saturating_sub(1),
                Id::PageDown => {
                    if (*page + 1) * OPEN_ROWS < files.len() {
                        *page += 1;
                    }
                }
                Id::Ok => {
                    if let Some(i) = *selected {
                        let p = self.dir.join(&files[i]);
                        self.close_dialog();
                        self.open_file(&p);
                    }
                }
                Id::Cancel => self.close_dialog(),
                _ => {}
            },
            Dialog::Text { name, purpose, .. } => match id {
                Id::Key(c) => {
                    if name.chars().count() < NAME_MAX {
                        name.push(c);
                    }
                }
                Id::Space => {
                    if name.chars().count() < NAME_MAX && !name.is_empty() {
                        name.push(' ');
                    }
                }
                Id::Backspace => {
                    name.pop();
                }
                Id::Ok => {
                    let name = name.trim().to_string();
                    let purpose = *purpose;
                    if name.is_empty() {
                        return;
                    }
                    self.close_dialog();
                    match purpose {
                        Purpose::SaveFile => {
                            let p = self.dir.join(format!("{name}.ase"));
                            self.path = Some(p);
                            self.save();
                            if !self.modified {
                                self.proceed();
                            }
                        }
                        Purpose::RenameLayer => {
                            self.undo.record(&self.sprite);
                            self.sprite.layers[self.layer].name = name;
                            self.modified = true;
                            self.dirty.push(LAYER_PANEL);
                        }
                    }
                }
                Id::Cancel => {
                    self.pending = Pending::None;
                    self.close_dialog();
                }
                _ => {}
            },
            Dialog::Export { format, scale, to_library, name } => match id {
                Id::Format(i) => *format = i,
                Id::Scale(i) => *scale = i,
                Id::Ok => {
                    let (format, scale, to_library, name) = (*format, SCALES[*scale], *to_library, name.clone());
                    self.close_dialog();
                    self.do_export(format, scale, to_library, &name);
                }
                Id::Cancel => self.close_dialog(),
                _ => {}
            },
            Dialog::EditColor { index, rgb, .. } => match id {
                Id::Ok => {
                    let (index, rgb) = (*index as usize, *rgb);
                    self.close_dialog();
                    self.undo.record(&self.sprite);
                    self.sprite.palette[index] = Rgba::rgb(rgb[0], rgb[1], rgb[2]);
                    self.view.invalidate();
                    self.modified = true;
                    self.dirty.push(CANVAS);
                    self.dirty.push(PALETTE_PANEL);
                }
                Id::Cancel => self.close_dialog(),
                _ => {}
            },
        }
        if self.dialog.is_some() {
            self.dirty.push(rect);
        }
    }

    // ---- files -------------------------------------------------------------------

    fn list_files(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| {
                        let l = n.to_lowercase();
                        l.ends_with(".ase") || l.ends_with(".aseprite") || l.ends_with(".png")
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|n| n.to_lowercase());
        out
    }

    fn load_sprite(&mut self, s: Sprite, path: Option<PathBuf>) {
        self.view.fit(s.w, s.h);
        self.view.invalidate();
        self.sprite = s;
        self.path = path;
        self.modified = false;
        self.undo.clear();
        self.layer = self.sprite.layers.len() - 1;
        self.layer_scroll = 0;
        self.preview = None;
        self.drag = Drag::None;
        self.stroke = None;
        self.float = None;
        self.selection = None;
        self.sel_preview = None;
        self.lasso.clear();
        self.poly.clear();
        self.flash = None;
        self.gesture = None;
        self.view.ink = false;
        self.color = self.color.min(PALETTE_SIZE as u8 - 1);
        self.dirty.push(SCREEN);
    }

    fn open_file(&mut self, p: &Path) {
        let lower = p.to_string_lossy().to_lowercase();
        let result = if lower.ends_with(".png") {
            export::read_png(p).map(|s| (s, vec![], None))
        } else {
            ase::load(p).map(|l| (l.sprite, l.warnings, Some(p.to_path_buf())))
        };
        match result {
            Ok((s, warnings, path)) => {
                self.load_sprite(s, path);
                self.set_status(&format!("OPENED {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                if !warnings.is_empty() {
                    self.show(Dialog::message("OPENED WITH NOTES", &warnings.join("\n")));
                }
            }
            Err(e) => self.show(Dialog::message("COULD NOT OPEN", &format!("{}: {e}", p.display()))),
        }
    }

    fn save(&mut self) {
        let Some(p) = self.path.clone() else {
            self.save_as();
            return;
        };
        match std::fs::create_dir_all(&self.dir).and_then(|_| ase::save(&p, &self.sprite)) {
            Ok(()) => {
                self.modified = false;
                self.set_status(&format!("SAVED {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
            }
            Err(e) => self.show(Dialog::message("COULD NOT SAVE", &format!("{}: {e}", p.display()))),
        }
    }

    fn save_as(&mut self) {
        let name = self.path.as_deref().map(stem).unwrap_or_default().to_uppercase();
        self.show(Dialog::Text { title: "SAVE AS (.ASE)".to_string(), name, purpose: Purpose::SaveFile });
    }

    fn do_export(&mut self, format: usize, scale: usize, to_library: bool, name: &str) {
        let _ = std::fs::create_dir_all(&self.dir);
        let wrote = |p: &Path| format!("Wrote {} in the sprites folder.", p.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default());
        let result: Result<String, std::io::Error> = match format {
            0 => {
                let p = self.dir.join(format!("{name}.png"));
                export::write_png(&p, &self.sprite, scale).map(|_| wrote(&p))
            }
            1 => {
                let p = self.dir.join(format!("{name}.jpg"));
                export::write_jpg(&p, &self.sprite, scale, 92).map(|_| wrote(&p))
            }
            _ => {
                let pdf = export::pdf_bytes(&self.sprite);
                match (to_library, xochitl::library_dir()) {
                    (true, Some(dir)) => xochitl::add_pdf(&dir, name, &pdf).map(|_| {
                        format!("'{name}' is in the tablet's library. It appears after xochitl restarts: restart now (this closes the editor) or reboot the tablet later.")
                    }),
                    _ => {
                        let p = self.dir.join(format!("{name}.pdf"));
                        std::fs::write(&p, &pdf).map(|_| wrote(&p))
                    }
                }
            }
        };
        match result {
            Ok(msg) => {
                if format == 2 && to_library {
                    self.show(Dialog::message_with("EXPORTED", &msg, &[("LATER", Id::Ok), ("RESTART NOW", Id::Restart)]));
                } else {
                    self.show(Dialog::message("EXPORTED", &msg));
                }
            }
            Err(e) => self.show(Dialog::message("EXPORT FAILED", &format!("{e}"))),
        }
    }

    // ---- painting ----------------------------------------------------------------

    fn build_widgets(&self) -> Vec<Widget> {
        let mut ws = Vec::new();
        // Top bar, row 1: files.
        ws.extend(row(
            10,
            10,
            80,
            10,
            &[(Id::New, "NEW", 110), (Id::Open, "OPEN", 120), (Id::Save, "SAVE", 120), (Id::SaveAs, "SAVE AS", 180), (Id::Export, "EXPORT", 160)],
        ));
        ws.push(Widget::new(Id::Quit, Rect::new(SCREEN_W - 120, 10, 110, 80), "QUIT"));
        ws.push(Widget::new(Id::InkMode, Rect::new(1084, 10, 190, 80), self.mode.label()));
        // Row 2: edit and view, then the selection group while a selection tool is active.
        let mut r2 = row(
            10,
            98,
            80,
            8,
            &[
                (Id::Undo, "UNDO", 90),
                (Id::Redo, "REDO", 90),
                (Id::ZoomOut, "-", 90),
                (Id::ZoomIn, "+", 90),
                (Id::Fit, "FIT", 120),
                (Id::Grid, "GRID", 90),
                (Id::Refresh, "REFRESH", 150),
            ],
        );
        r2[0].enabled = self.undo.can_undo();
        r2[1].enabled = self.undo.can_redo();
        for w in &mut r2[2..5] {
            w.icon = Some(Icon::Zoom);
        }
        r2[5].active = self.view.grid;
        ws.extend(r2);
        // The selection group is shown whenever there is a selection (whatever the tool) or a
        // selection tool is in use, and hidden when neither holds (owner's rule, 2026-10-28).
        let has_sel = self.selection.is_some();
        if self.tool.is_select() || has_sel || !self.poly.is_empty() {
            let mut sel = row(
                790,
                98,
                80,
                8,
                &[
                    (Id::SelCut, "CUT", 80),
                    (Id::SelCopy, "COPY", 100),
                    (Id::SelPaste, "PASTE", 110),
                    (Id::SelClear, "CLEAR", 110),
                    (Id::SelNone, "DESELECT", 170),
                ],
            );
            for (i, w) in sel.iter_mut().enumerate() {
                w.enabled = match i {
                    2 => self.clip.is_some(),
                    4 => has_sel || !self.poly.is_empty(), // DESELECT also drops a polygon in progress
                    _ => has_sel,
                };
            }
            ws.extend(sel);
        }
        // Tool column (geometry in ui.rs): single tools, then the SHAPES and SELECTION grids.
        for (i, t) in Tool::SINGLE.iter().enumerate() {
            let r = Rect::new(TOOL_X, TOOLBAR.y + TOOL_Y + i as i32 * TOOL_PITCH, TOOL_W, TOOL_H);
            ws.push(Widget::new(Id::Tool(*t), r, t.label()).active(self.tool == *t).icon(Icon::Tool(*t), Layout::Lead));
        }
        for (group, top) in [(&Tool::SHAPES, SHAPES_Y), (&Tool::SELECTIONS, SELECTION_Y)] {
            for (i, t) in group.iter().enumerate() {
                let (col, rw) = (i as i32 % 2, i as i32 / 2);
                let r = Rect::new(CELL_X + col * CELL_PITCH_X, TOOLBAR.y + top + rw * CELL_PITCH_Y, CELL_W, CELL_H);
                ws.push(Widget::new(Id::Tool(*t), r, t.label()).active(self.tool == *t).icon(Icon::Tool(*t), Layout::Stacked));
            }
        }
        for (i, n) in [1, 2, 3, 4].iter().enumerate() {
            let r = Rect::new(TOOL_X, TOOLBAR.y + BRUSH_Y + i as i32 * BRUSH_PITCH, TOOL_W, BRUSH_H);
            ws.push(Widget::new(Id::Brush(*n), r, &format!("{n}")).active(self.brush == *n));
        }
        let pp = Rect::new(TOOL_X, TOOLBAR.y + PIXEL_PERFECT_Y, TOOL_W, PIXEL_PERFECT_H);
        ws.push(Widget::new(Id::PixelPerfect, pp, if self.pixel_perfect { "ON" } else { "OFF" }).active(self.pixel_perfect));
        if self.tool == Tool::Wand {
            let c = Rect::new(TOOL_X, TOOLBAR.y + CONTIGUOUS_Y, TOOL_W, CONTIGUOUS_H);
            ws.push(Widget::new(Id::Contiguous, c, if self.contiguous { "ON" } else { "OFF" }).active(self.contiguous));
        }
        // Palette panel.
        for i in 0..PALETTE_SIZE {
            let (col, rw) = (i % 8, i / 8);
            let r = Rect::new(PALETTE_PANEL.x + 16 + col as i32 * 92, PALETTE_PANEL.y + 44 + rw as i32 * 92, 84, 84);
            ws.push(Widget::new(Id::Color(i as u8), r, "").active(self.color as usize == i).custom());
        }
        let view_label = match self.view.mode {
            ViewMode::Index => "VIEW:IDX",
            ViewMode::Luma => "VIEW:LUMA",
        };
        ws.push(Widget::new(Id::View, Rect::new(PALETTE_PANEL.x + 16, PALETTE_PANEL.y + 236, 180, 80), view_label));
        ws.push(Widget::new(Id::EditColor, Rect::new(PALETTE_PANEL.x + 204, PALETTE_PANEL.y + 236, 230, 80), "EDIT COLOR"));
        // Layer panel.
        let lp = LAYER_PANEL;
        ws.extend(row(
            lp.x + 8,
            lp.y + LAYER_BUTTONS_Y,
            64,
            8,
            &[(Id::LayerAdd, "ADD", 110), (Id::LayerDel, "DEL", 110), (Id::LayerUp, "UP", 100), (Id::LayerDown, "DOWN", 120), (Id::LayerRename, "NAME", 120)],
        ));
        let n = self.sprite.layers.len();
        for r in 0..LAYER_ROWS {
            let Some(li) = (n - 1).checked_sub(self.layer_scroll + r) else { break };
            let y = lp.y + LAYER_ROWS_Y + r as i32 * LAYER_ROW_PITCH;
            let l = &self.sprite.layers[li];
            let shown = self.sprite.effective_visible(li);
            let eye = if l.visible { Icon::EyeOpen } else { Icon::EyeClosed };
            ws.push(Widget::new(Id::LayerVis(li), Rect::new(lp.x + LAYER_VIS_X, y, LAYER_ICON_W, LAYER_ROW_H), "").custom().icon(eye, Layout::Center));
            // A hidden layer is locked whatever its flag says, and its lock cannot be tapped.
            let lock = if l.locked || !shown { Icon::LockClosed } else { Icon::LockOpen };
            ws.push(
                Widget::new(Id::LayerLock(li), Rect::new(lp.x + LAYER_LOCK_X, y, LAYER_ICON_W, LAYER_ROW_H), "")
                    .custom()
                    .icon(lock, Layout::Center)
                    .enabled(shown),
            );
            let name_w = lp.w - LAYER_NAME_X - 70;
            ws.push(Widget::new(Id::Layer(li), Rect::new(lp.x + LAYER_NAME_X, y, name_w, LAYER_ROW_H), "").active(self.layer == li).custom());
        }
        let list_h = LAYER_ROWS as i32 * LAYER_ROW_PITCH;
        let btn_h = (list_h - 8) / 2;
        ws.push(Widget::new(Id::LayerScrollUp, Rect::new(lp.right() - 66, lp.y + LAYER_ROWS_Y, 58, btn_h), "^").enabled(self.layer_scroll > 0));
        ws.push(
            Widget::new(Id::LayerScrollDown, Rect::new(lp.right() - 66, lp.y + LAYER_ROWS_Y + btn_h + 8, 58, btn_h), "V")
                .enabled(self.layer_scroll + LAYER_ROWS < n),
        );
        ws
    }

    /// Paint the dirty rectangles into the back buffer and send them to the panel. The widget
    /// list is the one `sync_widgets` built.
    fn render(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        let rects = merge_dirty(std::mem::take(&mut self.dirty));
        let any = rects.iter().fold(Rect::default(), |u, r| u.union(r));
        let (w, h) = (self.b.width(), self.b.height());
        let flashing = self.flash.as_ref().filter(|f| f.on).map(|f| f.id);
        {
            let App { fb, sprite, view, preview, layer, widgets, dialog, selection, sel_preview, drag, poly, title_shown, .. } = self;
            let mut cv = Canvas::new(fb, w, h);
            if !any.intersect(&TOPBAR).is_empty() {
                draw_topbar(&mut cv, widgets, title_shown);
            }
            if !any.intersect(&TOOLBAR).is_empty() {
                draw_toolbar(&mut cv, widgets, self.tool == Tool::Wand);
            }
            if !any.intersect(&PALETTE_PANEL).is_empty() {
                draw_palette(&mut cv, widgets, sprite, view.mode, self.color, &self.status);
            }
            if !any.intersect(&LAYER_PANEL).is_empty() {
                draw_layers(&mut cv, widgets, sprite, flashing);
            }
            let offset = match *drag {
                Drag::Move { start, cur, .. } => (cur.0 - start.0, cur.1 - start.1),
                _ => (0, 0),
            };
            let marks = Marks { selection: selection.as_ref(), offset, preview: sel_preview.as_ref(), start: poly.first().copied() };
            for r in &rects {
                if !r.intersect(&CANVAS).is_empty() {
                    view.render(&mut cv, sprite, preview.as_ref().map(|im| (*layer, im)), r, marks);
                }
            }
            if let Some(d) = dialog {
                d.render(&mut cv, sprite);
            }
        }
        for r in &rects {
            let r = r.intersect(&SCREEN);
            if r.is_empty() {
                continue;
            }
            copy_rect(self.b.fb(), &self.fb, w, h, &r);
            if self.trace {
                eprintln!("librepixel: update {} {} {}x{}", r.x, r.y, r.w, r.h);
            }
            if let Err(e) = self.b.update_rect(r.x, r.y, r.w, r.h) {
                eprintln!("librepixel: update: {e}");
            }
        }
    }
}

/// Each rectangle sent to the panel is one e-ink update, so send few and keep them tight.
const MAX_UPDATE_RECTS: usize = 6;

/// Reduce the dirty list to the rectangles the panel will be asked to refresh. Everything
/// inside the canvas (a stroke arrives as many small, contiguous segments) becomes one
/// rectangle; anything covered by another rectangle is dropped; what is left is merged
/// pairwise by least added area, so a top-bar rectangle never gets glued to the canvas just
/// because both happened to be dirty in the same frame.
fn merge_dirty(rects: Vec<Rect>) -> Vec<Rect> {
    let area = |r: &Rect| r.w as i64 * r.h as i64;
    let mut canvas = Rect::default();
    let mut rest: Vec<Rect> = Vec::new();
    for r in rects.iter().filter(|r| !r.is_empty()) {
        if r.intersect(&CANVAS) == *r {
            canvas = canvas.union(r);
        } else {
            rest.push(*r);
        }
    }
    if !canvas.is_empty() {
        rest.push(canvas);
    }
    let mut out: Vec<Rect> = Vec::new();
    for (i, r) in rest.iter().enumerate() {
        let covered = rest.iter().enumerate().any(|(j, o)| j != i && o.intersect(r) == *r && (o != r || j < i));
        if !covered {
            out.push(*r);
        }
    }
    while out.len() > MAX_UPDATE_RECTS {
        let (mut bi, mut bj, mut best) = (0, 1, i64::MAX);
        for i in 0..out.len() {
            for j in i + 1..out.len() {
                let added = area(&out[i].union(&out[j])) - area(&out[i]) - area(&out[j]);
                if added < best {
                    (bi, bj, best) = (i, j, added);
                }
            }
        }
        let b = out.remove(bj);
        out[bi] = out[bi].union(&b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_segments_become_one_canvas_rect_beside_the_top_bar() {
        let mut v: Vec<Rect> = (0..40).map(|i| Rect::new(CANVAS.x + 100 + i * 10, CANVAS.y + 200 + i * 7, 20, 20)).collect();
        v.push(TOPBAR);
        v.push(TOPBAR);
        let m = merge_dirty(v);
        assert_eq!(m.len(), 2);
        assert!(m.contains(&TOPBAR));
        let c = m.iter().find(|r| **r != TOPBAR).unwrap();
        assert_eq!(c.intersect(&CANVAS), *c, "the stroke rect stays inside the canvas");
        assert_eq!((c.x, c.y, c.right(), c.bottom()), (CANVAS.x + 100, CANVAS.y + 200, CANVAS.x + 510, CANVAS.y + 493));
    }

    #[test]
    fn covered_rects_drop_and_the_rest_merge_by_least_added_area() {
        assert_eq!(merge_dirty(vec![SCREEN, CANVAS, TOPBAR, Rect::default(), SCREEN]), vec![SCREEN]);
        let strips: Vec<Rect> = (0..10).map(|i| Rect::new(0, i * 180, 100, 50)).collect();
        let m = merge_dirty(strips.clone());
        assert!(m.len() <= MAX_UPDATE_RECTS);
        for s in &strips {
            assert!(m.iter().any(|r| r.intersect(s) == *s), "every strip is still covered");
        }
        // Two far-apart panels stay separate rather than being glued into one tall rectangle.
        let m = merge_dirty(vec![TOPBAR, PALETTE_PANEL, Rect::new(CANVAS.x + 5, CANVAS.y + 5, 10, 10)]);
        assert_eq!(m.len(), 3);
    }
}

/// Painting never leaves the selection: inside `r`, put `base` back wherever the mask is off.
fn clip_to_selection(img: &mut Image, base: &Image, r: &Rect, sel: Option<&Selection>) {
    let Some(sel) = sel else { return };
    if sel.is_empty() || base.px.len() != img.px.len() {
        return;
    }
    let r = r.intersect(&Rect::new(0, 0, img.w as i32, img.h as i32));
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            if !sel.contains(x, y) {
                let k = y as usize * img.w + x as usize;
                img.px[k] = base.px[k];
            }
        }
    }
}

fn apply_shape(img: &mut Image, tool: Tool, a: (i32, i32), b: (i32, i32), brush: i32, v: u8) -> Rect {
    match tool {
        Tool::Line => raster::line(img, a.0, a.1, b.0, b.1, brush, v),
        Tool::Rect => raster::rect_outline(img, a.0, a.1, b.0, b.1, brush, v),
        Tool::RectFill => raster::rect_filled(img, a.0, a.1, b.0, b.1, v),
        Tool::Ellipse => raster::ellipse(img, a.0, a.1, b.0, b.1, brush, v, false),
        Tool::EllipseFill => raster::ellipse(img, a.0, a.1, b.0, b.1, brush, v, true),
        _ => Rect::default(),
    }
}

fn draw_topbar(cv: &mut Canvas, widgets: &[Widget], title: &(String, String)) {
    cv.fill(&TOPBAR, BG);
    cv.fill(&Rect::new(0, TOPBAR.bottom() - 3, SCREEN_W, 3), BLACK);
    cv.text(TITLE_RECT.x, 18, TEXT, BLACK, &title.0);
    cv.text(TITLE_RECT.x, 54, SMALL, BLACK, &title.1);
    draw_buttons(cv, &widgets.iter().filter(|w| TOPBAR.contains(w.rect.x, w.rect.y)).cloned().collect::<Vec<_>>());
}

fn draw_toolbar(cv: &mut Canvas, widgets: &[Widget], wand: bool) {
    cv.fill(&TOOLBAR, BG);
    cv.fill(&Rect::new(TOOLBAR.right() - 3, TOOLBAR.y, 3, TOOLBAR.h), BLACK);
    cv.text(TOOLBAR.x + 12, TOOLBAR.y + SHAPES_LABEL_Y, SMALL, BLACK, "SHAPES");
    cv.text(TOOLBAR.x + 12, TOOLBAR.y + SELECTION_LABEL_Y, SMALL, BLACK, "SELECTION");
    cv.text(TOOLBAR.x + 12, TOOLBAR.y + BRUSH_LABEL_Y, SMALL, BLACK, "BRUSH SIZE");
    cv.text(TOOLBAR.x + 12, TOOLBAR.y + PIXEL_PERFECT_LABEL_Y, SMALL, BLACK, "PIX PERFECT");
    if wand {
        cv.text(TOOLBAR.x + 12, TOOLBAR.y + CONTIGUOUS_LABEL_Y, SMALL, BLACK, "CONTIGUOUS");
    }
    draw_buttons(cv, &widgets.iter().filter(|w| TOOLBAR.contains(w.rect.x, w.rect.y)).cloned().collect::<Vec<_>>());
}

fn draw_palette(cv: &mut Canvas, widgets: &[Widget], sprite: &Sprite, mode: ViewMode, color: u8, status: &str) {
    cv.fill(&PALETTE_PANEL, BG);
    cv.fill(&Rect::new(0, PALETTE_PANEL.y, SCREEN_W, 3), BLACK);
    cv.fill(&Rect::new(PALETTE_PANEL.right() - 3, PALETTE_PANEL.y, 3, PALETTE_PANEL.h), BLACK);
    cv.text(PALETTE_PANEL.x + 16, PALETTE_PANEL.y + 8, TITLE, BLACK, "PALETTE");
    let greys = sprite.display_palette(mode);
    for w in widgets.iter().filter(|w| matches!(w.id, Id::Color(_))) {
        let Id::Color(i) = w.id else { continue };
        let g = greys[i as usize].r;
        cv.fill(&w.rect, grey565(g));
        cv.frame(&w.rect, 2, BLACK);
        if w.active {
            cv.frame(&w.rect, 8, BLACK);
            cv.frame(&w.rect.inset(8), 3, WHITE);
        }
        let ink = if g > 128 { BLACK } else { WHITE };
        cv.text(w.rect.x + 6, w.rect.y + 6, SMALL, ink, &format!("{i}"));
    }
    let c = sprite.palette[color as usize];
    let label = format!("COLOR {color}: R{} G{} B{}", c.r, c.g, c.b);
    cv.text(PALETTE_PANEL.x + 450, PALETTE_PANEL.y + 236 + (80 - Canvas::text_height(SMALL)) / 2, SMALL, BLACK, &label);
    // The status line: the editor's last message, else the paper hint while slot 0 is chosen.
    let line = if !status.is_empty() {
        status
    } else if color == sprite.transparent {
        "0 IS THE PAPER: TRANSPARENT ON UPPER LAYERS"
    } else {
        ""
    };
    if let Some(l) = wrap(line, 60).first() {
        cv.text(PALETTE_PANEL.x + 16, PALETTE_PANEL.y + 340, SMALL, BLACK, l);
    }
    draw_buttons(cv, &widgets.iter().filter(|w| PALETTE_PANEL.contains(w.rect.x, w.rect.y)).cloned().collect::<Vec<_>>());
}

/// `flashing` is the eye or lock drawn inverted right now (a refused edit blinks it).
fn draw_layers(cv: &mut Canvas, widgets: &[Widget], sprite: &Sprite, flashing: Option<Id>) {
    let lp = LAYER_PANEL;
    cv.fill(&lp, BG);
    cv.fill(&Rect::new(0, lp.y, SCREEN_W, 3), BLACK);
    cv.text(lp.x + 8, lp.y + 8, TITLE, BLACK, "LAYERS");
    let head = |cv: &mut Canvas, x: i32, w: i32, s: &str| {
        let tx = x + (w - Canvas::text_width(SMALL, s)) / 2;
        cv.text(tx.max(x), lp.y + LAYER_HEAD_Y, SMALL, BLACK, s);
    };
    head(cv, lp.x + LAYER_VIS_X, LAYER_ICON_W, "SHOW");
    head(cv, lp.x + LAYER_LOCK_X, LAYER_ICON_W, "LOCK");
    cv.text(lp.x + LAYER_NAME_X + 12, lp.y + LAYER_HEAD_Y, SMALL, BLACK, "LAYER NAME");
    for w in widgets.iter().filter(|w| lp.contains(w.rect.x, w.rect.y)) {
        match w.id {
            Id::LayerVis(_) | Id::LayerLock(_) => {
                let (fill, ink, edge) = if flashing == Some(w.id) {
                    (BLACK, WHITE, BLACK)
                } else if !w.enabled {
                    (BG, DISABLED, DISABLED)
                } else {
                    (WHITE, BLACK, BLACK)
                };
                cv.fill(&w.rect, fill);
                cv.frame(&w.rect, 3, edge);
                if let Some(icon) = w.icon {
                    icon.draw_in(cv, &w.rect, ink);
                }
            }
            Id::Layer(i) => {
                let l = &sprite.layers[i];
                let (fill, ink) = if w.active { (BLACK, WHITE) } else { (WHITE, BLACK) };
                cv.fill(&w.rect, fill);
                cv.frame(&w.rect, 2, BLACK);
                let prefix = if l.kind == LayerKind::Group { "+ " } else { "" };
                let text = format!("{}{}{}", " ".repeat(l.child_level as usize * 2), prefix, l.name);
                cv.text_left(&w.rect, 12, TEXT, ink, &text);
                if !sprite.effective_visible(i) && l.visible {
                    cv.text(w.rect.right() - 90, w.rect.y + 16, SMALL, ink, "HIDDEN");
                }
            }
            _ => {
                if w.plain {
                    draw_button(cv, w);
                }
            }
        }
    }
}
