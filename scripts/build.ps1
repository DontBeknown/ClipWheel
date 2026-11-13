param([ValidatePattern('^[A-Za-z0-9-]+$')][string]$OutputName = 'ClipWheel')
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot/cargo.ps1" build --release --locked
if($LASTEXITCODE-ne0){throw 'Release build failed'}
$destination=Join-Path $root "dist/$OutputName"
New-Item -ItemType Directory -Force $destination | Out-Null
Copy-Item "$root/target/x86_64-pc-windows-gnullvm/release/clipwheel.exe" $destination
$llvm=Get-ChildItem "$root/.tools" -Directory -Filter 'llvm-mingw-*' | Select-Object -First 1
Copy-Item "$($llvm.FullName)/bin/libunwind.dll" $destination
Copy-Item "$($llvm.FullName)/LICENSE.TXT" "$destination/LLVM-LICENSE.txt"
Copy-Item "$root/README.md" $destination
$assetDestination=Join-Path $destination 'assets'
New-Item -ItemType Directory -Force $assetDestination | Out-Null
Get-ChildItem "$root/assets" -File | Copy-Item -Destination $assetDestination -Force
Copy-Item "$root/THIRD-PARTY-NOTICES.md" $destination
$verification=Join-Path $destination 'docs/verification'
New-Item -ItemType Directory -Force $verification | Out-Null
Copy-Item "$root/docs/verification/RESULTS.md","$root/docs/verification/smoke.json","$root/docs/verification/integration.json" $verification
$licenses=Join-Path $destination 'licenses'
New-Item -ItemType Directory -Force $licenses | Out-Null
Get-ChildItem "$root/.tools/cargo/registry/src" -Directory | ForEach-Object {Get-ChildItem -LiteralPath $_.FullName -Directory} | ForEach-Object {
    $crate=$_.Name
    Get-ChildItem -LiteralPath $_.FullName -File | Where-Object Name -Match 'LICENSE' | ForEach-Object {Copy-Item $_.FullName (Join-Path $licenses "$crate-$($_.Name)")}
    Get-ChildItem (Join-Path $_.FullName 'license*') -Directory -ErrorAction SilentlyContinue | ForEach-Object {
        Get-ChildItem $_.FullName -File | ForEach-Object {Copy-Item $_.FullName (Join-Path $licenses "$crate-$($_.Name)")}
    }
}
$rustDocs=Join-Path $root '.tools/rustup/toolchains/stable-x86_64-pc-windows-gnullvm/share/doc/rust'
Copy-Item "$rustDocs/COPYRIGHT-library.html" $licenses
Copy-Item "$rustDocs/licenses" (Join-Path $licenses 'rust') -Recurse -Force
Compress-Archive -Path "$destination/*" -DestinationPath "$root/dist/$OutputName-Windows-x64.zip" -Force
Get-Item "$destination/clipwheel.exe","$root/dist/$OutputName-Windows-x64.zip" | Select-Object FullName,Length
