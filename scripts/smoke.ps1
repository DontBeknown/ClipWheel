param([string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnullvm/release/clipwheel.exe")
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class WheelProbe {
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
 [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessage(IntPtr w, uint m, IntPtr p, IntPtr l);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr w, out RECT r);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr w);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr w, IntPtr after, int x, int y, int cx, int cy, uint flags);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr w);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
 public struct RECT {public int Left, Top, Right, Bottom;}
}
'@
$env:CLIPWHEEL_TRACE='1'
$process = Start-Process -FilePath (Resolve-Path $Exe) -ArgumentList '--demo' -PassThru -WindowStyle Hidden -RedirectStandardError "$PSScriptRoot/../.tools/smoke-trace.log"
Start-Sleep -Seconds 2
if ($process.HasExited) { throw "ClipWheel exited: $($process.ExitCode)" }
$hwnd = [WheelProbe]::FindWindow('ClipWheelOverlay', 'ClipWheel')
if ($hwnd -eq [IntPtr]::Zero) { throw 'Overlay window was not created' }
$rect = New-Object WheelProbe+RECT
[void][WheelProbe]::GetWindowRect($hwnd, [ref]$rect)
# Keep automated mouse messages separate from real pointer motion where possible.
$work = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
$pointer = [Windows.Forms.Cursor]::Position
$probeX = if ($pointer.X -gt ($work.Left + $work.Width/2)) { $work.Left } else { $work.Right - ($rect.Right-$rect.Left) }
$probeY = if ($pointer.Y -gt ($work.Top + $work.Height/2)) { $work.Top } else { $work.Bottom - ($rect.Bottom-$rect.Top) }
[void][WheelProbe]::SetWindowPos($hwnd,[IntPtr]::Zero,$probeX,$probeY,0,0,0x15)
[void][WheelProbe]::GetWindowRect($hwnd, [ref]$rect)
$directory = Join-Path $PSScriptRoot '../docs/verification'
New-Item -ItemType Directory -Force $directory | Out-Null
function Save-Crop([string]$Name) {
 $bitmap = New-Object Drawing.Bitmap ($rect.Right-$rect.Left),($rect.Bottom-$rect.Top)
 $graphics = [Drawing.Graphics]::FromImage($bitmap)
 $graphics.CopyFromScreen($rect.Left,$rect.Top,0,0,$bitmap.Size)
 $bitmap.Save((Join-Path $directory $Name))
 $graphics.Dispose(); $bitmap.Dispose()
}
Save-Crop 'wheel.png'
$scale = ($rect.Right-$rect.Left)/560.0
[void][WheelProbe]::SetForegroundWindow($hwnd)
$posted = [WheelProbe]::PostMessage($hwnd,0x0200,[IntPtr]::Zero,[IntPtr](([int](56*$scale) -shl 16) -bor [int](156*$scale)))
if (!$posted) {throw "Hover message blocked: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"}
Start-Sleep -Milliseconds 500
Save-Crop 'preview.png'
$tracePath = Join-Path $PSScriptRoot '../.tools/smoke-trace.log'
if ((Get-Content $tracePath | Where-Object { $_ -match '^preview ' } | Select-Object -Last 1) -ne 'preview true') { throw 'Overflow item did not open preview' }
# Enter the preview, then scroll it without selecting or changing the clipboard.
[void][WheelProbe]::PostMessage($hwnd,0x0200,[IntPtr]::Zero,[IntPtr](([int](80*$scale) -shl 16) -bor [int](400*$scale)))
Start-Sleep -Milliseconds 120
$screenX = $rect.Left + [int](400*$scale)
$screenY = $rect.Top + [int](80*$scale)
[void][WheelProbe]::PostMessage($hwnd,0x020A,[IntPtr](-120 -shl 16),[IntPtr](($screenY -shl 16) -bor ($screenX -band 0xffff)))
Start-Sleep -Milliseconds 150
Save-Crop 'preview-scrolled.png'
if (-not (Get-Content $tracePath | Where-Object { $_ -match '^preview-scroll [1-9]' })) { throw 'Preview did not scroll' }
# Index 1 is the fitting "Hi" sample. It must dismiss the panel.
[void][WheelProbe]::PostMessage($hwnd,0x0200,[IntPtr]::Zero,[IntPtr](([int](85*$scale) -shl 16) -bor [int](227*$scale)))
Start-Sleep -Milliseconds 250
Save-Crop 'fitting-item.png'
if ((Get-Content $tracePath | Where-Object { $_ -match '^preview ' } | Select-Object -Last 1) -ne 'preview false') { throw 'Fitting item incorrectly opened preview' }
[void][WheelProbe]::PostMessage($hwnd,0x0100,[IntPtr]27,[IntPtr]::Zero)
Start-Sleep -Milliseconds 300
$process.Refresh(); $cpuStart = $process.TotalProcessorTime.TotalMilliseconds
$watch = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds 5
$process.Refresh(); $cpuEnd = $process.TotalProcessorTime.TotalMilliseconds
$result = [pscustomobject]@{
 Alive = !$process.HasExited
 HiddenAfterEscape = ![WheelProbe]::IsWindowVisible($hwnd)
 IdleCpuPercent = [Math]::Round(100*($cpuEnd-$cpuStart)/$watch.Elapsed.TotalMilliseconds,3)
 WorkingSetMiB = [Math]::Round($process.WorkingSet64/1MB,2)
 PrivateMiB = [Math]::Round($process.PrivateMemorySize64/1MB,2)
}
$openSamples=@()
for($iteration=0;$iteration-lt20;$iteration++){
 $openWatch=[Diagnostics.Stopwatch]::StartNew()
 [void][WheelProbe]::PostMessage($hwnd,0x0312,[IntPtr]1,[IntPtr]::Zero)
 while(![WheelProbe]::IsWindowVisible($hwnd)-and$openWatch.ElapsedMilliseconds-lt2000){Start-Sleep -Milliseconds 1}
 if(![WheelProbe]::IsWindowVisible($hwnd)){throw 'Repeated opening failed'}
 $openSamples+=$openWatch.Elapsed.TotalMilliseconds
 Start-Sleep -Milliseconds 40
 [void][WheelProbe]::PostMessage($hwnd,0x0100,[IntPtr]27,[IntPtr]::Zero)
 Start-Sleep -Milliseconds 40
}
$process.Refresh()
$result | Add-Member NoteProperty WarmOpenP95Ms ([Math]::Round(($openSamples|Sort-Object)[18],2))
$result | Add-Member NoteProperty WorkingSetAfter20CyclesMiB ([Math]::Round($process.WorkingSet64/1MB,2))
$result | ConvertTo-Json | Set-Content (Join-Path $directory 'smoke.json')
$result
[void][WheelProbe]::PostMessage($hwnd,0x0111,[IntPtr]104,[IntPtr]::Zero)
$process.WaitForExit(3000) | Out-Null
if (!$process.HasExited) { throw 'Tray quit did not terminate the app' }
$intervals=@();$previous=$null
foreach($line in Get-Content "$PSScriptRoot/../.tools/smoke-trace.log"){
 if($line-match'^frame ([\d.]+) ([\d.]+)'){
  $elapsed=[double]::Parse($Matches[1],[Globalization.CultureInfo]::InvariantCulture)
  if($null-ne$previous-and$elapsed-lt$previous){break}
  if($null-ne$previous-and($elapsed-$previous)-gt5-and$elapsed-lt180){$intervals+=($elapsed-$previous)}
  $previous=$elapsed
 }
}
if($intervals.Count-gt0){$result|Add-Member NoteProperty AnimationMedianIntervalMs ([Math]::Round(($intervals|Sort-Object)[[int]($intervals.Count/2)],2))}
$result | ConvertTo-Json | Set-Content (Join-Path $directory 'smoke.json')
