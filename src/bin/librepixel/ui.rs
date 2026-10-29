//! Screen layout, widget ids, icons, and the drawing helpers every panel uses.
//!
//! Fixed portrait layout for the 1404x1872 panel: a two-row top bar, a tool column on the
//! left, the canvas, and a bottom strip with the palette (left) and the layers (right).
//! Everything is sized for fingers (buttons are 70 px and up); the pen only has to hit the canvas.

use librepixel::gfx::{grey16, Canvas, Rect, BLACK, WHITE};

pub const SCREEN_W: i32 = 1404;
pub const SCREEN_H: i32 = 1872;
pub const TOP_H: i32 = 184;
pub const LEFT_W: i32 = 168;
pub const BOTTOM_H: i32 = 400;

// Tool column geometry, relative to TOOLBAR.y. tools/sim_smoke.py mirrors these numbers.
// Five single tools (PEN ERASE LINE FILL PICK), then the SHAPES and SELECTION 2x2 grids.
pub const TOOL_X: i32 = 8;
pub const TOOL_W: i32 = 152;
pub const TOOL_Y: i32 = 6;
pub const TOOL_PITCH: i32 = 76;
pub const TOOL_H: i32 = 70;
pub const SHAPES_LABEL_Y: i32 = 392;
pub const SHAPES_Y: i32 = 412;
pub const SELECTION_LABEL_Y: i32 = 588;
pub const SELECTION_Y: i32 = 608;
/// Grid cells: two per row at x CELL_X and CELL_X + CELL_PITCH_X; 80 wide so "FILLED" fits.
pub const CELL_X: i32 = 3;
pub const CELL_W: i32 = 80;
pub const CELL_H: i32 = 80;
pub const CELL_PITCH_X: i32 = 82;
pub const CELL_PITCH_Y: i32 = 84;
pub const BRUSH_LABEL_Y: i32 = 786;
pub const BRUSH_Y: i32 = 806;
pub const BRUSH_PITCH: i32 = 54;
pub const BRUSH_H: i32 = 48;
pub const PIXEL_PERFECT_LABEL_Y: i32 = 1032;
pub const PIXEL_PERFECT_Y: i32 = 1050;
pub const PIXEL_PERFECT_H: i32 = 48;
/// Only while WAND is the tool.
pub const CONTIGUOUS_LABEL_Y: i32 = 1108;
pub const CONTIGUOUS_Y: i32 = 1126;
pub const CONTIGUOUS_H: i32 = 48;

// Layer panel geometry, relative to LAYER_PANEL.y / .x.
pub const LAYER_BUTTONS_Y: i32 = 44;
/// SHOW / LOCK / LAYER NAME headings.
pub const LAYER_HEAD_Y: i32 = 112;
pub const LAYER_ROWS_Y: i32 = 130;
pub const LAYER_ROW_PITCH: i32 = 50;
pub const LAYER_ROW_H: i32 = 46;
pub const LAYER_VIS_X: i32 = 8;
pub const LAYER_LOCK_X: i32 = 64;
pub const LAYER_NAME_X: i32 = 120;
pub const LAYER_ICON_W: i32 = 48;

pub const SCREEN: Rect = Rect::new(0, 0, SCREEN_W, SCREEN_H);
pub const TOPBAR: Rect = Rect::new(0, 0, SCREEN_W, TOP_H);
/// The file name (with its modified mark) and the size/zoom line; the INK button starts at 1084.
pub const TITLE_RECT: Rect = Rect::new(760, 10, 320, 80);
pub const TOOLBAR: Rect = Rect::new(0, TOP_H, LEFT_W, SCREEN_H - TOP_H - BOTTOM_H);
pub const CANVAS: Rect = Rect::new(LEFT_W, TOP_H, SCREEN_W - LEFT_W, SCREEN_H - TOP_H - BOTTOM_H);
pub const PALETTE_PANEL: Rect = Rect::new(0, SCREEN_H - BOTTOM_H, 760, BOTTOM_H);
pub const LAYER_PANEL: Rect = Rect::new(760, SCREEN_H - BOTTOM_H, SCREEN_W - 760, BOTTOM_H);

