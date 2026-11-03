$ErrorActionPreference = 'Stop'

if ($env:OS -ne 'Windows_NT') {
    throw 'Run this script on Windows with the Rust MSVC toolchain.'
}

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo build --locked --release --bin clify
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
    Write-Host "Windows binary: $(Join-Path $targetDir 'release/clify.exe')"
}
finally {
    Pop-Location
}
