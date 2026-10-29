# Build an app for the reMarkable 2 and copy it to the tablet. PowerShell version of deploy.sh.
# Usage (from the project root):  .\deploy.ps1 [-App librepixel|hello] [-Device 10.11.99.1] [-NoBuild] [-NoNativeInk]
# The librepixel-ink xovi extension (native ink) is built with zig and copied too unless
# -NoNativeInk is given, and if it is not running afterwards the script restarts xochitl (which
# closes whatever is open on the tablet) and waits for it to report. If any of that fails you
# get a warning and the app is still deployed.
# (The parameter is not called -Host: $Host is a read-only built-in in PowerShell.)
param(
    [string]$App = "librepixel",
    [string]$Device = "10.11.99.1",
    [switch]$NoBuild,
    [switch]$NoNativeInk
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path "app\$App")) { throw "no app\$App directory (apps: $((Get-ChildItem app -Directory).Name -join ', '))" }
$target = "armv7-unknown-linux-musleabihf"
$bin = "target\$target\release\$App"
$dest = "/home/root/xovi/exthome/appload/$App"

if (-not $NoBuild) {
    cargo build --release --target $target --bin $App
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}
if (-not (Test-Path $bin)) { throw "missing $bin" }

ssh "root@$Device" "mkdir -p $dest"
if ($LASTEXITCODE -ne 0) { throw "ssh to root@$Device failed" }
scp "app\$App\external.manifest.json" "app\$App\icon.png" $bin "root@${Device}:$dest/"
if ($LASTEXITCODE -ne 0) { throw "scp failed" }
# scp lands the binary without the execute bit. Without it AppLoad logs
# "execve: Permission denied" and shows a blank window.
ssh "root@$Device" "chmod +x $dest/$App && test -x $dest/$App"
if ($LASTEXITCODE -ne 0) { throw "could not make $dest/$App executable" }

Write-Host "Deployed $App to $dest. Open AppLoad on the tablet and tap it."

# Native ink: build (zig) and copy the extension, then make sure a xochitl that has it loaded
# is running (restart it if not). Every failure is a warning, not an exit.
$statusProbe = 'f=/tmp/librepixel-ink.status; now=$(date +%s); m=$(stat -c %Y "$f" 2>/dev/null || echo 0); [ $((now - m)) -le 5 ] && head -n 1 "$f" 2>/dev/null; true'
# Waits for xochitl to come back: its journal from the restart on settles it (the extension's
# first line says ready, or why it stays off); the status file is the fallback. __T0__ is
# replaced with the tablet's clock reading taken before the restart.
$statusWait = 't0=__T0__; f=/tmp/librepixel-ink.status; i=0; while [ $i -lt 90 ]; do line=$(journalctl -u xochitl --since "@$t0" --no-pager 2>/dev/null | grep -o "\[librepixel-ink\] .*" | tail -1); case "$line" in *"ready on"*) echo "$line" | sed "s/.*ready on \([^:]*\).*/ready \1/"; exit 0 ;; *"staying off"*|*"not found"*|*"odd pen"*|*"missing"*) echo "off ${line#*] }"; exit 0 ;; esac; now=$(date +%s); m=$(stat -c %Y "$f" 2>/dev/null || echo 0); l=$(head -n 1 "$f" 2>/dev/null); if [ $((now - m)) -le 5 ] && [ -n "$l" ]; then echo "$l"; exit 0; fi; sleep 1; i=$((i + 1)); done; exit 1'
function Get-InkStatus {
    # The first line of the extension's status file if it was refreshed in the last 5 s, else "".
    $out = ssh "root@$Device" $statusProbe 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $out) { return "" }
    return ([string]$out).Trim()
}
function Install-NativeInk {
    $so = "extension\librepixel-ink.so"
    if (-not $NoBuild -or -not (Test-Path $so)) {
        if (Get-Command zig -ErrorAction SilentlyContinue) {
            zig cc -target arm-linux-gnueabihf.2.31 -mcpu=cortex_a7 -shared -fPIC -O2 -s `
                -o $so extension\librepixel-ink.c extension\xovi.c -ldl -lpthread
            if ($LASTEXITCODE -ne 0) { Write-Warning "building $so failed"; return $false }
        } elseif (Test-Path $so) {
            Write-Warning "zig not found; using the existing $so as it is"
        } else {
            Write-Warning "zig not found and no prebuilt $so (winget install zig.zig)"
            return $false
        }
    }
    $old = ssh "root@$Device" 'md5sum /home/root/xovi/extensions.d/librepixel-ink.so 2>/dev/null | cut -d" " -f1' 2>$null
    $new = (Get-FileHash $so -Algorithm MD5).Hash.ToLower()
    $changed = ([string]$old).Trim() -ne $new
    if ($changed) {
        # Copy to a temp name and rename, so xovi never loads a half-written file.
        scp $so "root@${Device}:/home/root/xovi/extensions.d/.librepixel-ink.new"
        if ($LASTEXITCODE -ne 0) { Write-Warning "copying $so to the tablet failed"; return $false }
        ssh "root@$Device" "mv -f /home/root/xovi/extensions.d/.librepixel-ink.new /home/root/xovi/extensions.d/librepixel-ink.so"
        if ($LASTEXITCODE -ne 0) { Write-Warning "could not move the extension into place on the tablet"; return $false }
        Write-Host "Copied the librepixel-ink extension."
    } else {
        Write-Host "The tablet already has this build of the librepixel-ink extension."
    }
    $st = Get-InkStatus
    if (-not $changed) {
        if ($st -like "ready*") { Write-Host "Native ink is running: $st"; return $true }
        if ($st -like "off*") { Write-Warning "the extension is loaded but staying off: $st (restarting xochitl cannot change that)"; return $false }
    }
    # Not running (first install, new build, or xochitl started without it): restart xochitl.
    ssh "root@$Device" 'pgrep -x librepixel >/dev/null' 2>$null
    if ($LASTEXITCODE -eq 0) { Write-Host "LibrePixel is open on the tablet; restarting xochitl closes it." }
    Write-Host "Restarting xochitl so the extension loads (the tablet's screen reloads)..."
    $t0 = ssh "root@$Device" 'date +%s' 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $t0) { $t0 = "0" }
    ssh "root@$Device" 'systemctl restart xochitl'
    if ($LASTEXITCODE -ne 0) { Write-Warning "systemctl restart xochitl failed"; return $false }
    Write-Host "Waiting for xochitl and the extension (up to 90 s)..."
    $st = ssh "root@$Device" $statusWait.Replace('__T0__', ([string]$t0).Trim()) 2>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "xochitl restarted but the extension did not report within 90 s; look at: ssh root@$Device journalctl -b -u xochitl | grep librepixel-ink"
        return $false
    }
    $st = ([string]$st).Trim()
    if ($st -like "ready*") { Write-Host "Native ink is running: $st"; return $true }
    Write-Warning "the extension is staying off: $st"
    return $false
}
if (-not $NoNativeInk -and $App -eq "librepixel") {
    if (-not (Install-NativeInk)) {
        Write-Warning "native ink is not active; the editor offers INK:LIVE and INK:BATCH only (-NoNativeInk silences this)"
    }
}
Write-Host "Logs: ssh root@$Device journalctl -fu xochitl"