pub const BG: u16 = grey16(14);
pub const CANVAS_BG: u16 = grey16(12);
pub const DISABLED: u16 = grey16(9);
// Text scales are in half pixels (`Canvas::text`): 5 = 2.5x = 12x18 px glyphs.
pub const TEXT: i32 = 5;
pub const SMALL: i32 = 4;
pub const TITLE: i32 = 7;

/// A 16x16 picture, drawn at 2x (32 px) in buttons and boxes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Zoom,
    Tool(Tool),
    EyeOpen,
    EyeClosed,
    LockClosed,
    LockOpen,
}

pub const ICON_PX: i32 = 32;

/// Where a button's icon goes relative to its label.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Layout {
    /// Icon and label side by side, centred together (the zoom buttons).
    #[default]
    Center,
    /// Icon at the left edge, label after it (the single tools).
    Lead,
    /// Icon above the label; the label wraps at spaces, in the small text (grid cells).
    Stacked,
}

// Bit 15 is the leftmost column.
const ZOOM_ICON: [u16; 16] = [
    0b0000_0111_1000_0000,
    0b0001_1111_1110_0000,
    0b0011_1000_0111_0000,
    0b0110_0000_0001_1000,
    0b0110_0000_0001_1000,
    0b1100_0000_0000_1100,
    0b1100_0000_0000_1100,
    0b1100_0000_0000_1100,
    0b0110_0000_0001_1000,
    0b0110_0000_0001_1000,
    0b0011_1000_0111_1100,
    0b0001_1111_1111_1110,
    0b0000_0111_1001_1111,
    0b0000_0000_0000_1111,
    0b0000_0000_0000_0111,
    0b0000_0000_0000_0011,
];

const PEN_ICON: [u16; 16] = [
    0b0000_0000_0000_0110,
    0b0000_0000_0000_1111,
    0b0000_0000_0001_1111,
    0b0000_0000_0011_1110,
    0b0000_0000_0111_1100,
    0b0000_0000_1111_1000,
    0b0000_0001_1111_0000,
    0b0000_0011_1110_0000,
    0b0000_0111_1100_0000,
    0b0000_1111_1000_0000,
    0b0001_1111_0000_0000,
    0b0011_1110_0000_0000,
    0b0011_1100_0000_0000,
    0b0111_1000_0000_0000,
    0b0110_0000_0000_0000,
    0b0100_0000_0000_0000,
];

const ERASE_ICON: [u16; 16] = [
    0b0000_0000_0000_0000,
    0b0000_0000_0011_1100,
    0b0000_0000_0110_0110,
    0b0000_0000_1100_0011,
    0b0000_0001_1000_0011,
    0b0000_0011_0000_0110,
    0b0000_0110_0000_1100,
    0b0000_1100_0001_1000,
    0b0001_1111_1111_0000,
    0b0011_1111_1110_0000,
    0b0111_1111_1100_0000,
    0b0111_1111_1000_0000,
    0b0111_1111_0000_0000,
    0b0011_1110_0000_0000,
    0b0001_1100_0000_0000,
    0b0000_0000_0000_0000,
];

const LINE_ICON: [u16; 16] = [
    0b0000_0000_0000_0011,
    0b0000_0000_0000_0111,
    0b0000_0000_0000_1110,
    0b0000_0000_0001_1100,
    0b0000_0000_0011_1000,
    0b0000_0000_0111_0000,
    0b0000_0000_1110_0000,
    0b0000_0001_1100_0000,
    0b0000_0011_1000_0000,
    0b0000_0111_0000_0000,
    0b0000_1110_0000_0000,
    0b0001_1100_0000_0000,
    0b0011_1000_0000_0000,
    0b0111_0000_0000_0000,
    0b1110_0000_0000_0000,
    0b1100_0000_0000_0000,
];

const BOX_ICON: [u16; 16] = [
    0b0000_0000_0000_0000,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0000_0000_0000_0000,
];

const BOX_FILL_ICON: [u16; 16] = [
    0b0000_0000_0000_0000,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0000_0000_0000_0000,
];

