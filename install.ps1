# Cadence installer for Windows PowerShell
# Usage:
#   irm https://raw.githubusercontent.com/NoelPaff/cadence/main/install.ps1 | iex

$ErrorActionPreference = "Stop"
$Repo = "NoelPaff/cadence"
$InstallDir = Join-Path $HOME ".local\bin"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$Asset = "cadence-x86_64-pc-windows-msvc.zip"
$Url = "https://github.com/$Repo/releases/latest/download/$Asset"
$TmpZip = Join-Path $env:TEMP $Asset
$TmpExtract = Join-Path $env:TEMP "cadence_extract_$PID"

try {
    Write-Host "==> Downloading prebuilt Cadence for Windows..."
    Invoke-WebRequest -Uri $Url -OutFile $TmpZip -UseBasicParsing
    Expand-Archive -Path $TmpZip -DestinationPath $TmpExtract -Force
    Copy-Item -Path (Join-Path $TmpExtract "cadence.exe") -Destination (Join-Path $InstallDir "cadence.exe") -Force
} catch {
    Write-Host "==> Prebuilt zip unavailable; building with Cargo..."
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Error "Cargo not found. Install Rust from https://rustup.rs/ or download a release from https://github.com/$Repo/releases"
    }
    cargo install --git "https://github.com/$Repo.git" --locked --root (Join-Path $HOME ".local")
} finally {
    if (Test-Path $TmpZip) { Remove-Item $TmpZip -Force }
    if (Test-Path $TmpExtract) { Remove-Item $TmpExtract -Recurse -Force }
}

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "Added $InstallDir to your User PATH."
}

Write-Host "✓ Installed Cadence! Run 'cadence' in your terminal."
