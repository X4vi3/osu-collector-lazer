# Sets up a project-local toolchain environment. Dot-source it: `. .\scripts\env.ps1`
# Everything lives in <project>/.tools so deleting the project leaves nothing behind.
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$Tools = Join-Path $ProjectRoot ".tools"

# Only redirect toolchains that were actually installed into .tools (a normal global setup works as is).
$paths = @()
if (Test-Path (Join-Path $Tools "cargo/bin/cargo.exe")) {
    $env:RUSTUP_HOME = Join-Path $Tools "rustup"
    $env:CARGO_HOME = Join-Path $Tools "cargo"
    $paths += (Join-Path $env:CARGO_HOME "bin")
}
if (Test-Path $Tools) {
    $env:npm_config_cache = Join-Path $Tools "npm-cache"
}
$env:npm_config_update_notifier = "false"

$localNode = Join-Path $Tools "node"
if (Test-Path (Join-Path $localNode "node.exe")) { $paths += $localNode }
$paths += (Join-Path $ProjectRoot "node_modules\.bin")

foreach ($p in $paths) {
    if (-not (($env:PATH -split ';') -contains $p)) { $env:PATH = "$p;$env:PATH" }
}

# MSVC: globally installed VS Build Tools are detected by rustc automatically.
# A portable MSVC in .tools/msvc (if ever installed) takes precedence.
$portableMsvcEnv = Join-Path $Tools "msvc\setup_x64.bat"
if (Test-Path $portableMsvcEnv) {
    cmd /c "`"$portableMsvcEnv`" && set" | ForEach-Object {
        if ($_ -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($matches[1])" -Value $matches[2] }
    }
}

# Updater signing key for local release builds (CI gets it from the TAURI_SIGNING_PRIVATE_KEY secret).
$signingKey = Join-Path $ProjectRoot ".secrets/updater.key"
if ((Test-Path $signingKey) -and -not $env:TAURI_SIGNING_PRIVATE_KEY) {
    $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $signingKey -Raw).Trim()
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
}
