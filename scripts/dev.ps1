. "$PSScriptRoot/toolchain.ps1"
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo run --locked
    if ($LASTEXITCODE) { throw 'Codex Air exited with an error.' }
} finally { Pop-Location }