const ELLIPSE_ICON: [u16; 16] = [
    0b0000_0011_1100_0000,
    0b0000_1111_1111_0000,
    0b0001_1100_0011_1000,
    0b0011_0000_0000_1100,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b1100_0000_0000_0011,
    0b1100_0000_0000_0011,
    0b1100_0000_0000_0011,
    0b1100_0000_0000_0011,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0011_0000_0000_1100,
    0b0001_1100_0011_1000,
    0b0000_1111_1111_0000,
    0b0000_0011_1100_0000,
];

const ELLIPSE_FILL_ICON: [u16; 16] = [
    0b0000_0011_1100_0000,
    0b0000_1111_1111_0000,
    0b0001_1111_1111_1000,
    0b0011_1111_1111_1100,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b1111_1111_1111_1111,
    0b1111_1111_1111_1111,
    0b1111_1111_1111_1111,
    0b1111_1111_1111_1111,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0011_1111_1111_1100,
    0b0001_1111_1111_1000,
    0b0000_1111_1111_0000,
    0b0000_0011_1100_0000,
];

/// A bucket, straight-sided, handle up, half full of paint.
const FILL_ICON: [u16; 16] = [
    0b0000_0011_1100_0000,
    0b0000_0110_0110_0000,
    0b0000_1100_0011_0000,
    0b0001_1000_0001_1000,
    0b1111_1111_1111_1111,
    0b1111_1111_1111_1111,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0110_0000_0000_0110,
    0b0111_1111_1111_1110,
    0b0111_1111_1111_1110,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0000_0000_0000_0000,
];

/// An eyedropper, upright: rubber bulb, collar, glass tube, tip, and a drop below.
const PICK_ICON: [u16; 16] = [
    0b0000_0111_1110_0000,
    0b0000_1111_1111_0000,
    0b0000_1111_1111_0000,
    0b0000_1111_1111_0000,
    0b0000_0111_1110_0000,
    0b0000_0011_1100_0000,
    0b0001_1111_1111_1000,
    0b0001_1111_1111_1000,
    0b0000_0110_0110_0000,
    0b0000_0110_0110_0000,
    0b0000_0110_0110_0000,
    0b0000_0110_0110_0000,
    0b0000_0011_1100_0000,
    0b0000_0001_1000_0000,
    0b0000_0000_0000_0000,
    0b0000_0001_1000_0000,
];

/// A dashed rectangle.
const RECT_SEL_ICON: [u16; 16] = [
    0b1110_1110_1110_1111,
    0b1000_0000_0000_0001,
    0b1000_0000_0000_0001,
    0b0000_0000_0000_0000,
    0b1000_0000_0000_0001,
    0b1000_0000_0000_0001,
    0b1000_0000_0000_0001,
    0b0000_0000_0000_0000,
    0b1000_0000_0000_0001,
    0b1000_0000_0000_0001,
    0b1000_0000_0000_0001,
    0b0000_0000_0000_0000,
    0b1000_0000_0000_0001,
    0b1000_0000_0000_0001,
    0b1000_0000_0000_0001,
    0b1110_1110_1110_1111,
];

/// A rope loop with its tail.
const LASSO_ICON: [u16; 16] = [
    0b0000_0111_1110_0000,
    0b0001_1100_0011_1000,
    0b0011_0000_0000_1100,
    0b0110_0000_0000_0110,
    0b0100_0000_0000_0010,
    0b0100_0000_0000_0010,
    0b0110_0000_0000_0110,
    0b0011_0000_0000_1100,
    0b0001_1100_0011_1000,
    0b0000_0111_0110_0000,
    0b0000_0000_1000_0000,
    0b0000_0000_1100_0000,
    0b0000_0000_0110_0000,
    0b0000_0000_0011_0000,
    0b0000_0000_0001_1000,
    0b0000_0000_0000_1100,
];

/// A star on a stick.
const WAND_ICON: [u16; 16] = [
    0b0000_0000_0001_0000,
    0b0000_0000_0101_0100,
    0b0000_0000_0011_1000,
    0b0000_0000_1111_1110,
    0b0000_0000_0011_1000,
    0b0000_0000_0101_0100,
    0b0000_0000_1101_0000,
    0b0000_0001_1000_0000,
    0b0000_0011_0000_0000,
    0b0000_0110_0000_0000,
    0b0000_1100_0000_0000,
    0b0001_1000_0000_0000,
    0b0011_0000_0000_0000,
    0b0110_0000_0000_0000,
    0b1100_0000_0000_0000,
    0b0000_0000_0000_0000,
];

