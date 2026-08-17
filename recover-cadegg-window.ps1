param(
  [string]$ProcessName = "cadegg"
)

$process = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
  Where-Object { $_.MainWindowHandle -ne 0 } |
  Sort-Object StartTime -Descending |
  Select-Object -First 1

if (-not $process) {
  exit 2
}

Add-Type -Namespace Win32 -Name NativeMethods -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool ShowWindowAsync(System.IntPtr hWnd, int nCmdShow);

[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool SetForegroundWindow(System.IntPtr hWnd);
'@

$hWnd = [IntPtr]$process.MainWindowHandle

# 9 = SW_RESTORE. It shows minimized or hidden top-level windows without resizing them.
[void][Win32.NativeMethods]::ShowWindowAsync($hWnd, 9)
Start-Sleep -Milliseconds 100
[void][Win32.NativeMethods]::SetForegroundWindow($hWnd)

exit 0
