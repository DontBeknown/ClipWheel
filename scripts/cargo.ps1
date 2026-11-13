param([Parameter(ValueFromRemainingArguments=$true)][string[]]$CargoArgs)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$env:CARGO_HOME = Join-Path $root '.tools/cargo'
$env:RUSTUP_HOME = Join-Path $root '.tools/rustup'
$env:CARGO_INCREMENTAL = '0'
$llvm = Get-ChildItem (Join-Path $root '.tools') -Directory -Filter 'llvm-mingw-*' | Select-Object -First 1
if ($llvm) {
    $env:PATH = "$($llvm.FullName)/bin;$env:PATH"
    $env:CARGO_BUILD_TARGET = 'x86_64-pc-windows-gnullvm'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = "$($llvm.FullName)/bin/clang.exe"
}
& (Join-Path $env:CARGO_HOME 'bin/cargo.exe') @CargoArgs
exit $LASTEXITCODE
