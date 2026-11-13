# Run with powershell.exe -STA -File scripts/integration.ps1.
# The ignored Rust test uses real clipboard APIs, but never injects keyboard input.
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
$backup=New-Object Windows.Forms.DataObject
$existing=[Windows.Forms.Clipboard]::GetDataObject()
if($existing){foreach($format in $existing.GetFormats($false)){
    $data=$existing.GetData($format,$false)
    if($null-ne$data){$backup.SetData($format,$false,$data)}
}}
try {
    # Invoke a separate shell because cargo.ps1 exits with Cargo's result.
    $command = "& '$PSScriptRoot/cargo.ps1' -CargoArgs @('test','--offline','--bin','clipwheel','--','--ignored','--test-threads=1')"
    & powershell.exe -NoProfile -Command $command
    $code=$LASTEXITCODE
} finally {
    if($existing){[Windows.Forms.Clipboard]::SetDataObject($backup,$true)}else{[Windows.Forms.Clipboard]::Clear()}
}
if($code-ne0){throw "Native clipboard test failed (exit $code)"}
$directory = Join-Path $PSScriptRoot '../target/verification'
New-Item -ItemType Directory -Force $directory | Out-Null
@{NativeClipboardRoundTrips='passed';Formats=@('Unicode text','DIBV5 with alpha','File references');MissingFile='rejected';ClipboardRestored=$true} |
    ConvertTo-Json | Set-Content (Join-Path $directory 'integration.json')
