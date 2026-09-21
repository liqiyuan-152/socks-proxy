param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_application_switch_smoke.exe' }
foreach ($port in 18101..18105) {
    if (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) {
        throw "Validation port is already in use: $port"
    }
}
$result = & $ValidationExe $CorePath $Workspace
if ($LASTEXITCODE -ne 0) { throw "Application switch smoke failed with exit code $LASTEXITCODE" }
$parsed = $result | ConvertFrom-Json
if (-not $parsed.path_a -or -not $parsed.path_b -or -not $parsed.rollback_b -or
    -not $parsed.restart_warning -or -not $parsed.direct_stopped) {
    throw "Unexpected switch result: $result"
}
$result
