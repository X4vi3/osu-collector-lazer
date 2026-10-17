# Runs a command inside the project-local toolchain environment.
# Usage: scripts/run.ps1 <command> [args...]
. (Join-Path $PSScriptRoot "env.ps1")
Set-Location (Split-Path -Parent $PSScriptRoot)
$cmd = $args[0]
$rest = @()
if ($args.Count -gt 1) { $rest = $args[1..($args.Count - 1)] }
& $cmd @rest
exit $LASTEXITCODE
