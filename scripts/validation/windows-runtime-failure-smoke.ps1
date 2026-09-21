param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_runtime_failure_smoke.exe' }
if (-not (Test-Path -LiteralPath $CorePath -PathType Leaf)) { throw "Locked core missing: $CorePath" }
if (-not (Test-Path -LiteralPath $ValidationExe -PathType Leaf)) { throw "Validation executable missing: $ValidationExe" }
if (Get-NetTCPConnection -LocalPort 18111 -State Listen -ErrorAction SilentlyContinue) { throw 'Port 18111 is busy' }

$result = & $ValidationExe $CorePath $Workspace
if ($LASTEXITCODE -ne 0) { throw "Runtime failure validation failed with exit code $LASTEXITCODE" }
$parsed = $result | ConvertFrom-Json
if (-not $parsed.exit_detected -or -not $parsed.error_visible -or
    -not $parsed.traffic_may_be_direct -or -not $parsed.last_mode_preserved -or
    -not $parsed.retry_rules -or -not $parsed.direct_recovery) {
    throw "Unexpected validation result: $result"
}
$result | Set-Content (Join-Path $Workspace 'runtime-failure-results.json')
$result
