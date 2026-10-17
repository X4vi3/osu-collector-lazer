# Removes everything the project installed or built locally.
$ProjectRoot = Split-Path -Parent $PSScriptRoot
foreach ($d in @(".tools", "node_modules", "src-tauri\target", "dist")) {
    $p = Join-Path $ProjectRoot $d
    if (Test-Path $p) { Write-Host "Removing $p"; Remove-Item -Recurse -Force $p }
}