/// A dashed pentagon.
const POLY_SEL_ICON: [u16; 16] = [
    0b0000_0001_1000_0000,
    0b0000_0010_0100_0000,
    0b0000_0000_0000_0000,
    0b0001_1000_0001_1000,
    0b0010_0000_0000_0100,
    0b0000_0000_0000_0000,
    0b1000_0000_0000_0001,
    0b1000_0000_0000_0001,
    0b0000_0000_0000_0000,
    0b0100_0000_0000_0010,
    0b0100_0000_0000_0010,
    0b0000_0000_0000_0000,
    0b0010_0000_0000_0100,
    0b0010_0000_0000_0100,
    0b0000_0000_0000_0000,
    0b0001_1011_0110_1000,
];

const EYE_OPEN_ICON: [u16; 16] = [
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
    0b0000_0011_1100_0000,
    0b0001_1111_1111_1000,
    0b0011_1000_0001_1100,
    0b0110_0011_1100_0110,
    0b1100_0111_1110_0011,
    0b1100_0111_1110_0011,
    0b1100_0111_1110_0011,
    0b0110_0011_1100_0110,
    0b0011_1000_0001_1100,
    0b0001_1111_1111_1000,
    0b0000_0011_1100_0000,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
];

/// A closed lid with three lashes.
const EYE_CLOSED_ICON: [u16; 16] = [
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
    0b1100_0000_0000_0011,
    0b0110_0000_0000_0110,
    0b0011_1000_0001_1100,
    0b0001_1111_1111_1000,
    0b0000_0011_1100_0000,
    0b0000_1001_1001_0000,
    0b0001_0001_1000_1000,
    0b0010_0001_1000_0100,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
];

const LOCK_CLOSED_ICON: [u16; 16] = [
    0b0000_0000_0000_0000,
    0b0000_0011_1100_0000,
    0b0000_0110_0110_0000,
    0b0000_1100_0011_0000,
    0b0000_1100_0011_0000,
    0b0000_1100_0011_0000,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0011_1110_0111_1100,
    0b0011_1110_0111_1100,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
];

/// The shackle lifted off to the right.
const LOCK_OPEN_ICON: [u16; 16] = [
    0b0000_0000_0111_1000,
    0b0000_0000_1100_1100,
    0b0000_0001_1000_0110,
    0b0000_0001_1000_0110,
    0b0000_0001_1000_0000,
    0b0000_0001_1000_0000,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0011_1110_0111_1100,
    0b0011_1110_0111_1100,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0011_1111_1111_1100,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000,
];

impl Icon {
    pub fn bitmap(self) -> &'static [u16; 16] {
        match self {
            Icon::Zoom => &ZOOM_ICON,
            Icon::Tool(Tool::Pencil) => &PEN_ICON,
            Icon::Tool(Tool::Eraser) => &ERASE_ICON,
            Icon::Tool(Tool::Line) => &LINE_ICON,
            Icon::Tool(Tool::Rect) => &BOX_ICON,
            Icon::Tool(Tool::RectFill) => &BOX_FILL_ICON,
            Icon::Tool(Tool::Ellipse) => &ELLIPSE_ICON,
            Icon::Tool(Tool::EllipseFill) => &ELLIPSE_FILL_ICON,
            Icon::Tool(Tool::Fill) => &FILL_ICON,
            Icon::Tool(Tool::Pick) => &PICK_ICON,
            Icon::Tool(Tool::Select) => &RECT_SEL_ICON,
            Icon::Tool(Tool::Lasso) => &LASSO_ICON,
            Icon::Tool(Tool::Wand) => &WAND_ICON,
            Icon::Tool(Tool::Poly) => &POLY_SEL_ICON,
            Icon::EyeOpen => &EYE_OPEN_ICON,
            Icon::EyeClosed => &EYE_CLOSED_ICON,
            Icon::LockClosed => &LOCK_CLOSED_ICON,
            Icon::LockOpen => &LOCK_OPEN_ICON,
        }
    }

    /// Drawn at 2x, centred in `r`.
    pub fn draw_in(self, cv: &mut Canvas, r: &Rect, color: u16) {
        cv.sprite16(r.x + (r.w - ICON_PX) / 2, r.y + (r.h - ICON_PX) / 2, 2, self.bitmap(), color);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pencil,
    Eraser,
    Line,
    Rect,
    RectFill,
    Ellipse,
    EllipseFill,
    Fill,
    Pick,
    Select,
    Lasso,
    Wand,
    Poly,
}

