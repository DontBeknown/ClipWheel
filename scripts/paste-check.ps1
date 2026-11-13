param([string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnullvm/release/clipwheel.exe")
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class IntegrationProbe {
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
 [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessage(IntPtr w,uint m,IntPtr p,IntPtr l);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr w,out RECT r);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr w);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr w);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool IsClipboardFormatAvailable(uint f);
 public struct RECT {public int Left,Top,Right,Bottom;}
}
'@
function Pump([int]$ms) { $watch=[Diagnostics.Stopwatch]::StartNew(); while($watch.ElapsedMilliseconds -lt $ms){[Windows.Forms.Application]::DoEvents();Start-Sleep -Milliseconds 10} }
function Assert([bool]$value,[string]$message){if(!$value){throw $message}}
$backup=New-Object Windows.Forms.DataObject
$existing=[Windows.Forms.Clipboard]::GetDataObject()
if($existing){foreach($format in $existing.GetFormats($false)){try{$data=$existing.GetData($format,$false);if($null-ne$data){$backup.SetData($format,$false,$data)}}catch{}}}
$form=New-Object Windows.Forms.Form
$form.Text='ClipWheel integration test';$form.Size=New-Object Drawing.Size 500,220;$form.TopMost=$true
$edit=New-Object Windows.Forms.TextBox;$edit.Multiline=$true;$edit.Dock='Fill';$form.Controls.Add($edit)
$process=$null;$hwnd=[IntPtr]::Zero
$results=[ordered]@{}
try {
 $form.Show();$edit.Focus()|Out-Null
 $env:CLIPWHEEL_TRACE='1'
 $process=Start-Process -FilePath (Resolve-Path $Exe) -PassThru -WindowStyle Hidden -RedirectStandardError "$PSScriptRoot/../.tools/integration-trace.log"
 Pump 1000
 $hwnd=[IntegrationProbe]::FindWindow('ClipWheelOverlay','ClipWheel')
 Assert ($hwnd-ne[IntPtr]::Zero) 'Application did not start'
 function Post([uint32]$message,[int]$w,[int]$l){Assert ([IntegrationProbe]::PostMessage($hwnd,$message,[IntPtr]$w,[IntPtr]$l)) "Message $message failed"}
 function Open-Wheel {
  $form.Activate();$edit.Focus()|Out-Null;[void][IntegrationProbe]::SetForegroundWindow($form.Handle);Pump 100
  Assert ([IntegrationProbe]::GetForegroundWindow()-eq$form.Handle) 'Windows did not foreground the test window; refusing to paste into another application'
  $watch=[Diagnostics.Stopwatch]::StartNew();Post 0x0312 1 0
  while(![IntegrationProbe]::IsWindowVisible($hwnd)-and$watch.ElapsedMilliseconds-lt3000){Pump 10}
  $script:lastOpenMs=$watch.Elapsed.TotalMilliseconds
  Assert ([IntegrationProbe]::IsWindowVisible($hwnd)) 'Wheel did not open'
  Pump 200
 }
 function Select-Slice([int]$y) {
  $r=New-Object IntegrationProbe+RECT;[void][IntegrationProbe]::GetWindowRect($hwnd,[ref]$r)
  $scale=($r.Right-$r.Left)/820.0
  Post 0x0202 0 (([int]($y*$scale)-shl16)-bor[int](240*$scale));Pump 500
 }
 [Windows.Forms.Clipboard]::SetText('ClipWheel alpha');Pump 250
 [Windows.Forms.Clipboard]::SetText('ClipWheel beta');Pump 250
 Open-Wheel;$results.ColdOpenMs=[Math]::Round($lastOpenMs,1)
 Select-Slice 375
 Assert ([Windows.Forms.Clipboard]::GetText()-eq'ClipWheel alpha') 'Selecting an older text entry did not restore it'
 Assert ($edit.Text-eq'ClipWheel alpha') 'Automatic paste did not reach the original text field'
 $results.TextHistoryAndAutoPaste=$true
 Open-Wheel;$results.WarmOpenMs=[Math]::Round($lastOpenMs,1)
 Post 0x0100 27 0;Pump 100
 Assert (![IntegrationProbe]::IsWindowVisible($hwnd)) 'Escape failed'
 Post 0x0111 102 0;Pump 100
 $fixture=Join-Path $PSScriptRoot '../.tools/clipboard-fixture.txt';Set-Content $fixture 'fixture'
 $files=New-Object Collections.Specialized.StringCollection;[void]$files.Add((Resolve-Path $fixture).Path)
 [Windows.Forms.Clipboard]::SetFileDropList($files);Pump 250
 Open-Wheel;Select-Slice 105
 Assert ([Windows.Forms.Clipboard]::ContainsFileDropList()) 'File reference did not round-trip'
 Assert ([Windows.Forms.Clipboard]::GetFileDropList()[0]-eq$files[0]) 'File path changed'
 $results.FileReferences=$true
 Post 0x0111 102 0;Pump 100
 $bitmap=New-Object Drawing.Bitmap 80,60
 $g=[Drawing.Graphics]::FromImage($bitmap);$g.Clear([Drawing.Color]::CornflowerBlue);$g.Dispose()
 [Windows.Forms.Clipboard]::SetImage($bitmap);Pump 250
 Open-Wheel;Select-Slice 105
 Assert ([IntegrationProbe]::IsClipboardFormatAvailable(17)) 'Image did not restore an alpha-capable DIB'
 $results.ImageRoundTrip=$true
 $bitmap.Dispose()
 # Clear must invalidate in-flight reads, not just the visible list.
 [Windows.Forms.Clipboard]::SetText('discard this');Post 0x0111 102 0;Pump 300
 Open-Wheel;Post 0x0100 27 0;Pump 100
 Post 0x0111 104 0;Pump 200
 Assert ($process.HasExited) 'Quit failed'
 $results.Quit=$true
 $results | ConvertTo-Json | Set-Content "$PSScriptRoot/../docs/verification/integration.json"
 [pscustomobject]$results
} finally {
 if($hwnd-ne[IntPtr]::Zero){[void][IntegrationProbe]::PostMessage($hwnd,0x0111,[IntPtr]104,[IntPtr]::Zero)}
 Pump 100
 $form.Dispose()
 if($existing){[Windows.Forms.Clipboard]::SetDataObject($backup,$true)}else{[Windows.Forms.Clipboard]::Clear()}
}
