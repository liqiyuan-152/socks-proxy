param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_process_core_smoke.exe' }
if (Get-NetTCPConnection -LocalPort 18110 -State Listen -ErrorAction SilentlyContinue) {
    throw 'Validation port 18110 is already in use'
}
$result = & $ValidationExe $CorePath $Workspace
if ($LASTEXITCODE -ne 0) { throw "Process core smoke failed with exit code $LASTEXITCODE" }
$parsed = $result | ConvertFrom-Json
if ($parsed.version -ne '1.14.1-socks-proxy.2' -or
    -not $parsed.ready -or
    -not $parsed.hidden_process -or
    -not $parsed.stopped -or
    -not $parsed.temporary_removed) {
    throw "Unexpected process core result: $result"
}
$result
