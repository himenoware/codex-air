param([switch]$Check)
. "$PSScriptRoot/toolchain.ps1"
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    if ($Check) {
        cargo fmt --check
        if ($LASTEXITCODE) { throw 'Formatting check failed.' }
        cargo clippy --locked -- -D warnings
        if ($LASTEXITCODE) { throw 'Clippy failed.' }
    }
    cargo build --release --locked
    if ($LASTEXITCODE) { throw 'Release build failed.' }
    Write-Output (Join-Path (Get-Location) 'target\release\codex-air.exe')
} finally { Pop-Location }
