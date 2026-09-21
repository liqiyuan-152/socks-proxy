param(
    [string]$ResultPath = 'C:\OpenSpecValidation\windows11-physical-network-transition.json',
    [string]$WifiDescription = 'Intel(R) Wi-Fi 6 AX201 160MHz',
    [string]$EthernetDescription = 'Realtek Gaming 2.5GbE Family Controller',
    [string]$ProbeAddress = '10.168.1.136',
    [int]$ProbePort = 22
)

$ErrorActionPreference = 'Stop'
$recoveryTask = 'OpenSpec-Win11-NetworkRecovery'
$recoveryScript = 'C:\OpenSpecValidation\windows-physical-network-recovery.ps1'

function Get-PhysicalAdapter([string]$description) {
    $adapter = Get-NetAdapter -Physical | Where-Object InterfaceDescription -eq $description
    if (@($adapter).Count -ne 1) { throw "Expected one physical adapter: $description" }
    $adapter
}

function Wait-AdapterReady([string]$description, [int]$timeoutSeconds = 45) {
    $deadline = (Get-Date).AddSeconds($timeoutSeconds)
    do {
        Start-Sleep -Milliseconds 500
        $adapter = Get-PhysicalAdapter $description
        $addresses = @(Get-NetIPAddress -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object AddressState -eq 'Preferred')
    } while (($adapter.Status -ne 'Up' -or $addresses.Count -eq 0) -and (Get-Date) -lt $deadline)
    if ($adapter.Status -ne 'Up' -or $addresses.Count -eq 0) {
        throw "Adapter did not become ready: $description"
    }
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

function Get-TransitionSnapshot([string]$stage) {
    $wifi = Get-PhysicalAdapter $WifiDescription
    $ethernet = Get-PhysicalAdapter $EthernetDescription
    $route = Find-NetRoute -RemoteIPAddress $ProbeAddress -ErrorAction SilentlyContinue
    [pscustomobject]@{
        stage = $stage
        capturedAt = (Get-Date).ToString('o')
        wifi = [pscustomobject]@{
            status = [string]$wifi.Status
            ifIndex = $wifi.ifIndex
            ipv4 = @(Get-NetIPAddress -InterfaceIndex $wifi.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
                Where-Object AddressState -eq 'Preferred' | Select-Object -ExpandProperty IPAddress)
        }
        ethernet = [pscustomobject]@{
            status = [string]$ethernet.Status
            ifIndex = $ethernet.ifIndex
            ipv4 = @(Get-NetIPAddress -InterfaceIndex $ethernet.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
                Where-Object AddressState -eq 'Preferred' | Select-Object -ExpandProperty IPAddress)
        }
        selectedInterfaceIndex = if ($route) { $route.InterfaceIndex } else { $null }
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
$wifi = Get-PhysicalAdapter $WifiDescription
$ethernet = Get-PhysicalAdapter $EthernetDescription
if ($wifi.Status -ne 'Up' -or $ethernet.Status -ne 'Up') {
    throw 'Wi-Fi and Ethernet must both be connected before validation'
}
if (@(Get-Process socks-proxy,socks-proxy-latest -ErrorAction SilentlyContinue).Count -ne 1) {
    throw 'Exactly one Socks Proxy application must be running before validation'
}

$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument "-NoProfile -ExecutionPolicy Bypass -File $recoveryScript"
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(3)
$principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
Register-ScheduledTask -TaskName $recoveryTask -Action $action -Trigger $trigger -Principal $principal -Force | Out-Null

$before = Get-TransitionSnapshot 'before'
$ethernetOnly = $null
$wifiOnly = $null
$restored = $null
$failure = $null
try {
    Disable-NetAdapter -InterfaceDescription $WifiDescription -Confirm:$false
    $ethernet = Wait-AdapterReady $EthernetDescription
    $ethernetOnly = Get-TransitionSnapshot 'ethernet-only'

    Enable-NetAdapter -InterfaceDescription $WifiDescription -Confirm:$false
    $wifi = Wait-AdapterReady $WifiDescription
    Disable-NetAdapter -InterfaceDescription $EthernetDescription -Confirm:$false
    $wifi = Wait-AdapterReady $WifiDescription
    $wifiOnly = Get-TransitionSnapshot 'wifi-only'
} catch {
    $failure = $_.Exception.Message
} finally {
    Enable-NetAdapter -InterfaceDescription $WifiDescription -Confirm:$false -ErrorAction SilentlyContinue
    Enable-NetAdapter -InterfaceDescription $EthernetDescription -Confirm:$false -ErrorAction SilentlyContinue
    try {
        [void](Wait-AdapterReady $WifiDescription)
        [void](Wait-AdapterReady $EthernetDescription)
        $restored = Get-TransitionSnapshot 'restored'
    } catch {
        if (-not $failure) { $failure = $_.Exception.Message }
    }
}

$passed = -not $failure -and
    $ethernetOnly.tcpProbe -and $ethernetOnly.selectedInterfaceIndex -eq $ethernet.ifIndex -and
    $wifiOnly.tcpProbe -and $wifiOnly.selectedInterfaceIndex -eq $wifi.ifIndex -and
    $restored.tcpProbe -and
    $ethernetOnly.applicationProcesses -eq 1 -and $wifiOnly.applicationProcesses -eq 1 -and
    $restored.applicationProcesses -eq 1

[pscustomobject]@{
    date = '2026-09-21'
    platform = 'Microsoft Windows 11 Pro 10.0.26200 x64'
    probe = "$ProbeAddress`:$ProbePort"
    passed = $passed
    failure = $failure
    before = $before
    ethernetOnly = $ethernetOnly
    wifiOnly = $wifiOnly
    restored = $restored
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $ResultPath -Encoding UTF8

if ($passed) {
    Unregister-ScheduledTask -TaskName $recoveryTask -Confirm:$false -ErrorAction SilentlyContinue
    exit 0
}
exit 1
