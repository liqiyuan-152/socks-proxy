param(
    [string]$ResultPath = 'C:\OpenSpecValidation\windows11-wifi-reconnect-transition.json',
    [string]$WifiDescription = 'Intel(R) Wi-Fi 6 AX201 160MHz',
    [string]$ProbeAddress = '10.168.1.136',
    [int]$ProbePort = 22
)

$ErrorActionPreference = 'Stop'
$recoveryTask = 'OpenSpec-Win11-NetworkRecovery'
$recoveryScript = 'C:\OpenSpecValidation\windows-physical-network-recovery.ps1'

function Get-Wifi {
    $adapter = Get-NetAdapter -Physical | Where-Object InterfaceDescription -eq $WifiDescription
    if (@($adapter).Count -ne 1) { throw "Expected one Wi-Fi adapter: $WifiDescription" }
    $adapter
}

function Wait-WifiReady([int]$timeoutSeconds = 60) {
    $deadline = (Get-Date).AddSeconds($timeoutSeconds)
    do {
        Start-Sleep -Milliseconds 500
        $adapter = Get-Wifi
        $addresses = @(Get-NetIPAddress -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object AddressState -eq 'Preferred')
    } while (($adapter.Status -ne 'Up' -or $addresses.Count -eq 0) -and (Get-Date) -lt $deadline)
    if ($adapter.Status -ne 'Up' -or $addresses.Count -eq 0) { throw 'Wi-Fi did not reconnect' }
    $adapter
}

function Test-TcpProbe {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $connect = $client.BeginConnect($ProbeAddress, $ProbePort, $null, $null)
        if (-not $connect.AsyncWaitHandle.WaitOne(5000)) { return $false }
        $client.EndConnect($connect)
        $true
    } catch {
        $false
    } finally {
        $client.Dispose()
    }
}

function Get-Snapshot([string]$stage) {
    $wifi = Get-Wifi
    [pscustomobject]@{
        stage = $stage
        capturedAt = (Get-Date).ToString('o')
        wifiStatus = [string]$wifi.Status
        wifiIfIndex = $wifi.ifIndex
        wifiIpv4 = @(Get-NetIPAddress -InterfaceIndex $wifi.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object AddressState -eq 'Preferred' | Select-Object -ExpandProperty IPAddress)
        tcpProbe = Test-TcpProbe
        applicationProcesses = @(Get-Process socks-proxy,socks-proxy-latest -ErrorAction SilentlyContinue).Count
        coreProcesses = @(Get-Process sing-box -ErrorAction SilentlyContinue).Count
        tunAdapters = @(Get-NetAdapter -ErrorAction SilentlyContinue |
            Where-Object Name -in @('socks-proxy', 'socks-proxy-tun-v1')).Count
        ownedRoutes = @(Get-NetRoute -ErrorAction SilentlyContinue |
            Where-Object InterfaceAlias -in @('socks-proxy', 'socks-proxy-tun-v1')).Count
    }
}

Remove-Item -LiteralPath $ResultPath -Force -ErrorAction SilentlyContinue
$wifi = Get-Wifi
if ($wifi.Status -ne 'Up') { throw 'Wi-Fi must be connected before validation' }
if (@(Get-Process socks-proxy,socks-proxy-latest -ErrorAction SilentlyContinue).Count -ne 1) {
    throw 'Exactly one Socks Proxy application must be running before validation'
}

$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument "-NoProfile -ExecutionPolicy Bypass -File $recoveryScript"
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(3)
$principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
Register-ScheduledTask -TaskName $recoveryTask -Action $action -Trigger $trigger -Principal $principal -Force | Out-Null

$before = Get-Snapshot 'before'
$disconnected = $null
$reconnected = $null
$failure = $null
try {
    Disable-NetAdapter -InterfaceDescription $WifiDescription -Confirm:$false
    Start-Sleep -Seconds 5
    $disconnected = Get-Snapshot 'disconnected'
    Enable-NetAdapter -InterfaceDescription $WifiDescription -Confirm:$false
    [void](Wait-WifiReady)
    Start-Sleep -Seconds 3
    $reconnected = Get-Snapshot 'reconnected'
} catch {
    $failure = $_.Exception.Message
} finally {
    Enable-NetAdapter -InterfaceDescription $WifiDescription -Confirm:$false -ErrorAction SilentlyContinue
}

$passed = -not $failure -and
    $before.wifiStatus -eq 'Up' -and $before.tcpProbe -and
    $disconnected.wifiStatus -ne 'Up' -and -not $disconnected.tcpProbe -and
    $reconnected.wifiStatus -eq 'Up' -and $reconnected.tcpProbe -and
    $before.applicationProcesses -eq 1 -and
    $disconnected.applicationProcesses -eq 1 -and
    $reconnected.applicationProcesses -eq 1

[pscustomobject]@{
    date = '2026-09-21'
    platform = 'Microsoft Windows 11 Pro 10.0.26200 x64'
    transition = 'Wi-Fi disconnect and reconnect; wired adapter unavailable'
    probe = "$ProbeAddress`:$ProbePort"
    passed = $passed
    failure = $failure
    before = $before
    disconnected = $disconnected
    reconnected = $reconnected
} | ConvertTo-Json -Depth 7 | Set-Content -LiteralPath $ResultPath -Encoding UTF8

if ($passed) {
    Unregister-ScheduledTask -TaskName $recoveryTask -Confirm:$false -ErrorAction SilentlyContinue
    exit 0
}
exit 1
