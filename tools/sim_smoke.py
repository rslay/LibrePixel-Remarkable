"""Smoke test of the sprite editor through the simulator's HTTP endpoints.

Start the editor with its files in a scratch directory, then point this script at it:

    RM2_SPRITE_DIR=/tmp/sprites HELLO_SIM_OPEN=0 cargo run --bin librepixel -- --sim &
    python tools/sim_smoke.py /tmp/sprites

Taps are finger press+release, strokes are pen press/move/release (the same qtfb events the
tablet sends); `256 0 0 1` / `256 0 0 0` is the simulator's "eraser end near / tip" report and
`257 0 0 1` / `257 0 0 0` the pen entering / leaving hover range.
Frames come back as raw RGB565; screenshots go next to the sprites as ui-*.png.
Needs numpy and Pillow; pypdf if present validates the PDF. Exits non-zero on the first failure.
"""
import os
import sys
import time
import urllib.request

import numpy as np
from PIL import Image

BASE = os.environ.get("RM2_SIM_URL", "http://127.0.0.1:1404")
W, H = 1404, 1872
SPRITES = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/sprites")

# Geometry, from src/bin/librepixel/ui.rs + app.rs build_widgets() and dialogs.rs.
TOP_H, BOTTOM_H, LEFT_W = 184, 400, 168
PANEL_Y = H - BOTTOM_H                       # 1472: palette + layers start here
CANVAS = (LEFT_W, TOP_H, W - LEFT_W, H - TOP_H - BOTTOM_H)
NEW, OPEN, SAVE, SAVE_AS, EXPORT, QUIT = (65, 50), (190, 50), (320, 50), (480, 50), (660, 50), (1339, 50)
INK = (1179, 50)                             # INK:LIVE / INK:BATCH / INK:NATIVE, x 1084..1274
ARM_FILE = os.environ.get("RM2_INK_ARM")     # set when the editor runs with RM2_INK_FAKE=1
CMD_FILE = os.environ.get("RM2_INK_CMD")     # the extension's command pipe, a plain file here
SETTINGS = os.environ.get("RM2_SETTINGS")
SETTLE = 0.25                                # > TOUCH_SETTLE (150 ms): a finger must rest this long before it may pan
COOLDOWN = 1.3                               # > PEN_COOLDOWN (1200 ms)
UNDO, REDO, ZOOM_OUT, ZOOM_IN = (55, 138), (153, 138), (251, 138), (349, 138)
# The selection group, shown only while a selection tool is active.
CUT, COPY, PASTE, CLEAR, DESELECT = (830, 138), (928, 138), (1041, 138), (1159, 138), (1307, 138)
CLEAR_X, DESELECT_X = 1104, 1222             # their left edges, for the enabled checks
# Tool column: five single buttons, then the SHAPES and SELECTION 2x2 grids (row by row).
TOOL_BOX = {name: (8, TOP_H + 6 + i * 76) for i, name in enumerate(["PEN", "ERASE", "LINE", "FILL", "PICK"])}
GRID_X, GRID_PITCH = 3, 82                                                           # CELL_X, CELL_PITCH_X (cells 80x80)
for top, names in ((412, ["BOX", "BOX FILLED", "ELLIP", "ELLIP FILLED"]), (608, ["RECT SEL", "LASSO", "POLY SEL", "WAND"])):
    for i, name in enumerate(names):
        TOOL_BOX[name] = (GRID_X + (i % 2) * GRID_PITCH, TOP_H + top + (i // 2) * 84)
TOOLS = {name: (x + (76 if x == 8 else 40), y + (35 if x == 8 else 40)) for name, (x, y) in TOOL_BOX.items()}
BRUSH = {n: (80, TOP_H + 806 + (n - 1) * 54 + 24) for n in (1, 2, 3, 4)}           # BRUSH_Y, BRUSH_PITCH
PIX_TOP = TOP_H + 1050                                                               # PIXEL_PERFECT_Y
PIX = (80, PIX_TOP + 24)
CONTIG = (80, TOP_H + 1126 + 24)                                                     # CONTIGUOUS_Y, with WAND
SWATCH = {i: (16 + (i % 8) * 92 + 42, PANEL_Y + 44 + (i // 8) * 92 + 42) for i in range(16)}
EDIT_COLOR = (204 + 115, PANEL_Y + 236 + 40)
LAYER_ADD = (823, PANEL_Y + 44 + 32)
LAYER_ROWS_Y, LAYER_ROW_PITCH = PANEL_Y + 130, 50
VIS_BOX, LOCK_BOX = (760 + 8, LAYER_ROWS_Y), (760 + 64, LAYER_ROWS_Y)            # row 0 boxes, 48x46
LAYER_VIS_ROW0, LAYER_LOCK_ROW0 = (VIS_BOX[0] + 24, VIS_BOX[1] + 23), (LOCK_BOX[0] + 24, LOCK_BOX[1] + 23)
LAYER_ROW1_Y = LAYER_ROWS_Y + LAYER_ROW_PITCH
DIALOG_OK_X, DIALOG_CANCEL_X = 1138, 882  # right-aligned 240 px buttons: OK rightmost
DISCARD = (926, 1062)                     # in the unsaved-changes dialog
seq = 0


def post(body):
    req = urllib.request.Request(BASE + "/input", data=body.encode(), method="POST")
    urllib.request.urlopen(req, timeout=10).read()


def tap(xy, dev=1):
    x, y = xy
    post(f"16 {x} {y} 0 {dev};17 {x} {y} 0 {dev}")
    time.sleep(0.25)


def pen_lines(points, pressure=50, eraser=False):
    (x0, y0) = points[0]
    lines = ["257 0 0 1 1"] + (["256 0 0 1 1"] if eraser else [])
    lines += [f"32 {x0} {y0} {pressure} 1"] + [f"34 {x} {y} {pressure} 1" for (x, y) in points[1:]]
    (xn, yn) = points[-1]
    lines.append(f"33 {xn} {yn} 0 1")
    if eraser:
        lines.append("256 0 0 0 1")
    lines.append("257 0 0 0 1")
    return lines


def pen(points, pressure=50, eraser=False):
    post(";".join(pen_lines(points, pressure, eraser)))
    time.sleep(0.3)


def frame():
    global seq
    r = urllib.request.urlopen(BASE + "/frame?after=0", timeout=30)
    seq = int(r.headers["X-Frame-Seq"])
    px = np.frombuffer(r.read(), dtype="<u2").reshape(H, W)
    return (((px >> 11) & 31) * 255 // 31).astype(np.uint8)


def shot(name):
    g = frame()
    Image.fromarray(g, "L").resize((W // 2, H // 2), Image.LANCZOS).save(os.path.join(SPRITES, f"ui-{name}.png"))
    print(f"[{seq:3}] {name}")
    return g


def dialog_bottom(g):
    """y of the lowest row that is black across a dialog panel's width but not right of it
    (full-width separators are black there too; the tool column on the left may be black
    anywhere, so it is not sampled)."""
    for y in range(1600, 300, -1):
        if (g[y, 170:1284] < 60).all() and g[y, 1314] > 60:
            return y
    raise AssertionError("no dialog on screen")


def dialog_ok(g):
    tap((DIALOG_OK_X, dialog_bottom(g) - 44 - 42))


def check(cond, msg):
    if not cond:
        print("FAIL:", msg)
        sys.exit(1)
    print("ok:", msg)


def dark(g, x, y, w, h):
    return float((g[y : y + h, x : x + w] < 128).mean())


def button_active(g, x_left, y_top):
    """A plain button is black-filled when active: sample just inside its frame."""
    return g[y_top + 10, x_left + 10] < 60


def button_enabled(g, x_left, y_top):
    """Disabled buttons have a mid-grey frame instead of a black one."""
    return g[y_top + 40, x_left + 1] < 60


def button_present(g, x_left, y_top):
    """Any frame at all (black or the disabled grey) where the panel background would be."""
    return g[y_top + 40, x_left + 1] < 200


def box_fill(g, box):
    """Grey just inside a layer-row box's frame: white normally, black while it blinks, the
    panel grey when it is disabled."""
    x, y = box
    return int(g[y + 6, x + 6])


def title(g):
    return g[10:50, 760:1080].copy()


def cell(g, z, px, py, ox=106, oy=132):
    """Mean grey of the inside of sprite pixel (px, py) at zoom z (fit-centred sprite)."""
    x0, y0 = CANVAS[0] + ox + px * z + z // 4, CANVAS[1] + oy + py * z + z // 4
    return float(g[y0 : y0 + z // 2, x0 : x0 + z // 2].mean())


def centre(z, px, py, ox=106, oy=132):
    return (CANVAS[0] + ox + px * z + z // 2, CANVAS[1] + oy + py * z + z // 2)


for _ in range(60):
    try:
        urllib.request.urlopen(BASE + "/", timeout=2).read()
        break
    except Exception:
        time.sleep(0.5)
else:
    sys.exit("simulator not reachable at " + BASE)

g = shot("01-initial")
before = dark(g, *CANVAS)
t0 = title(g)  # "UNTITLED" without the modified mark
for name, (x, y) in TOOL_BOX.items():
    check(button_present(g, x, y), f"tool button {name} is on screen")
check(button_active(g, *TOOL_BOX["PEN"]) and not button_active(g, *TOOL_BOX["BOX"]), "PEN starts active")
check(box_fill(g, VIS_BOX) > 250 and box_fill(g, LOCK_BOX) > 250, "layer row 0 has an eye box and a lock box")
check(not button_present(g, DESELECT_X, 98), "no selection group while PEN is the tool")


def read_text(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


if ARM_FILE and SETTINGS:
    # Native ink (faked by RM2_INK_FAKE): armed at start, off while a dialog is up, and the
    # INK button cycles the mode and records it in settings.json.
    time.sleep(0.3)
    check(read_text(ARM_FILE).startswith("on 274 316 1024 1024 "), f"native ink armed on the sprite rect: {read_text(ARM_FILE)!r}")
    check('"stroke_mode": "native"' in read_text(SETTINGS), "settings.json says native on first run")
    tap(NEW)
    time.sleep(0.3)
    check(read_text(ARM_FILE) == "off\n", "arm file says off while a dialog is open")
    tap((DIALOG_CANCEL_X, 1200))
    time.sleep(0.3)
    check(read_text(ARM_FILE).startswith("on "), "armed again after the dialog closed")
    if CMD_FILE:
        open(CMD_FILE, "w").close()
        pen([(400, 500), (600, 650)])
        lines = read_text(CMD_FILE).splitlines()
        check(len(lines) >= 3 and all(l.startswith("fill ") for l in lines[:-1]) and lines[-1] == "end",
              f"native stroke sent {len(lines) - 1} fill blocks then end to the extension")
        check(all(len(l.split()) == 6 and int(l.split()[3]) % 16 == 0 for l in lines[:-1]), "fills are whole zoom-16 blocks")
    tap(LAYER_LOCK_ROW0)
    time.sleep(0.3)
    check(read_text(ARM_FILE) == "off\n", "a locked layer disarms native ink")
    tap(LAYER_LOCK_ROW0)
    time.sleep(0.3)
    check(read_text(ARM_FILE).startswith("on "), "unlocking arms it again")
    ink_label = g[20:80, 1090:1270].copy()
    tap(INK)
    g = frame()
    check(not np.array_equal(ink_label, g[20:80, 1090:1270]), "INK button changed its label")
    check('"stroke_mode": "live"' in read_text(SETTINGS), "settings.json follows the INK button")
    check(read_text(ARM_FILE) == "off\n", "leaving native mode disarms the extension")
else:
    print("skip: native-ink checks (run the editor with RM2_INK_FAKE=1 RM2_INK_ARM=... RM2_SETTINGS=...)")
pen([(400, 500), (600, 650), (900, 1000)])
g = shot("02-pencil")
after_pencil = dark(g, *CANVAS)
check(after_pencil > before, "pencil stroke darkened the canvas")
check(not np.array_equal(t0, title(g)), "the modified mark appeared with the first stroke")
check(button_enabled(g, 10, 98), "UNDO is enabled right after the first stroke")

# The eraser end of the pen erases without choosing ERASE, then PEN comes back on its own.
post("256 0 0 1 1;32 400 500 50 1;34 600 650 50 1;34 900 1000 50 1")  # eraser end still down
time.sleep(0.3)
g = shot("03-eraser-end")
check(dark(g, *CANVAS) < after_pencil, "eraser end erased the stroke")
check(button_active(g, *TOOL_BOX["ERASE"]), "ERASE was selected while the eraser end was down")
post("33 900 1000 0 1;256 0 0 0 1")
time.sleep(0.4)
g = shot("04-pen-restored")
check(button_active(g, *TOOL_BOX["PEN"]), "PEN came back 150 ms after the eraser end lifted")
pen([(400, 500), (600, 650), (900, 1000)])
check(dark(frame(), *CANVAS) > before, "and the tip paints again")

tap(TOOLS["BOX FILLED"]); tap(SWATCH[7]); pen([(500, 900), (900, 1100)])
tap(TOOLS["ELLIP"]); tap(BRUSH[3]); pen([(300, 400), (1100, 1150)])
g = shot("05-shapes")
check(button_active(g, *TOOL_BOX["ELLIP"]) and not button_active(g, *TOOL_BOX["BOX FILLED"]), "grid cells show the active tool")
tap(TOOLS["FILL"]); tap(SWATCH[0]); pen([(700, 760)])
g1 = shot("06-fill")
tap(UNDO)
g2 = shot("07-undo")
check(not np.array_equal(g1, g2), "undo changed the canvas back")

tap(LAYER_ADD); tap(TOOLS["PEN"]); tap(BRUSH[1]); tap(SWATCH[2]); pen([(300, 300), (1100, 300)])
g = shot("08-layer2")
tap(LAYER_VIS_ROW0)
g2 = shot("09-layer2-hidden")
check(not np.array_equal(g, g2), "hiding the top layer changed the canvas")
check(150 < box_fill(g2, LOCK_BOX) < 245, "a hidden layer's lock is greyed out")
tap(LAYER_VIS_ROW0)

tap(SAVE_AS)
keys = ["1234567890", "QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM-_"]
for ch in "SMOKE":
    r = next(i for i, row in enumerate(keys) if ch in row)
    n = len(keys[r])
    x0 = 102 + (1200 - (n * 104 + (n - 1) * 8)) // 2
    tap((x0 + keys[r].index(ch) * 112 + 52, 716 + r * 96 + 44))
shot("10-typed")
tap((DIALOG_OK_X, 1370))
time.sleep(0.3)
ase = os.path.join(SPRITES, "SMOKE.ase")
check(os.path.isfile(ase), f"saved {ase}")

for fmt, ext in enumerate(("png", "jpg", "pdf")):
    tap(EXPORT)
    if fmt:
        tap((406 + fmt * 200 + 90, 718))
    tap((DIALOG_OK_X, 1260))
    time.sleep(0.5)
    dialog_ok(frame())
    p = os.path.join(SPRITES, f"SMOKE.{ext}")
    check(os.path.isfile(p), f"exported {p}")
png = Image.open(os.path.join(SPRITES, "SMOKE.png"))
check(png.size == (256, 256) and png.mode == "RGBA", f"png is {png.size} {png.mode}")
try:
    from pypdf import PdfReader
    r = PdfReader(os.path.join(SPRITES, "SMOKE.pdf"))
    check(len(r.pages) == 1 and r.pages[0].images[0].image.size[0] >= 1024, "pdf has one page with a large image")
except ImportError:
    print("skip: pypdf not installed")

tap(OPEN)
g = shot("11-open")
tap((700, 312)); tap((DIALOG_OK_X, 1630))
g = shot("12-reopened")
check(dark(g, 760 + 130, LAYER_ROW1_Y + 10, 230, 28) > 0, "reopened sprite shows two layer rows")
# The modified mark: appears with an edit, goes away the moment SAVE finishes.
t0 = title(g)
pen([(400, 600), (700, 600)])
g = frame()
check(not np.array_equal(t0, title(g)), "editing the reopened file marks it modified")
tap(SAVE)
g = frame()
check(np.array_equal(t0, title(g)), "SAVE cleared the modified mark at once")

tap(SWATCH[7]); tap(EDIT_COLOR)
post("16 400 672 0 1;18 1000 672 0 1;17 1000 672 0 1")
time.sleep(0.3)
shot("13-color-slider")
tap((DIALOG_OK_X, 1300))

tap(NEW)
tap(DISCARD)
tap((466, 1048))  # preset 32
tap((DIALOG_OK_X, 1200))
g = shot("14-new-32")
Z = 32  # a 32x32 sprite fits at zoom 32

# Pixel perfect: a hand-drawn staircase comes out as a one-pixel diagonal.
tap(PIX)
g = frame()
check(button_active(g, 8, PIX_TOP), "PIX PERFECT toggled on")
if SETTINGS:
    check('"pixel_perfect": true' in read_text(SETTINGS), "settings.json remembers pixel perfect")
tap(TOOLS["PEN"]); tap(BRUSH[1]); tap(SWATCH[1])
pen([centre(Z, *p) for p in [(10, 10), (11, 10), (11, 11), (12, 11), (12, 12), (13, 12), (13, 13)]])
g = shot("15-pixel-perfect")
check(cell(g, Z, 10, 10) < 60 and cell(g, Z, 11, 11) < 60 and cell(g, Z, 13, 13) < 60, "diagonal pixels drawn")
check(cell(g, Z, 11, 10) > 250 and cell(g, Z, 12, 11) > 250, "the L corners were dropped")
tap(PIX)


def blink(box, lines, secs=0.9):
    """Post the input, then watch a layer-row box: did it show both inverted and normal?"""
    post(";".join(lines))
    seen = set()
    end = time.time() + secs
    while time.time() < end:
        f = box_fill(frame(), box)
        seen.add("black" if f < 60 else "white" if f > 250 else "grey")
        time.sleep(0.03)
    return seen


# A locked layer refuses the pen and blinks its lock; a hidden layer is locked too (lock greyed
# out) and blinks its eye instead.
tap(LAYER_LOCK_ROW0)
g = shot("16-locked")
check(button_present(g, *LOCK_BOX), "layer lock toggled")
seen = blink(LOCK_BOX, pen_lines([centre(Z, 16, 16), centre(Z, 18, 16)]))
g = frame()
check(cell(g, Z, 16, 16) > 250 and cell(g, Z, 18, 16) > 250, "nothing was drawn on the locked layer")
check({"black", "white"} <= seen, f"the lock blinked ({sorted(seen)})")
tap(LAYER_LOCK_ROW0)
pen([centre(Z, 16, 16), centre(Z, 18, 16)])
check(cell(frame(), Z, 16, 16) < 60, "unlocked: the pen paints again")
tap(LAYER_VIS_ROW0)
g = frame()
check(150 < box_fill(g, LOCK_BOX) < 245, "hidden layer: the lock is greyed out")
seen = blink(VIS_BOX, pen_lines([centre(Z, 16, 20), centre(Z, 18, 20)]))
check({"black", "white"} <= seen, f"the eye blinked ({sorted(seen)})")
tap(LAYER_VIS_ROW0)
g = frame()
check(cell(g, Z, 16, 20) > 250 and cell(g, Z, 16, 16) < 60, "nothing was drawn while hidden; shown again")
tap(UNDO)  # the show
tap(UNDO)  # the hide
tap(UNDO)  # the stroke
check(cell(frame(), Z, 16, 16) > 250, "UNDO covers the stroke behind the lock/hide toggles")
tap(UNDO)  # the unlock
tap(UNDO)  # the lock

# RECT SEL: its buttons appear at once; paint is clipped to the selection; dragging inside
# moves it; CLEAR empties it.
tap(TOOLS["RECT SEL"])
g = frame()
check(button_present(g, DESELECT_X, 98) and not button_enabled(g, CLEAR_X, 98), "selection group appeared, CLEAR disabled")
pen([centre(Z, 2, 2), centre(Z, 4, 4), centre(Z, 6, 6)])
g = shot("17-selected")
check(button_enabled(g, CLEAR_X, 98), "CLEAR is enabled with a selection")
tap(TOOLS["PEN"]); pen([centre(Z, 3, 3), centre(Z, 5, 3)]); pen([centre(Z, 10, 20), centre(Z, 14, 20)])
g = shot("18-clipped-paint")
check(button_present(g, DESELECT_X, 98), "selection group stays with the PEN tool while a selection exists")
check(cell(g, Z, 4, 3) < 60, "painting inside the selection lands")
check(cell(g, Z, 12, 20) > 250, "painting outside the selection is clipped")
tap(TOOLS["RECT SEL"]); pen([centre(Z, 4, 4), centre(Z, 9, 9), centre(Z, 14, 14)])
g = shot("19-moved")
check(cell(g, Z, 14, 13) < 60 and cell(g, Z, 4, 3) > 250, "the selection moved its pixels by (10, 10)")
tap(CLEAR)
g = shot("20-cleared")
check(cell(g, Z, 14, 13) > 250, "CLEAR emptied the selection")
tap(DESELECT)
check(not button_enabled(frame(), CLEAR_X, 98), "DESELECT dropped the selection")

# LASSO around what is left of the diagonal, then CLEAR.
tap(TOOLS["LASSO"]); pen([centre(Z, *p) for p in [(9, 9), (15, 9), (15, 15), (9, 15), (9, 9)]])
g = shot("21-lasso")
tap(CLEAR)
g = shot("22-lasso-cleared")
check(cell(g, Z, 10, 10) > 250 and cell(g, Z, 11, 11) > 250, "lasso + CLEAR removed the circled pixels")
tap(DESELECT)

# WAND: a tap picks the connected patch; CUT takes it to the clipboard and deselects; PASTE
# puts it back on the same spot and selects it again; with CONTIGUOUS off a tap picks every
# pixel of that color.
tap(TOOLS["PEN"]); pen([centre(Z, 20, 5)]); pen([centre(Z, 24, 5)])
tap(TOOLS["WAND"]); pen([centre(Z, 20, 5)])
g = shot("23-wand")
check(button_enabled(g, CLEAR_X, 98), "wand selected the tapped cell")
tap(CUT)
g = shot("24-cut")
check(cell(g, Z, 20, 5) > 250 and not button_enabled(g, CLEAR_X, 98), "CUT removed the cell and deselected")
tap(PASTE)
g = shot("25-pasted")
check(cell(g, Z, 20, 5) < 60 and button_enabled(g, CLEAR_X, 98), "PASTE put it back and selected it")
tap(DESELECT)
tap(CONTIG); pen([centre(Z, 20, 5)]); tap(CLEAR)
g = shot("26-wand-all-cleared")
check(cell(g, Z, 20, 5) > 250 and cell(g, Z, 24, 5) > 250, "non-contiguous wand + CLEAR removed both cells")
tap(CONTIG); tap(DESELECT)

# POLY SEL: taps add corners (the first one gets a marker), tapping the first corner closes.
tap(TOOLS["PEN"]); pen([centre(Z, 20, 20)]); pen([centre(Z, 22, 22)])
tap(TOOLS["POLY SEL"])
pen([centre(Z, 18, 18)])
g = shot("27-poly-start")
mx, my = centre(Z, 18, 18)
check(g[my, mx] > 250 and g[my + 10, mx] < 60, "the first corner has a white marker in a black ring")
check(button_enabled(g, DESELECT_X, 98), "DESELECT is enabled while a polygon is being built")
for p in [(25, 18), (25, 25), (18, 25)]:
    pen([centre(Z, *p)])
pen([centre(Z, 18, 18)])
g = shot("28-poly-closed")
check(button_enabled(g, CLEAR_X, 98), "closing the polygon made a selection")
check(g[my + 10, mx] > 200, "the start marker is gone")
tap(CLEAR)
g = shot("29-poly-cleared")
check(cell(g, Z, 20, 20) > 250 and cell(g, Z, 22, 22) > 250, "polygon + CLEAR removed the pixels inside")
tap(DESELECT)
tap(TOOLS["PEN"])
check(not button_present(frame(), DESELECT_X, 98), "selection group gone: no selection and no selection tool")
tap(TOOLS["POLY SEL"])

label = g[20:60, 760:1000].copy()
time.sleep(COOLDOWN)  # the polygon was closed with the pen: fingers may not zoom yet
post("16 600 700 0 1;16 800 700 0 2")
time.sleep(SETTLE)
post("18 500 700 0 1;18 900 700 0 2;17 500 700 0 1;17 900 700 0 2")
time.sleep(0.3)
g = shot("30-pinched")
check(not np.array_equal(label, g[20:60, 760:1000]), "pinch changed the zoom")
label = g[20:60, 760:1000].copy()


def corner(g):
    """The bottom-right corner of the canvas: the sprite's frame runs through it, so it moves
    when the canvas pans, and strokes in the test stay far away from it."""
    return g[1100:1300, 1200:1400].copy()


def drag(points, settle=SETTLE, dev=1):
    (x0, y0) = points[0]
    post(f"16 {x0} {y0} 0 {dev}")
    time.sleep(settle)
    post(";".join(f"18 {x} {y} 0 {dev}" for (x, y) in points[1:]) + f";17 {points[-1][0]} {points[-1][1]} 0 {dev}")
    time.sleep(0.3)


before = corner(g)
drag([(700, 800), (760, 840), (820, 900)])
g = shot("31-panned")
check(not np.array_equal(before, corner(g)), "one-finger drag panned the canvas")

# Palm rejection: a finger that lands and moves at once (no settling) is not a pan ...
before = corner(g)
drag([(700, 800), (760, 840), (820, 900)], settle=0)
check(np.array_equal(before, corner(frame())), "a finger that moves before settling does not pan")
# ... three contacts at once are a palm ...
post("16 600 600 0 1;16 700 650 0 2;16 650 720 0 3")
time.sleep(SETTLE)
post("18 700 700 0 1;18 800 750 0 2;18 750 820 0 3;17 700 700 0 1;17 800 750 0 2;17 750 820 0 3")
time.sleep(0.3)
check(np.array_equal(before, corner(frame())), "three contacts do not pan or zoom")
# ... a contact that jumps across the screen is not a finger ...
post("16 600 700 0 1")
time.sleep(SETTLE)
post("18 900 1000 0 1;18 920 1020 0 1;17 920 1020 0 1")
time.sleep(0.3)
check(np.array_equal(before, corner(frame())), "a contact that jumps 300 px does not pan")
# ... a contact joining one that has been down a while is the hand, not a pinch ...
post("16 600 700 0 1")
time.sleep(0.4)
post("16 800 700 0 2")
time.sleep(SETTLE)
post("18 500 700 0 1;18 900 700 0 2;17 500 700 0 1;17 900 700 0 2")
time.sleep(0.3)
g = frame()
check(np.array_equal(label, g[20:60, 760:1000]) and np.array_equal(before, corner(g)), "a late second contact neither zooms nor pans")
# ... a touch under a pen stroke is the hand, and so is one right after the pen lifts ...
post("16 700 800 0 1;257 0 0 1 1;32 500 500 50 1;34 520 520 50 1;33 520 520 0 1;257 0 0 0 1")
time.sleep(SETTLE)
post("18 760 840 0 1;18 820 900 0 1;17 820 900 0 1")
time.sleep(0.3)
check(np.array_equal(before, corner(frame())), "a touch while the pen is down does not pan")
drag([(700, 800), (760, 840), (820, 900)])  # within the cooldown after the pen lifted
check(np.array_equal(before, corner(frame())), "a touch right after the pen lifted does not pan")
# ... and once the hand has had time to leave, a finger pans again.
time.sleep(COOLDOWN)
drag([(700, 800), (760, 840), (820, 900)])
g = shot("32-panned-again")
check(not np.array_equal(before, corner(g)), "a finger pans again after the cooldown")
# A pan that is under way when the pen comes into range was the hand: the view goes back.
before = corner(g)
post("16 700 800 0 1")
time.sleep(SETTLE)
post("18 760 840 0 1;18 820 900 0 1")
time.sleep(0.3)
check(not np.array_equal(before, corner(frame())), "a settled finger pans")
post("257 0 0 1 1")
time.sleep(0.3)
g = shot("33-pan-undone")
check(np.array_equal(before, corner(g)), "the pan was undone when the pen came into range")
post("17 820 900 0 1;257 0 0 0 1")
time.sleep(COOLDOWN)

tap(QUIT)
tap(DISCARD)
time.sleep(0.5)
try:
    urllib.request.urlopen(BASE + "/", timeout=2).read()
    print("FAIL: editor still running after QUIT")
    sys.exit(1)
except Exception:
    print("ok: editor exited on QUIT")
print("PASS")
