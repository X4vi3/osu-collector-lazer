# Bumps the version everywhere, commits, tags and pushes. GitHub Actions then builds and publishes the release.
# Usage: scripts/release.ps1 0.2.0
param([Parameter(Mandatory = $true)][string]$Version)

$ErrorActionPreference = "Stop"
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3" }
. (Join-Path $PSScriptRoot "env.ps1")
Set-Location (Split-Path -Parent $PSScriptRoot)

if (git status --porcelain) { throw "Commit or stash your changes first." }

# tauri.conf.json (the version the app and the updater report)
$conf = "src-tauri/tauri.conf.json"
$text = [IO.File]::ReadAllText($conf)
$text = [regex]::Replace($text, '"version":\s*"[^"]+"', "`"version`": `"$Version`"", 1)
[IO.File]::WriteAllText($conf, $text)

# Cargo.toml [package] version + Cargo.lock
$cargo = "src-tauri/Cargo.toml"
$text = [IO.File]::ReadAllText($cargo)
$text = [regex]::Replace($text, '(?m)^version = "[^"]+"', "version = `"$Version`"", 1)
[IO.File]::WriteAllText($cargo, $text)
cargo update --manifest-path src-tauri/Cargo.toml -p collector-to-lazer --quiet

# package.json + package-lock.json
npm version $Version --no-git-tag-version --allow-same-version | Out-Null

git add $conf $cargo src-tauri/Cargo.lock package.json package-lock.json
git commit -m "Release v$Version"
git tag "v$Version"
git push origin HEAD --tags
Write-Host "Pushed v$Version. The release appears on GitHub in ~10 minutes (Actions tab)."
