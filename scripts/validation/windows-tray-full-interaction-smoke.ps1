param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$Application = '',
    [string]$CorePath = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if (-not $Application) { $Application = Join-Path $Workspace 'socks-proxy.exe' }
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box.exe' }
if (-not (Test-Path $Application)) { throw "Application is missing: $Application" }
if (-not (Test-Path $CorePath)) { throw "Core is missing: $CorePath" }

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TrayFullSmokeNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct NOTIFYICONIDENTIFIER { public uint cbSize; public IntPtr hWnd; public uint uID; public Guid guidItem; }
    [DllImport("shell32.dll")] public static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT rect);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr childAfter, string className, string windowName);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
}
'@

function Click-Rect($rect, [ValidateSet('Left','Right')]$button = 'Left') {
    $x = [int](($rect.left + $rect.right) / 2); $y = [int](($rect.top + $rect.bottom) / 2)
    if (-not [TrayFullSmokeNative]::SetCursorPos($x, $y)) { throw "SetCursorPos failed at $x,$y" }
    if ($button -eq 'Left') { $down = 2; $up = 4 } else { $down = 8; $up = 16 }
    [TrayFullSmokeNative]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero)
    [TrayFullSmokeNative]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 250
}
function Find-AppWindow {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $c = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, 'Socks Proxy')
    $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $c)
}
function Get-AppTexts {
    $window = Find-AppWindow
    if (-not $window) { return @() }
    @($window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ })
}
function Get-IconRect([int]$ProcessId) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $icon = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | Where-Object {
        $_.Current.Name -like 'Socks Proxy*' -and $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and $_.Current.BoundingRectangle.Width -gt 0 -and $_.Current.BoundingRectangle.Height -gt 0
    } | Select-Object -First 1)
    if ($icon) { $b=$icon[0].Current.BoundingRectangle; return [pscustomobject]@{left=[int]$b.Left;top=[int]$b.Top;right=[int]$b.Right;bottom=[int]$b.Bottom} }
    $candidate = [IntPtr]::Zero
    do {
        $candidate = [TrayFullSmokeNative]::FindWindowEx([IntPtr]::Zero, $candidate, 'tray_icon_app', $null)
        if ($candidate -eq [IntPtr]::Zero) { break }
        $owner = [uint32]0; [void][TrayFullSmokeNative]::GetWindowThreadProcessId($candidate, [ref]$owner)
        if ($owner -eq $ProcessId) { break }
    } while ($true)
    if ($candidate -eq [IntPtr]::Zero) { throw 'Application tray window was not found' }
    $id = New-Object TrayFullSmokeNative+NOTIFYICONIDENTIFIER
    $id.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($id); $id.hWnd = $candidate; $id.uID = 1
    $id.guidItem = [Guid]'7f461940-79c4-4a26-a1b2-b1a8ac3e11d2'
    $rect = New-Object TrayFullSmokeNative+RECT
    if ([TrayFullSmokeNative]::Shell_NotifyIconGetRect([ref]$id, [ref]$rect) -ne 0) { throw 'Tray icon bounds were not available' }
    [pscustomobject]@{ left=$rect.Left; top=$rect.Top; right=$rect.Right; bottom=$rect.Bottom }
}
function Get-MenuItems {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $menus = @($root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition) | Where-Object { $_.Current.ClassName -eq '#32768' })
    @($menus | ForEach-Object { $_.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) } | Where-Object { $_.Current.Name } | ForEach-Object {
        $b = $_.Current.BoundingRectangle
        [pscustomobject]@{ name=$_.Current.Name; left=[int]$b.Left; top=[int]$b.Top; right=[int]$b.Right; bottom=[int]$b.Bottom; element=$_ }
    })
}
function Open-Menu($icon) {
    Click-Rect $icon 'Right'
    $until=(Get-Date).AddSeconds(4)
    do { $items=Get-MenuItems; if($items.Count){ return $items }; Start-Sleep -Milliseconds 100 } while((Get-Date) -lt $until)
    throw 'Tray menu did not appear'
}
function Close-Menu { [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); Start-Sleep -Milliseconds 150 }
function Select-Menu([int]$ProcessId, [string]$name) {
    $items=Open-Menu (Get-IconRect $ProcessId); $item=$items | Where-Object name -eq $name | Select-Object -First 1
    if(-not $item){ throw "Tray menu item missing: $name" }; Click-Rect $item
}
function Select-Mode([int]$ProcessId, [string]$name) {
    $items=Open-Menu (Get-IconRect $ProcessId); $parent=$items | Where-Object name -eq '代理模式' | Select-Object -First 1
    if(-not $parent){ throw 'Proxy mode submenu missing' }; Click-Rect $parent; Start-Sleep -Milliseconds 200
    $item=Get-MenuItems | Where-Object name -eq $name | Select-Object -First 1
    if(-not $item){ throw "Mode item missing: $name" }; Click-Rect $item
    Start-Sleep -Milliseconds 800
    $confirmation = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, '确认切换')))
    [pscustomobject]@{ confirmationShown=[bool]$confirmation; status=(@(Open-Menu (Get-IconRect $ProcessId)).name)[0] }
}
function Click-Exit([int]$ProcessId) {
    $items=Open-Menu (Get-IconRect $ProcessId)
    $item=$items | Where-Object { $_.name.Length -ge 2 -and [int][char]$_.name[0] -eq 0x9000 -and [int][char]$_.name[1] -eq 0x51FA } | Select-Object -First 1
    if(-not $item){ throw "Exit item missing: $($items.name -join ' | ')" }; Click-Rect $item
}

