# Build Trassenger for Windows: produces an .msi installer
# Prerequisites: cargo-wix (cargo install cargo-wix), WiX Toolset v3
# Usage: .\scripts\build-windows.ps1

$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

Write-Host "==> Building release binaries..."
cargo build --workspace --release

Write-Host "==> Building MSI with cargo-wix..."
# cargo-wix uses wix/main.wxs automatically
cargo wix --nocapture --output "target/release/Trassenger-0.2.1-x86_64.msi"

Write-Host "==> MSI created: target/release/Trassenger-0.2.1-x86_64.msi"
Write-Host "==> Done!"
