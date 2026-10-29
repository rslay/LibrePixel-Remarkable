# LibrePixel

A raster image editor and pixel-art editor for the reMarkable 2, written from in Rust.

- Draw with the pen on a layered sprite over a 16-color palette. The eraser end of the Marker
  erases. Finger pans, two fingers zoom.
- Saves `.ase` files that Aseprite and LibreSprite open as they are.
- Exports images as PDF straight into the tablet's own library, and PNG and JPG to copy off USB or SSH.
- Runs on your PC too: `cargo run` opens the tablet in a browser window where the mouse is your
  finger (left button) or pen (right button). Same code, same events as on the device.

## Set up Windows

You only need to open PowerShell to install dependencies:

```powershell
winget install --id Rustlang.Rustup -e
winget install --id Git.Git -e
```

This installs Rust and Git for Windows. When done, close that window.

From now on use **Git Bash** (Start menu → Git Bash). In it:

```bash
rustup default stable-gnu                          # the Rust toolchain that needs no Visual Studio
rustup target add armv7-unknown-linux-musleabihf   # the tablet's CPU
cargo --version                                    # prints a version: you're set
```

If running `rustup` or `cargo` gives you "command not found", close Git Bash and open it again; the installer changed PATH but the open window doesn't know yet. Already have Visual Studio and rustup's default MSVC toolchain? That works too; skip the `stable-gnu` line.

Get the code and run it:

```bash
git clone <this repository> librepixel
cd librepixel
cargo run
```

The first build takes a minute or two (it fetches three small pure-Rust crates for zlib, PNG and
JPG). Then a window with the tablet opens showing the editor. That is the loop you'll use while
developing: edit, `cargo run`, try it.

## Using the editor

The screen is laid out for fingers and the pen: a two-row top bar, tools down the left, the
canvas, and the palette and layers along the bottom.

- **Pen** draws with the active tool. **One finger** on the canvas pans, **two fingers** zoom; the
  magnifier buttons (-, +, FIT) do the same. Any button is a finger tap.
- **Your hand** is ignored, aggressively: anything touching the screen while the pen is on or
  near it is the hand; so is a touch that moves before it has rested 150 ms or wobbles less than
  30 px, three or more contacts at once, a contact that joins one already down for more than a
  quarter second (the two fingers of a pinch land together), and a contact that jumps more than
  120 px between two reports. For 1.2 s after the pen lifts a touch may still tap a button (a
  short tap) but cannot pan or zoom. If a pan or pinch is still going, or began less than 0.6 s
  earlier, when the pen comes back into range, it was the hand coming down: the view jumps back
  to where it was. To pan on purpose, rest one finger a moment, then drag; to zoom, land two
  fingers together. On the tablet the touch panel also reports each contact's size; set
  `palm_touch_major` in the settings file to the size above which a contact is a hand
  (`RM2_TRACE=1` logs the sizes it sees, and every touch it rejects and why, so you can pick a
  value; 0 leaves this off).
- **Eraser end:** flip the Marker over and it erases, whatever tool is chosen. If PEN was the tool
  before, it comes back by itself about 150 ms after the eraser end lifts; any other tool stays on
  ERASE until you pick one.
- **Tools:** PEN, ERASE, LINE, FILL (bucket) and PICK (eyedropper), each with its icon; then
  SHAPES (BOX, BOX FILLED, ELLIP, ELLIP FILLED) and SELECTION (RECT SEL, LASSO, POLY SEL, WAND)
  as 2x2 grids. Brush size 1 to 4 applies to PEN, ERASE, LINE and outlines.
  **PIX PERFECT** (under the brush sizes) is Aseprite's pixel-perfect pencil: the corner pixel of
  every L in a freehand stroke is dropped, so hand-drawn diagonals come out one pixel wide. With
  WAND active a **CONTIGUOUS** toggle appears below it: ON picks only the connected patch of the
  tapped color, OFF every pixel of that color on the layer.
