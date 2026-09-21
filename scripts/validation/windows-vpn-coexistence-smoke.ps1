param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_ui_controller_smoke.exe' }
$openVpn = 'C:\Program Files\OpenVPN\bin\openvpn.exe'
$config = Join-Path $Workspace 'socks-proxy-vpn-client.ovpn'
$log = Join-Path $Workspace 'socks-proxy-vpn-client.log'
$vpn = $null

try {
    Remove-Item $log -Force -ErrorAction SilentlyContinue
    $vpn = Start-Process $openVpn -ArgumentList @('--config', ('"' + $config + '"')) `
        -WorkingDirectory $Workspace -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $log -RedirectStandardError (Join-Path $Workspace 'socks-proxy-vpn-client.err')
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        $vpn.Refresh()
        if ($vpn.HasExited) {
            throw "OpenVPN exited before ready: $(Get-Content (Join-Path $Workspace 'socks-proxy-vpn-client.err') -Raw)"
        }
        if ((Get-Content $log -Raw -ErrorAction SilentlyContinue) -match 'Initialization Sequence Completed') { break }
        Start-Sleep -Milliseconds 250
    }
    $adapter = Get-NetAdapter -Name 'OpenVPN TAP-Windows6' -ErrorAction Stop
    $vpnRoute = Get-NetRoute -DestinationPrefix '203.0.113.0/24' -ErrorAction Stop |
        Where-Object InterfaceIndex -eq $adapter.ifIndex |
        Select-Object -First 1
    if ($adapter.Status -ne 'Up' -or -not $vpnRoute) { throw 'OpenVPN adapter or route is not active' }

    $applicationRaw = & $ValidationExe $CorePath $Workspace
    if ($LASTEXITCODE -ne 0) { throw "Application validation failed with exit code $LASTEXITCODE" }
    $application = $applicationRaw | ConvertFrom-Json
    if (-not $application.rules_path_a -or -not $application.path_b -or
        -not $application.global_path_b -or -not $application.failure_kept_direct) {
        throw "Application validation result was incomplete: $applicationRaw"
    }

    $vpn.Refresh()
    $adapterAfter = Get-NetAdapter -Name 'OpenVPN TAP-Windows6' -ErrorAction Stop
    $vpnRouteAfter = Get-NetRoute -DestinationPrefix '203.0.113.0/24' -ErrorAction Stop |
        Where-Object InterfaceIndex -eq $adapterAfter.ifIndex |
        Select-Object -First 1
    $appTunRemoved = -not [bool](Get-NetAdapter -Name 'socks-proxy-tun-v1' -ErrorAction SilentlyContinue)
    if ($vpn.HasExited -or $adapterAfter.Status -ne 'Up' -or -not $vpnRouteAfter -or -not $appTunRemoved) {
        throw 'Application lifecycle changed VPN-owned resources or left its TUN behind'
    }
    [pscustomobject]@{
        vpnConnectedBeforeApp = $true
        vpnProcessSurvived = $true
        vpnAdapterSurvived = $true
        vpnRouteSurvived = $true
        rulesPath = [bool]$application.rules_path_a
        proxySwitchPath = [bool]$application.path_b
        globalPath = [bool]$application.global_path_b
        applicationTunRemoved = $appTunRemoved
    } | ConvertTo-Json -Compress
} finally {
    if ($vpn) {
        $vpn.Refresh()
        if (-not $vpn.HasExited) { Stop-Process -Id $vpn.Id -Force }
    }
}
