<#
.SYNOPSIS
Builds the release binaries with every local build path remapped, then verifies
that neither exe carries a machine-specific path, a user name or an address.

.DESCRIPTION
Rust bakes compile-time paths into the binary: `file!()`-style panic locations
from dependencies point into the cargo registry, and one build-script path from
our own tree leaked into glutin's output. A shipped exe must not tell the world
where it was built or who built it, so this script:

  1. remaps the repo root, CARGO_HOME, RUSTUP_HOME and USERPROFILE to neutral
     virtual roots (`C:\build\disk-usage-inspector`, `C:\cargo`, `C:\rustup`,
     `C:\home`) via CARGO_ENCODED_RUSTFLAGS (0x1f-separated, so spaces in the
     real paths are fine);
  2. builds `--release --locked` through the activated toolchain;
  3. scans both PE files (ASCII and UTF-16LE) for the real prefixes, the local
     user name and e-mail markers, and fails if anything is found.

Run it from an environment where `cargo` works, i.e. after `gccenv.bat` on this
machine:

    cmd /c "E:\msys64\gccenv.bat && pwsh -NoProfile -File tools\build-release.ps1"

CI calls it with the GNU toolchain already active. `-SkipVerify` builds only.
#>
[CmdletBinding()]
param(
    [switch]$SkipVerify
)

$ErrorActionPreference = 'Stop'

$root = (Split-Path -Parent $PSScriptRoot).TrimEnd('\')
$userHome = $env:USERPROFILE.TrimEnd('\')
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME.TrimEnd('\') } else { Join-Path $userHome '.cargo' }
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME.TrimEnd('\') } else { Join-Path $userHome '.rustup' }

# Longest prefix first: rustup lives under the profile, the registry under cargo.
$remaps = @(
    [pscustomobject]@{ From = $root; To = 'C:\build\disk-usage-inspector' }
    [pscustomobject]@{ From = $rustupHome; To = 'C:\rustup' }
    [pscustomobject]@{ From = $cargoHome; To = 'C:\cargo' }
    [pscustomobject]@{ From = $userHome; To = 'C:\home' }
) | Where-Object { $_.From } | Sort-Object { $_.From.Length } -Descending

$seps = [string][char]0x1f
$flags = @($remaps | ForEach-Object { "--remap-path-prefix=$($_.From)=$($_.To)" })
# CARGO_ENCODED_RUSTFLAGS replaces `[target.*] rustflags` from .cargo/config.toml
# instead of extending it, so the static-CRT choice has to be repeated here.
$flags += '-Ctarget-feature=+crt-static'
$existing = $env:CARGO_ENCODED_RUSTFLAGS
$all = if ($existing) { @($existing.Split([char]0x1f)) + $flags } else { $flags }
$env:CARGO_ENCODED_RUSTFLAGS = ($all -join $seps)

# Pin the release profile. A machine-level `[profile.release]` in
# %CARGO_HOME%\config.toml is merged over Cargo.toml (this box has
# opt-level="z", lto=true, strip=true there), which would make the local
# artifact differ from CI in both size and symbol content. Environment
# overrides win over manifest and config, so pinning here keeps the published
# bytes deterministic.
$env:CARGO_PROFILE_RELEASE_OPT_LEVEL = '3'
$env:CARGO_PROFILE_RELEASE_LTO = 'thin'
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '1'
$env:CARGO_PROFILE_RELEASE_PANIC = 'abort'
$env:CARGO_PROFILE_RELEASE_STRIP = 'symbols'

Write-Host 'remapping build paths:'
foreach ($r in $remaps) { Write-Host ("  {0} -> {1}" -f $r.From, $r.To) }

Push-Location $root
try {
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

if ($SkipVerify) {
    Write-Host 'build only (-SkipVerify): path check skipped'
    return
}

# Needles that must not appear in a shipped binary.
$userName = Split-Path -Leaf $userHome
$needles = @($root, $cargoHome, $rustupHome, $userHome)
if ($userName -and $userName.Length -ge 3) { $needles += $userName }
# Generic profile markers as well as the concrete paths: they catch a build
# whose real CARGO_HOME/RUSTUP_HOME did not match what this script resolved
# (CI images move them around), and any other user profile that leaked in.
$needles += @(':\Users\', '\AppData\', '\Documents and Settings\')
$needles += @('@gmail.com', '@users.noreply', 'theevildays')
$needles = $needles | Where-Object { $_ } | Select-Object -Unique

function Test-Binary([string]$path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    $views = @(
        [System.Text.Encoding]::Latin1.GetString($bytes),
        [System.Text.Encoding]::Unicode.GetString($bytes)
    )
    $hits = @()
    foreach ($needle in $needles) {
        $n = 0
        foreach ($v in $views) { $n += ([regex]::Matches($v, [regex]::Escape($needle))).Count }
        if ($n -gt 0) { $hits += ("{0} x{1}" -f $needle, $n) }
    }
    return $hits
}

$failed = $false
foreach ($exe in @('disk-usage-inspector.exe', 'duiscan.exe')) {
    $path = Join-Path $root "target\release\$exe"
    if (-not (Test-Path $path)) { throw "missing build artifact: $path" }
    $hits = Test-Binary $path
    if ($hits) {
        $failed = $true
        Write-Host "LEAK in ${exe}:" -ForegroundColor Red
        $hits | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
    } else {
        Write-Host "clean: $exe carries no local path, user name or address"
    }
}

if ($failed) {
    throw 'release binaries leak local build information; see the list above'
}
Write-Host 'release build verified: no local paths or personal identifiers'