- **Selection:** RECT SEL drags out a rectangle, LASSO circles pixels freehand, POLY SEL takes
  one tap per corner (the first corner gets a white dot in a black ring; tap it again to close
  the polygon, DESELECT abandons it), WAND taps a color; a dashed outline marks the selection.
  While one exists every painting tool is clipped to it.
  With a selection tool active, press inside the outline and drag to move the selected pixels.
  Whenever a selection exists, or a selection tool is active, the top bar shows CUT, COPY,
  PASTE, CLEAR and DESELECT: COPY puts the selected pixels on
  an in-memory clipboard and keeps the selection; CUT does the same, empties them to the paper
  and deselects; PASTE puts the clipboard back on the exact same spot of whichever layer is
  current and selects it again (so it can be moved); CLEAR empties the selection; DESELECT, or
  a tap outside it with RECT SEL or LASSO, drops it.
- **Palette:** 16 slots. Slot 0 is "the paper": white on the background layer and transparent on
  every layer above it, so ERASE simply paints slot 0. Tap a slot to pick it, EDIT COLOR to set its
  true RGB with sliders. Messages from the editor (saved, exported, warnings) appear on the line
  under the palette.
- **Seeing color on a grey screen:** VIEW:IDX (the default, next to EDIT COLOR) gives every slot
  its own distinct grey, ordered by the slot's brightness, so all 16 colors stay tellable apart
  while you edit. VIEW:LUMA shows each slot at its real brightness instead. Files always keep
  the real colors.
