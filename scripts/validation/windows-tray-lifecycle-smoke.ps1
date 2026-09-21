param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$Application = ''
)
$ErrorActionPreference = 'Stop'
if (-not $Application) { $Application = Join-Path $Workspace 'socks-proxy-tray.exe' }
$data = Join-Path $Workspace 'tray-smoke-data'
Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
$env:LOCALAPPDATA = $data
$env:SOCKS_PROXY_DIRECT_DNS = '1.1.1.1'
$env:SOCKS_PROXY_CORE_PATH = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe'
$env:SOCKS_PROXY_TRAY_REQUIRED = '1'
$first = Start-Process -FilePath $Application -PassThru
try {
    Start-Sleep -Seconds 3
    if ($first.HasExited) { throw "Application exited while creating the tray: $($first.ExitCode)" }
    $second = Start-Process -FilePath $Application -PassThru
    if (-not $second.WaitForExit(5000)) {
        Stop-Process -Id $second.Id -Force
        throw 'Second application instance did not exit'
    }
    if ($first.HasExited) { throw 'Primary application exited after duplicate launch' }
    $windowCloseKeepsProcess = $false
    if ($first.MainWindowHandle -ne 0) {
        Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TraySmokeNative {
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
}
'@
        [void][TraySmokeNative]::PostMessage($first.MainWindowHandle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        Start-Sleep -Seconds 2
        $windowCloseKeepsProcess = -not $first.HasExited
    }
    [pscustomobject]@{
        tray_created = $true
        single_instance = $true
        window_close_observed = ($first.MainWindowHandle -ne 0)
        window_close_keeps_process = $windowCloseKeepsProcess
    } | ConvertTo-Json -Compress
}
finally {
    if (-not $first.HasExited) { Stop-Process -Id $first.Id -Force }
    Remove-Item Env:SOCKS_PROXY_TRAY_REQUIRED -ErrorAction SilentlyContinue
}