impl Tool {
    /// The tool column: single buttons, then the two 2x2 grids (row by row).
    pub const SINGLE: [Tool; 5] = [Tool::Pencil, Tool::Eraser, Tool::Line, Tool::Fill, Tool::Pick];
    pub const SHAPES: [Tool; 4] = [Tool::Rect, Tool::RectFill, Tool::Ellipse, Tool::EllipseFill];
    pub const SELECTIONS: [Tool; 4] = [Tool::Select, Tool::Lasso, Tool::Poly, Tool::Wand];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Pencil => "PEN",
            Tool::Eraser => "ERASE",
            Tool::Line => "LINE",
            Tool::Rect => "BOX",
            Tool::RectFill => "BOX FILLED",
            Tool::Ellipse => "ELLIP",
            Tool::EllipseFill => "ELLIP FILLED",
            Tool::Fill => "FILL",
            Tool::Pick => "PICK",
            Tool::Select => "RECT SEL",
            Tool::Lasso => "LASSO",
            Tool::Wand => "WAND",
            Tool::Poly => "POLY SEL",
        }
    }

    /// Tools that drag out a shape between two points.
    pub fn is_shape(self) -> bool {
        matches!(self, Tool::Line | Tool::Rect | Tool::RectFill | Tool::Ellipse | Tool::EllipseFill)
    }

    /// Tools that make a selection instead of painting.
    pub fn is_select(self) -> bool {
        matches!(self, Tool::Select | Tool::Lasso | Tool::Wand | Tool::Poly)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Id {
    // top bar
    New,
    Open,
    Save,
    SaveAs,
    Export,
    Quit,
    InkMode,
    Undo,
    Redo,
    ZoomOut,
    ZoomIn,
    Fit,
    View,
    Grid,
    Refresh,
    SelCut,
    SelCopy,
    SelPaste,
    SelClear,
    SelNone,
    // tool column
    Tool(Tool),
    Brush(i32),
    PixelPerfect,
    Contiguous,
    // palette panel
    Color(u8),
    EditColor,
    // layer panel
    LayerAdd,
    LayerDel,
    LayerUp,
    LayerDown,
    LayerRename,
    Layer(usize),
    LayerVis(usize),
    LayerLock(usize),
    LayerScrollUp,
    LayerScrollDown,
    // dialogs
    Ok,
    Cancel,
    SaveFirst,
    Discard,
    Restart,
    Key(char),
    Backspace,
    Space,
    Item(usize),
    PageUp,
    PageDown,
    SizeW(i32),
    SizeH(i32),
    Preset(i32),
    Format(usize),
    Scale(usize),
    Slider(usize),
}

/// Everything that decides how a widget looks is in here, so the editor can compare the list
/// it built last time with the one it builds now and repaint exactly the buttons that changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub id: Id,
    pub rect: Rect,
    pub label: String,
    pub active: bool,
    pub enabled: bool,
    /// Drawn by `draw_button`; false for things a panel draws itself (swatches, rows, sliders).
    pub plain: bool,
    pub icon: Option<Icon>,
    pub layout: Layout,
}

impl Widget {
    pub fn new(id: Id, rect: Rect, label: &str) -> Widget {
        Widget { id, rect, label: label.to_string(), active: false, enabled: true, plain: true, icon: None, layout: Layout::Center }
    }
    pub fn active(mut self, a: bool) -> Widget {
        self.active = a;
        self
    }
    pub fn enabled(mut self, e: bool) -> Widget {
        self.enabled = e;
        self
    }
    pub fn custom(mut self) -> Widget {
        self.plain = false;
        self
    }
    pub fn icon(mut self, icon: Icon, layout: Layout) -> Widget {
        self.icon = Some(icon);
        self.layout = layout;
        self
    }
}