- **Layers:** ADD, DEL, UP, DOWN, NAME; every row has an eye (SHOW: open = visible, closed =
  hidden), a lock (LOCK) and the name; tap a row to work on it. A locked layer refuses every edit
  and its lock blinks when you try; a hidden layer is locked as well (its lock is greyed out) and
  its eye blinks instead. The lock is kept in the file (Aseprite's "editable" flag). Groups from
  Aseprite files are shown indented and respected but not created here.
- **UNDO/REDO** cover every edit; **GRID** toggles the pixel grid; **REFRESH** asks the panel for a
  full refresh when ghosting builds up (the screen pauses for about a second; that wait is inside
  AppLoad, so the editor never asks for it on its own).
- **INK** (top bar) picks how a stroke reaches the e-ink panel, and is remembered between runs:
  - **INK:LIVE** sends many small updates (one every 30 ms) and shows the live stroke in solid
    black or white, then its real grey when the pen lifts. On the panel these updates queue up
    behind each other, so long strokes trail the pen.
  - **INK:BATCH** sends fewer, larger updates (one every 100 ms, `batched_flush_ms` in the
    settings file), which the panel keeps up with.
  - **INK:NATIVE** lets xochitl itself draw the live line through the same pen path it uses for
    notebooks and PDFs, so it appears as you draw; the editor then paints the real pixels about
    once a second and when the pen lifts. It needs the `librepixel-ink` extension (see "Native ink"
    below) and is the default the first time the editor finds it.
- **Files:** NEW, OPEN, SAVE, SAVE AS (an on-screen keyboard; names are letters, digits, `-`, `_`).
  Sprites live in `~/sprites` (`/home/root/sprites` on the tablet, your user folder on the PC);
  `RM2_SPRITE_DIR=<dir>` moves them. OPEN also imports PNG files found there.
- **EXPORT:** PNG (with transparency) or JPG at 1x to 8x, or PDF. PNG and JPG land next to your
  sprites, named after the sprite: `/home/root/sprites/NAME.png` on the tablet, the `sprites`
  folder in your user directory on the PC (`RM2_SPRITE_DIR` moves both). On the tablet the PDF goes
  into the tablet's own library (`/home/root/.local/share/remarkable/xochitl/`) next to your
  notebooks; it shows up once xochitl restarts, which the export dialog offers to do right away
  (that closes the editor) or you can reboot later. On the PC the PDF is written next to the
  sprite files.

### What it does and doesn't do with files

Native format is `.ase` (Aseprite/LibreSprite), one frame, indexed color, 16-entry palette, any
number of image layers and groups. Opening a file with more than 16 colors maps pixels to the
nearest of its first 16 colors; an RGBA or grayscale file is converted the same way; an animation
keeps only its first frame. You are told when any of this happens. Saving always writes an indexed
single-frame file.

*Not supported*: animation, flip and rotate, custom brushes, layer opacity editing, or more than 16 colors at a time.

### Settings

`settings.json` next to the binary (`/home/root/xovi/exthome/appload/librepixel/settings.json` on
the tablet; `RM2_SETTINGS=<path>` moves it) remembers the INK mode, PIX PERFECT, CONTIGUOUS,
GRID, the VIEW mode, `batched_flush_ms`, `palm_touch_major`, and the tablet OS version the
editor last ran on. Edit it by hand if you like; a broken file just means defaults.

## The simulator

`cargo run` (on Linux `cargo run -- --sim`, the default there is the tablet) serves the simulator at
`http://127.0.0.1:1404/` and opens it as an app window in Edge or Chrome, or a tab in your default browser.

- The white area is the tablet's 1404×1872 screen, scaled to fit the window (Zoom: Fit / 50% / 100%).
- Left mouse button = finger, right button or Input → Pen = pen, middle button or Input → Eraser =
  the eraser end of the pen, with the pressure slider. A real stylus on a touchscreen reports its
  own pressure and its own eraser.
- "16 greys" previews the panel's grey levels; untick it to see the raw RGB565 pixels.
- Leave the window open: the next `cargo run` reconnects to it.
- `HELLO_SIM_OPEN=0 cargo run` never launches a browser (open the printed address yourself).
  `HELLO_SIM_ADDR=127.0.0.1:9000 cargo run` moves the server.
- `cargo run -- path/to/file.ase` opens a file at start.

Not simulated: e-ink refresh timing and ghosting, rotation. The simulator only listens on 127.0.0.1
and never connects anywhere; any internet traffic you notice when the window opens is the browser's
own start-up activity (extension updates and the like), not the app.

`tools/sim_smoke.py` drives the editor through the simulator (strokes, taps, save, all three
exports, open, new, pinch, pan, quit) and checks the results; its header says how to run it.

## Put it on the tablet

### Tablet, once

1. **SSH.** Settings → General → Software → Developer mode (on older 3.x firmware it is simply on).
   The root password is shown under Settings → Help → Copyright and licenses. Over USB the tablet is
   `root@10.11.99.1`. To stop typing the password, put your key on it (make one with `ssh-keygen` if
   `~/.ssh` is empty):
   ```bash
   cat ~/.ssh/id_*.pub | ssh root@10.11.99.1 'mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys'
   ```
2. **Disable auto-update** (Settings → Software). An update unloads xovi until you reinstall it.
3. **Install xovi + AppLoad.** Easiest with [reManager](https://github.com/rmitchellscott/remanager)
   (a desktop app). Or over SSH, once Vellum is on the tablet:
   ```bash
   vellum add launcherctl-appload
   launcherctl switch-launcher --start appload
   ```
4. Reboot. AppLoad appears in xochitl's top-left menu.

### Deploy

```bash
./deploy.sh                      # build the editor for the tablet, copy over USB, make it executable
./deploy.sh 192.168.1.50         # over Wi-Fi instead
./deploy.sh --no-build           # copy only
./deploy.sh --app hello          # the hello world instead of the editor
```

Every deploy of the editor also builds and copies the native-ink extension (next section) and,
if the extension is not already running, restarts xochitl so it loads, which closes whatever is
open on the tablet. `./deploy.sh --no-native-ink` skips all of that. If any of it fails (no zig,
no copy, the extension refusing the build) the script warns and the editor is still deployed.
PowerShell users: `.\deploy.ps1 [-App hello] [-Device 192.168.1.50] [-NoBuild] [-NoNativeInk]`
does the same.

### Native ink (the `librepixel-ink` extension)

Through AppLoad every app's pixels go through xochitl's Qt renderer and get its slow greyscale
refresh, which is why INK:LIVE and INK:BATCH can never feel like writing in a notebook. The
`librepixel-ink` xovi extension fixes that the way xochitl's own pen does: it runs inside xochitl
and puts pixels on the panel through xochitl's fast pen path. While the editor arms it (only
with PEN or ERASE active, only inside the sprite's rectangle, only while the editor is running)
the editor sends it every block it has just decided to paint, with PIX PERFECT's corner
decisions already made, so the preview is exactly the editor's own pixels; grid lines are left
alone, black for the tip, white for the eraser end. The editor's real greys follow and replace
the preview. It is derived from the pen booster of
[RemarkableAnimator](https://github.com/dav01d/RemarkableAnimator) (MIT).

It calls two functions inside xochitl by address, so it is tied to exact OS builds and checks
xochitl's build id before doing anything. Known builds: **3.29.0.153**. On any other build it
logs `[librepixel-ink] unknown xochitl build, staying off` and the editor simply offers LIVE and
BATCH. Adding a build is one entry in the table at the top of `extension/librepixel-ink.c`.

Install (needs zig on the PC: `winget install zig.zig`):

```bash
./deploy.sh      # builds the editor and the extension, copies both, restarts xochitl if the extension isn't running yet
```

The script ends with `Native ink is running: ready 3.29.0.153` (or a warning saying why not; the
journal has the same under `[librepixel-ink]`). Start the editor: the INK button reads
INK:NATIVE. To remove the extension:
`ssh root@10.11.99.1 'rm /home/root/xovi/extensions.d/librepixel-ink.so && systemctl restart xochitl'`.

On the tablet: AppLoad → LibrePixel. Long-press the icon for windowed mode. By hand, the script is just:

```bash
cargo build --release --target armv7-unknown-linux-musleabihf --bin librepixel
ssh root@10.11.99.1 'mkdir -p /home/root/xovi/exthome/appload/librepixel'
scp app/librepixel/external.manifest.json app/librepixel/icon.png target/armv7-unknown-linux-musleabihf/release/librepixel \
    root@10.11.99.1:/home/root/xovi/exthome/appload/librepixel/
ssh root@10.11.99.1 'chmod +x /home/root/xovi/exthome/appload/librepixel/librepixel'
```

Getting your work off the tablet: `scp root@10.11.99.1:/home/root/sprites/NAME.png .` (or the
`.ase`, `.jpg`, `.pdf`). Putting an `.ase` or `.png` on it: `scp FILE root@10.11.99.1:/home/root/sprites/`.

### If something is wrong

`ssh root@10.11.99.1 journalctl -fu xochitl` shows the app's lines (prefixed `librepixel:` or `hello:`)
and AppLoad's. A healthy launch logs `[QTFB]: Connection established`, then
`librepixel: connected, 1404x1872 rgb565`.

- **Blank window, nothing reacts, no app lines**, and the journal says
  `Failed to start process: "Child process set up failed: execve: Permission denied"`: the binary is
  not executable. `scp` drops the execute bit on every copy, so run the `chmod +x` again
  (`deploy.sh` does, and checks it).
- **`librepixel: connect /tmp/qtfb.sock: ...`**: AppLoad isn't running, or the binary was started outside
  it. To run it by hand, AppLoad must be up: `QTFB_KEY=1 /home/root/xovi/exthome/appload/librepixel/librepixel`.
  Add `RM2_TRACE=1` to that line and the journal shows every rectangle the editor sends to the
  panel, which is the thing to look at if drawing ever feels slow again.
- **The exported PDF isn't in the library**: xochitl only scans its documents when it starts.
  Restart it (`ssh root@10.11.99.1 systemctl restart xochitl`) or reboot the tablet.

## How it works

The rM2's screen can only be driven by xochitl, reMarkable's own UI: the e-ink driver lives inside it
and there is no `/dev/fb0`. So the app doesn't touch hardware. It talks to AppLoad, an extension that
the xovi hook framework loads *into* xochitl:

1. Connect to AppLoad's qtfb server on the unix socket `/tmp/qtfb.sock` and send `INITIALIZE`.
2. The server creates a shared-memory framebuffer (1404×1872, RGB565) and answers with its key; `mmap` it.
3. Draw into it and send `UPDATE` for the changed rectangle; xochitl's own driver refreshes the panel.
4. Touch, pen and button events come back on the same socket.

While you draw, the pixels the stroke touched become one small `UPDATE` per flush: every 30 ms
in INK:LIVE (solid black or white while the pen is down, real greys once it lifts), every 100 ms
in INK:BATCH, about once a second in INK:NATIVE where the extension shows the live line. The
panel runs each update for a few hundred milliseconds and makes overlapping ones wait for each
other, which is why fewer, larger updates feel faster than many small ones. The protocol also has
refresh-mode and full-refresh messages, but AppLoad pauses this client for a second after each
one and does not apply the mode to the display, so the editor only sends one when you tap REFRESH.

AppLoad reports both ends of the pen as the same pen, so to tell the eraser end apart the editor
also reads the Wacom digitizer's own input device (`/dev/input/event1`, found by name), read-only
and never grabbed, so xochitl keeps seeing the pen exactly as before.

Because everything goes through xochitl's renderer this keeps working across firmware updates, unlike
the older rm2fb route that patched addresses inside xochitl. The trade-off is coarse refresh control.
The wire structs are `#[repr(C)]` in `src/qtfb.rs` and must match the server inside xochitl;
`cargo test` checks their sizes.

```text
src/backend.rs               one trait for "where it draws": the tablet or the simulator
src/qtfb.rs                  AppLoad's wire protocol + the socket/shm client (Linux only)
src/sim.rs, src/sim.html     the simulator: a tiny HTTP server + the page (std only)
src/gfx.rs, src/font.rs      rectangles, RGB565 colors, a canvas, a 5x7 font
src/doc.rs                   sprite, layers, 16-color palette, compositing, the two grey view modes
src/ase.rs                   the .ase file format (spec in docs/ase-format.txt)
src/raster.rs                brush, line, rectangle, ellipse, flood fill, pixel-perfect rule on indexed images
src/select.rs                selections: rectangle, lasso polygon, move
src/undo.rs                  snapshot undo/redo with a memory cap
src/export.rs, src/pdf.rs    PNG/JPG export, PNG import, the one-page PDF writer
src/xochitl.rs               dropping a PDF into the tablet's library
src/settings.rs              settings.json (flat JSON, no crate)
src/ink.rs                   the editor's side of the librepixel-ink extension (arm + status files)
extension/                   librepixel-ink.c (xovi extension), xovi.c (generated glue), build with tools/build_ink.sh
src/bin/librepixel/          the editor: layout + widgets, canvas view, dialogs, the app itself
src/bin/hello.rs             the hello world (smoke test for the pipeline)
src/bin/fake_qtfb_server.rs  Linux/macOS stand-in for the on-device server (tools/run_local_test.sh)
app/librepixel, app/hello    AppLoad manifests + icons
deploy.sh, deploy.ps1        build + copy to the tablet
.cargo/config.toml           linker settings: rust-lld for both targets, so rustup alone is enough
```

## Linux and macOS

Same commands, no extra setup. `cargo run` defaults to the tablet there, so use `cargo run -- --sim`
for the simulator, and `bash deploy.sh` if the clone lost the execute bit. `tools/run_local_test.sh`
additionally runs the hello world's real socket/shm client against a fake server.

## License and credits

GPL-2.0 (see `LICENSE`). The design follows Aseprite and its GPL fork LibreSprite: the `.ase` file
format (`docs/ase-format.txt` is their spec), the layer and palette model, and the idea of the tools.
No code was taken from them. The default palette is DawnBringer's 16-color palette. The
`librepixel-ink` extension is derived from RemarkableAnimator's `animator-ink` (MIT, David @
TheoryStudios; `extension/LICENSE.animator`).
