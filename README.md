# rm2-hello

"Hello World" program for the Remarkable 2. It draws on the e-ink screen and exits when you tap.

Follow the steps below, and you can run it on your PC: `cargo run` opens the tablet in a window where the mouse emulates your finger/pen.

## Set up Windows

You only need to open PowerShell to install dependencies:

```powershell
winget install --id Rustlang.Rustup -e
winget install --id Git.Git -e
```

This installs Rust and Git for Windows.

Once installed, use the new **Git Bash** terminal (Start menu → Git Bash), and run:

```bash
rustup default stable-gnu                          # the Rust toolchain that needs no Visual Studio
rustup target add armv7-unknown-linux-musleabihf   # the tablet's CPU for rust to build for
cargo --version                                    # if you see it prints a version, you're done
```

If `rustup` or `cargo` is **"command not found"**, close Git Bash and open it again; the installer
changed PATH but the open window doesn't know yet. Already have Visual Studio and rustup's default
MSVC toolchain? That works too; skip the `stable-gnu` line.

Get the code and run it:

```bash
git clone <this repository> rm2-hello
cd rm2-hello
cargo run
```

The first build takes a minute. Then a window with the tablet opens; click it and the app says BYE
and exits. That is the loop you'll use while developing: edit, `cargo run`, click.

## The simulator

`cargo run` (on Linux `cargo run -- --sim`, the default there is the tablet) serves the simulator at
`http://127.0.0.1:1404/` and opens it as an app window in Edge or Chrome, or a tab in your default browser.

- The white area is the tablet's 1404×1872 screen, scaled to fit the window (Zoom: Fit / 50% / 100%).
- Left mouse button = finger, right button or Input → Pen = pen, with the pressure slider. Dragging sends
  move events. A real stylus on a touchscreen reports its own pressure.
- "16 greys" previews the panel's grey levels; untick it to see the raw RGB565 pixels.
- Leave the window open: the next `cargo run` reconnects to it.
- `HELLO_SIM_OPEN=0 cargo run` never launches a browser (open the printed address yourself).
  `HELLO_SIM_ADDR=127.0.0.1:9000 cargo run` moves the server.

Not simulated: e-ink refresh timing and ghosting, rotation. The simulator only listens on 127.0.0.1
and never connects anywhere; any internet traffic you notice when the window opens is the browser's
own start-up activity (extension updates and the like), not the app.

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
./deploy.sh                 # build for the tablet, copy over USB, make it executable
./deploy.sh 192.168.1.50    # over Wi-Fi instead
./deploy.sh --no-build      # copy only
```

PowerShell users: `.\deploy.ps1 [-Device 192.168.1.50] [-NoBuild]` does the same.

On the tablet: AppLoad → Hello. Tap to launch, tap the screen to exit. Long-press the icon for
windowed mode. By hand, the script is just:

```bash
cargo build --release --target armv7-unknown-linux-musleabihf
ssh root@10.11.99.1 'mkdir -p /home/root/xovi/exthome/appload/hello'
scp app/external.manifest.json app/icon.png target/armv7-unknown-linux-musleabihf/release/hello \
    root@10.11.99.1:/home/root/xovi/exthome/appload/hello/
ssh root@10.11.99.1 'chmod +x /home/root/xovi/exthome/appload/hello/hello'
```

### If something is wrong

`ssh root@10.11.99.1 journalctl -fu xochitl` shows the app's lines (prefixed `hello:`) and AppLoad's.
A healthy launch logs `[QTFB]: Connection established`, then `hello: connected, 1404x1872 rgb565`.

- **Blank window, nothing reacts, no `hello:` lines**, and the journal says
  `Failed to start process: "Child process set up failed: execve: Permission denied"`: the binary is
  not executable. `scp` drops the execute bit on every copy, so run the `chmod +x` again
  (`deploy.sh` does, and checks it).
- **`hello: connect /tmp/qtfb.sock: ...`**: AppLoad isn't running, or the binary was started outside
  it. To run it by hand, AppLoad must be up: `QTFB_KEY=1 /home/root/xovi/exthome/appload/hello/hello`.

## How it works

The rM2's screen can only be driven by xochitl, reMarkable's own UI: the e-ink driver lives inside it
and there is no `/dev/fb0`. So the app doesn't touch hardware. It talks to AppLoad, an extension that
the xovi hook framework loads *into* xochitl:

1. Connect to AppLoad's qtfb server on the unix socket `/tmp/qtfb.sock` and send `INITIALIZE`.
2. The server creates a shared-memory framebuffer (1404×1872, RGB565) and answers with its key; `mmap` it.
3. Draw into it and send `UPDATE`; xochitl's own driver refreshes the panel.
4. Touch, pen and button events come back on the same socket. Tap → draw BYE → exit.

Because everything goes through xochitl's renderer this keeps working across firmware updates, unlike
the older rm2fb route that patched addresses inside xochitl. The trade-off is coarse refresh control.
The wire structs are `#[repr(C)]` in `src/qtfb.rs` and must match the server inside xochitl;
`cargo test` checks their sizes.

```text
src/main.rs                  the app: drawing + the tap-to-exit loop
src/backend.rs               one trait for "where it draws": the tablet or the simulator
src/qtfb.rs                  AppLoad's wire protocol + the socket/shm client (Linux only)
src/sim.rs, src/sim.html     the simulator: a tiny HTTP server + the page (std only, no crates)
src/font.rs                  5x7 bitmap font
src/bin/fake_qtfb_server.rs  Linux/macOS stand-in for the on-device server (tools/run_local_test.sh)
app/                         AppLoad manifest + icon
deploy.sh, deploy.ps1        build + copy to the tablet
.cargo/config.toml           linker settings: rust-lld for both targets, so rustup alone is enough
```

## Make it yours

Everything the app does with the screen goes through `backend::Backend` (see `run_app` in
`src/main.rs`). Try changes in the simulator, then deploy.

- `b.fb()` is a `&mut [u16]` of 1404×1872 RGB565 pixels: `0x0000` black, `0xFFFF` white.
- Pen events arrive as `INPUT_PEN_*` with screen-pixel x/y and `d` = pressure 0–100.
- `update_rect()` repaints one region; `set_refresh_mode(RefreshMode::Fast)` while drawing,
  `request_full_refresh()` for a clean repaint.
- Manifest extras: `"supportsVirtualKeyboard": true` (`INPUT_VKB_*` events), `"supportsRotation": true`,
  `"disablesWindowedMode": true`.

## Linux and macOS

Same commands, no extra setup. `cargo run` defaults to the tablet there, so use `cargo run -- --sim`
for the simulator, and `bash deploy.sh` if the clone lost the execute bit. `tools/run_local_test.sh`
additionally runs the real socket/shm client against a fake server. If you'd rather link the tablet
binary with zig, `cargo install cargo-zigbuild` and `cargo zigbuild --release --target
armv7-unknown-linux-musleabihf` works too.