pub fn draw_button(cv: &mut Canvas, w: &Widget) {
    let (fill, ink, edge) = if !w.enabled {
        (BG, DISABLED, DISABLED)
    } else if w.active {
        (BLACK, WHITE, BLACK)
    } else {
        (WHITE, BLACK, BLACK)
    };
    cv.fill(&w.rect, fill);
    cv.frame(&w.rect, 3, edge);
    let Some(icon) = w.icon else {
        cv.text_in(&w.rect, TEXT, ink, &w.label);
        return;
    };
    let r = w.rect;
    match w.layout {
        Layout::Center | Layout::Lead => {
            let tw = Canvas::text_width(TEXT, &w.label);
            let x = if w.layout == Layout::Lead { r.x + 10 } else { r.x + (r.w - (ICON_PX + 6 + tw)) / 2 };
            cv.sprite16(x, r.y + (r.h - ICON_PX) / 2, 2, icon.bitmap(), ink);
            cv.text(x + ICON_PX + 6, r.y + (r.h - Canvas::text_height(TEXT)) / 2, TEXT, ink, &w.label);
        }
        Layout::Stacked => {
            cv.sprite16(r.x + (r.w - ICON_PX) / 2, r.y + 7, 2, icon.bitmap(), ink);
            let lines: Vec<&str> = w.label.split(' ').collect();
            let lh = Canvas::text_height(SMALL);
            let top = r.y + 7 + ICON_PX + 4;
            let room = r.bottom() - 4 - top;
            let pitch = lh + 2;
            let mut y = top + (room - (lines.len() as i32 * pitch - 2)).max(0) / 2;
            for line in lines {
                let x = r.x + (r.w - Canvas::text_width(SMALL, line)) / 2;
                cv.text(x, y, SMALL, ink, line);
                y += pitch;
            }
        }
    }
}

pub fn draw_buttons(cv: &mut Canvas, widgets: &[Widget]) {
    for w in widgets.iter().filter(|w| w.plain) {
        draw_button(cv, w);
    }
}

/// The topmost enabled widget under a point.
pub fn hit(widgets: &[Widget], x: i32, y: i32) -> Option<Id> {
    widgets.iter().rev().find(|w| w.enabled && w.rect.contains(x, y)).map(|w| w.id)
}

/// A row of buttons starting at (x, y): (id, label, width) triples, `gap` apart.
pub fn row(x: i32, y: i32, h: i32, gap: i32, items: &[(Id, &str, i32)]) -> Vec<Widget> {
    let mut out = Vec::new();
    let mut cx = x;
    for (id, label, w) in items {
        out.push(Widget::new(*id, Rect::new(cx, y, *w, h), label));
        cx += w + gap;
    }
    out
}

/// Word-wrap plain text to `max_chars` per line.
pub fn wrap(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max_chars {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_an_icon_and_a_place_in_the_column() {
        let all: Vec<Tool> = Tool::SINGLE.iter().chain(&Tool::SHAPES).chain(&Tool::SELECTIONS).copied().collect();
        assert_eq!(all.len(), 13);
        for t in &all {
            assert!(Icon::Tool(*t).bitmap().iter().any(|&r| r != 0), "{t:?} has a blank icon");
            assert_eq!(all.iter().filter(|o| *o == t).count(), 1, "{t:?} appears once");
        }
        // Grid labels wrap at the space, so each word must fit a cell in the small text.
        for t in Tool::SHAPES.iter().chain(&Tool::SELECTIONS) {
            for word in t.label().split(' ') {
                assert!(Canvas::text_width(SMALL, word) <= CELL_W - 6, "{word} is too wide for a cell");
            }
        }
        // The column fits: CONTIGUOUS is the last thing in it.
        assert!(CONTIGUOUS_Y + CONTIGUOUS_H <= TOOLBAR.h);
        assert!(LAYER_ROWS_Y + 5 * LAYER_ROW_PITCH <= BOTTOM_H);
    }
}
