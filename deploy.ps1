# Build for the reMarkable 2 and copy the app to it. PowerShell version of deploy.sh.
# Usage (from the project root):  .\deploy.ps1 [-Device 10.11.99.1] [-NoBuild]
# (The parameter is not called -Host: $Host is a read-only built-in in PowerShell.)
param(
    [string]$Device = "10.11.99.1",
    [switch]$NoBuild
)
$ErrorActionPreference = "Stop"
$target = "armv7-unknown-linux-musleabihf"
$app = "hello"
$bin = "target\$target\release\$app"
$dest = "/home/root/xovi/exthome/appload/$app"

if (-not $NoBuild) {
    cargo build --release --target $target
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}
if (-not (Test-Path $bin)) { throw "missing $bin" }

ssh "root@$Device" "mkdir -p $dest"
if ($LASTEXITCODE -ne 0) { throw "ssh to root@$Device failed" }
scp "app\external.manifest.json" "app\icon.png" $bin "root@${Device}:$dest/"
if ($LASTEXITCODE -ne 0) { throw "scp failed" }
# scp lands the binary without the execute bit. Without it AppLoad logs
# "execve: Permission denied" and shows a blank window.
ssh "root@$Device" "chmod +x $dest/$app && test -x $dest/$app"
if ($LASTEXITCODE -ne 0) { throw "could not make $dest/$app executable" }

Write-Host "Deployed to $dest. Open AppLoad on the tablet and tap 'Hello'."
Write-Host "Logs: ssh root@$Device journalctl -fu xochitl"