$data=Join-Path $Workspace 'tray-full-smoke-data'; New-Item -ItemType Directory -Force -Path $data | Out-Null
$config=@'
{"schema_version":1,"revision":3,"cache_initialized":false,"profiles":{"profiles":{"11111111-1111-4111-8111-111111111111":{"id":"11111111-1111-4111-8111-111111111111","name":"Tray validation fixture","protocol":"socks5","host":{"ip":"127.0.0.1"},"port":18141,"auth_enabled":false,"credential_ref":null}},"active":"11111111-1111-4111-8111-111111111111"},"rules":[],"last_applied_mode":"rules","preferences":{"start_with_windows":false}}
'@
New-Item -ItemType Directory -Force -Path (Join-Path $data 'SocksProxy') | Out-Null
[IO.File]::WriteAllText((Join-Path $data 'SocksProxy\config.json'), $config, (New-Object Text.UTF8Encoding($false)))
[void][TrayFullSmokeNative]::SetProcessDpiAwarenessContext([IntPtr](-4))
$oldLocal=$env:LOCALAPPDATA; $oldCore=$env:SOCKS_PROXY_CORE_PATH; $oldReq=$env:SOCKS_PROXY_TRAY_REQUIRED; $oldInject=$env:SOCKS_PROXY_TEST_FORCE_DNS_FLUSH_TIMEOUT_ONCE
$env:LOCALAPPDATA=$data; $env:SOCKS_PROXY_CORE_PATH=$CorePath; $env:SOCKS_PROXY_TRAY_REQUIRED='1'; $env:SOCKS_PROXY_TEST_FORCE_DNS_FLUSH_TIMEOUT_ONCE='1'
$app=$null
try {
    $app=Start-Process -FilePath $Application -PassThru
    $until=(Get-Date).AddSeconds(20); do { Start-Sleep -Milliseconds 250; $ready=$app -and -not $app.HasExited -and (Find-AppWindow) } while(-not $ready -and (Get-Date) -lt $until)
    if(-not $ready){ throw 'App or window did not become ready' }
    $appPid=$app.Id; $inventory=Open-Menu (Get-IconRect $appPid); $top=@($inventory.name); Close-Menu
    $proxyItems=Open-Menu (Get-IconRect $appPid); $proxyParent=$proxyItems|Where-Object name -eq '切换代理'|Select-Object -First 1; Click-Rect $proxyParent; Start-Sleep -Milliseconds 200; $proxyNames=@((Get-MenuItems).name); Close-Menu
    $modeResults=@(); foreach($mode in @('全局直连','全局代理','规则代理')) { $modeResults += [pscustomobject]@{ mode=$mode; result=(Select-Mode $appPid $mode) } }
    $pages=@(@{menu='状态'; heading='状态'},@{menu='代理管理...'; heading='代理'},@{menu='规则管理...'; heading='分流规则'},@{menu='查看连接日志...'; heading='连接日志'},@{menu='设置...'; heading='设置'})
    $pageResults=@(); foreach($page in $pages){ Select-Menu $appPid $page.menu; Start-Sleep -Milliseconds 250; $pageResults += [pscustomobject]@{menu=$page.menu; heading=$page.heading; visible=[bool](Find-AppWindow); reached=((Get-AppTexts) -contains $page.heading)} }
    $firstStart=[Diagnostics.Stopwatch]::StartNew(); Click-Exit $appPid
    $until=(Get-Date).AddSeconds(3); $progress=$false; do { $progress=(Get-AppTexts) -contains '正在退出'; if(-not $progress){Start-Sleep -Milliseconds 50} } while(-not $progress -and (Get-Date) -lt $until)
    $firstVisibleMs=$firstStart.ElapsedMilliseconds
    $until=(Get-Date).AddSeconds(15); $failure=$false; do { $failure=((Get-AppTexts) -join "`n") -match '测试注入|DNS.*超时'; if(-not $failure){Start-Sleep -Milliseconds 100} } while(-not $failure -and (Get-Date) -lt $until)
    $app.Refresh(); $stillRunning=-not $app.HasExited
    $retryStart=[Diagnostics.Stopwatch]::StartNew(); Click-Exit $appPid
    $until=(Get-Date).AddSeconds(20); do { $app.Refresh(); if(-not $app.HasExited){Start-Sleep -Milliseconds 100} } while(-not $app.HasExited -and (Get-Date) -lt $until)
    [pscustomobject]@{sessionId=[Diagnostics.Process]::GetCurrentProcess().SessionId; topLevel=$top; proxyItems=$proxyNames; modes=$modeResults; pages=$pageResults; exitProgressVisible=$progress; exitProgressFirstVisibleMs=$firstVisibleMs; injectedTimeoutVisible=$failure; appStayedOpenForRetry=$stillRunning; retryExited=$app.HasExited; retryTotalMs=$retryStart.ElapsedMilliseconds; coreProcessesAfterExit=@(Get-Process sing-box -ErrorAction SilentlyContinue).Count; tunAdaptersAfterExit=@(Get-NetAdapter -Name 'socks-proxy' -ErrorAction SilentlyContinue).Count} | ConvertTo-Json -Depth 8
} finally { if($app -and -not $app.HasExited){Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue}; $env:LOCALAPPDATA=$oldLocal; $env:SOCKS_PROXY_CORE_PATH=$oldCore; $env:SOCKS_PROXY_TRAY_REQUIRED=$oldReq; $env:SOCKS_PROXY_TEST_FORCE_DNS_FLUSH_TIMEOUT_ONCE=$oldInject }
