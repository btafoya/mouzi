if ($env:CI -ne 'true') { throw 'Desktop diagnostics are restricted to isolated CI runners' }
$appProcess = Get-Process -Id ([int]$env:MOUZI_CAPTURE_PID) -ErrorAction Stop
if ($appProcess.ProcessName -notlike 'Mouzi*') { throw 'Unexpected application process' }
$appProcess | Select-Object Id,ProcessName,MainWindowTitle,Responding | Format-Table
if ($appProcess.MainWindowHandle -eq 0) { Write-Output 'Mouzi has not created a main window'; exit 0 }
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class MouziWindowCapture {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
'@
$rect = New-Object MouziWindowCapture+Rect
[MouziWindowCapture]::GetWindowRect($appProcess.MainWindowHandle, [ref]$rect) | Out-Null
$bitmap = New-Object Drawing.Bitmap(($rect.Right-$rect.Left), ($rect.Bottom-$rect.Top))
$graphics = [Drawing.Graphics]::FromImage($bitmap)
$device = $graphics.GetHdc()
try {
  [MouziWindowCapture]::PrintWindow($appProcess.MainWindowHandle, $device, 2) | Out-Null
} finally {
  $graphics.ReleaseHdc($device)
  $graphics.Dispose()
}
try {
  $bitmap.Save((Join-Path $env:MOUZI_CAPTURE_DIR 'startup-mouzi-window.png'))
} finally {
  $bitmap.Dispose()
}
