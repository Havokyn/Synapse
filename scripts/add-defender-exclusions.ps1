<#
.SYNOPSIS
  Add Microsoft Defender real-time-scan exclusions for the Rust build tree.
  This is one of the largest single build-speed wins on Windows: without it,
  Defender scans every one of the hundreds of thousands of object/rlib/incremental
  files Cargo writes, on every build, serializing I/O behind the AV engine.

.DESCRIPTION
  Excludes the code root and the Rust toolchain/cargo caches from real-time
  scanning, and excludes the compiler processes themselves. Adding an exclusion
  requires administrator rights, so this script SELF-ELEVATES (one UAC prompt).

  Excludes:
    * <code root>           (default C:\code — all repos + their target/ dirs)
    * %USERPROFILE%\.cargo   (registry, git deps, installed binaries)
    * %USERPROFILE%\.rustup  (toolchains, incl. rust-lld)
    * processes: rustc.exe, cargo.exe, rust-lld.exe, link.exe, cl.exe

  SAFETY: exclusions only apply to your own dev directories. Code you build and
  run still executes normally; this only tells Defender not to RE-SCAN these
  build inputs/outputs on every file touch. Review before running if unsure.

.PARAMETER CodeRoot
  Root directory to exclude. Default C:\code.

.PARAMETER Remove
  Remove the exclusions instead of adding them.

.EXAMPLE
  pwsh -File .\scripts\add-defender-exclusions.ps1
#>
[CmdletBinding()]
param(
    [string]$CodeRoot = 'C:\code',
    [switch]$Remove
)
$ErrorActionPreference = 'Stop'

# self-elevate if not admin
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "[defender] Elevating (UAC prompt)..."
    $argl = @('-NoProfile','-ExecutionPolicy','Bypass','-File',"`"$PSCommandPath`"",'-CodeRoot',"`"$CodeRoot`"")
    if ($Remove) { $argl += '-Remove' }
    Start-Process -FilePath (Get-Command pwsh).Source -ArgumentList $argl -Verb RunAs
    return
}

$paths = @($CodeRoot, (Join-Path $env:USERPROFILE '.cargo'), (Join-Path $env:USERPROFILE '.rustup'))
$procs = @('rustc.exe','cargo.exe','rust-lld.exe','link.exe','cl.exe')

if ($Remove) {
    foreach ($p in $paths) { Remove-MpPreference -ExclusionPath $p -ErrorAction SilentlyContinue }
    foreach ($x in $procs) { Remove-MpPreference -ExclusionProcess $x -ErrorAction SilentlyContinue }
    Write-Host "[defender] Removed Synapse build exclusions."
} else {
    foreach ($p in $paths) { Add-MpPreference -ExclusionPath $p; Write-Host "[defender] + path  $p" }
    foreach ($x in $procs) { Add-MpPreference -ExclusionProcess $x; Write-Host "[defender] + proc  $x" }
    Write-Host "[defender] Done. Current path exclusions:"
    (Get-MpPreference).ExclusionPath | ForEach-Object { Write-Host "    $_" }
}
