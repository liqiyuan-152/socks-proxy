param([string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'))
$ErrorActionPreference = 'Stop'

$workspacePrefix = [IO.Path]::GetFullPath($Workspace).TrimEnd('\') + '\'
$coreProcesses = @(
    Get-CimInstance Win32_Process -Filter "Name = 'sing-box.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase) }
)
$ports = @(18080, 18081, 18082, 18083, 18091, 18092, 18093, 18094, 18100, 18101, 18103, 18109, 18122, 18400, 18401, 15353, 15354, 15355, 15356, 15357, 15358)
$tcpListeners = @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object { $ports -contains $_.LocalPort })
$udpListeners = @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue | Where-Object { $ports -contains $_.LocalPort })
$adapters = @(Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object { $_.Name -in @('socks-proxy-tun-v1', 'socks-proxy-m0', 'socks-proxy-fakeip') })
$routes = @(
    Get-NetRoute -ErrorAction SilentlyContinue |
        Where-Object { $_.DestinationPrefix -in @('198.18.0.0/15', '198.19.0.0/24') }
)
$result = [pscustomobject]@{
    CoreProcesses = $coreProcesses.Count
    TcpListeners = $tcpListeners.Count
    UdpListeners = $udpListeners.Count
    TestAdapters = $adapters.Count
    TestRoutes = $routes.Count
}
$result | ConvertTo-Json
if ($coreProcesses.Count -or $tcpListeners.Count -or $udpListeners.Count -or $adapters.Count -or $routes.Count) {
    throw 'Validation resources remain active'
}
