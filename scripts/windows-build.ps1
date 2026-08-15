$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Target = "x86_64-pc-windows-msvc"
$Dist = Join-Path $Root "dist/windows"

function Require-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found in PATH."
    }
}

Require-Command cargo
Require-Command rustup
Require-Command go
Require-Command gcc

rustup target add $Target | Out-Null
$env:CGO_ENABLED = "1"
$env:RUSTFLAGS = (($env:RUSTFLAGS + " -C target-feature=+crt-static").Trim())

Push-Location $Root
try {
    cargo build --release --target $Target
    New-Item -ItemType Directory -Force -Path $Dist | Out-Null
    Copy-Item "target/$Target/release/fast-explorer.exe" $Dist -Force
    Copy-Item "target/$Target/release/fast_explorer_tsnet.dll" $Dist -Force
    Write-Host "Windows package: $Dist"
} finally {
    Pop-Location
}
