# Build Trassenger for Windows
# Usage:
#   .\scripts\build-windows.ps1           — x86_64 MSI (default)
#   .\scripts\build-windows.ps1 -Arm      — ARM64 zip

param([switch]$Arm)

$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

if ($Arm) {
    Write-Host "==> Building release binaries (ARM64)..."
    cargo build --workspace --release --target aarch64-pc-windows-msvc

    Write-Host "==> Creating zip..."
    $Out = "target\release\Trassenger-0.3.1-arm64.zip"
    $Tmp = "target\release\Trassenger-arm64"
    New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
    Copy-Item "target\aarch64-pc-windows-msvc\release\trassenger-daemon.exe" $Tmp\
    Copy-Item "target\aarch64-pc-windows-msvc\release\trassenger-tui.exe" $Tmp\
    Copy-Item "README-windows-arm.txt" $Tmp\
    Compress-Archive -Force -Path "$Tmp\*" -DestinationPath $Out
    Remove-Item -Recurse -Force $Tmp

    Write-Host "==> Zip created: $Out"
} else {
    Write-Host "==> Building release binaries (x86_64)..."
    cargo build --workspace --release

    Write-Host "==> Building MSI with cargo-wix..."
    cargo wix --nocapture --output "target\release\Trassenger-0.3.1-x86_64.msi"

    Write-Host "==> MSI created: target\release\Trassenger-0.3.1-x86_64.msi"
}

Write-Host "==> Done!"
