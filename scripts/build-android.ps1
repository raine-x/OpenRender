<#
.SYNOPSIS
  Cross-compile OpenRender for Android and lay out jniLibs/.

.DESCRIPTION
  Produces, for each ABI:
    android/jniLibs/<abi>/libopenrender.so            (cdylib, JNI/CEF loading)
    android/jniLibs/static/<abi>/libopenrender.a      (staticlib, C/C++ linking)

  Requires: Rust (rustup), cargo-ndk, Android NDK r25+, and the three Android
  targets installed via `rustup target add`.

.EXAMPLE
  ./scripts/build-android.ps1
  ./scripts/build-android.ps1 -WithLatex -WithJni
#>
param(
  [string[]]$Abis = @("arm64-v8a", "armeabi-v7a", "x86_64"),
  [string]$OutDir = "android/jniLibs",
  [ValidateSet("release", "debug")] [string]$Profile = "release",
  [switch]$WithLatex,
  [switch]$WithJni,
  [switch]$SkipHeader
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$triples = @{
  "arm64-v8a"    = "aarch64-linux-android"
  "armeabi-v7a"  = "armv7-linux-androideabi"
  "x86_64"       = "x86_64-linux-android"
}

if (-not $env:ANDROID_NDK_HOME -and -not $env:ANDROID_NDK_ROOT) {
  Write-Warning "ANDROID_NDK_HOME / ANDROID_NDK_ROOT is not set; cargo-ndk may fail to locate the NDK."
}

$features = @()
if ($WithLatex) { $features += "latex" }
if ($WithJni) { $features += "jni" }
$featureArgs = @()
if ($features.Count -gt 0) { $featureArgs = @("--features", ($features -join ",")) }

$abiArgs = @()
foreach ($abi in $Abis) {
  if (-not $triples.ContainsKey($abi)) { throw "Unknown ABI: $abi" }
  $abiArgs += @("-t", $abi)
}

$profileArgs = @()
if ($Profile -eq "release") { $profileArgs = @("--release") }

Write-Host "==> cargo ndk build ($($Abis -join ', '), features: $($features -join ', '))"
& cargo ndk @abiArgs -o $OutDir build @profileArgs -p openrender-core @featureArgs
if ($LASTEXITCODE -ne 0) { throw "cargo ndk failed" }

# cargo-ndk only copies the shared library; collect the static archives too.
foreach ($abi in $Abis) {
  $triple = $triples[$abi]
  $src = Join-Path "target/$triple/$Profile" "libopenrender.a"
  if (Test-Path $src) {
    $dst = Join-Path $OutDir "static/$abi"
    New-Item -ItemType Directory -Force -Path $dst | Out-Null
    Copy-Item $src (Join-Path $dst "libopenrender.a") -Force
    Write-Host "    static: $dst/libopenrender.a"
  }
}

if (-not $SkipHeader) {
  if (Get-Command cbindgen -ErrorAction SilentlyContinue) {
    Write-Host "==> cbindgen include/engine.h"
    & cbindgen --config cbindgen.toml --crate openrender-core --output include/engine.h
  } else {
    Write-Host "==> cbindgen not installed; keeping the checked-in header (cargo install cbindgen)"
  }
}

Write-Host "==> done: $OutDir"
Get-ChildItem -Recurse $OutDir -Filter "libopenrender.*" | ForEach-Object { $_.FullName }
