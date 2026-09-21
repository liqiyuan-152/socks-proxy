param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (-not $CorePath) {
    $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe'
}
if (-not $ValidationExe) {
    $ValidationExe = Join-Path $Workspace 'windows_core_config_smoke.exe'
}
if (-not (Test-Path -LiteralPath $CorePath -PathType Leaf)) {
    throw "Locked core missing: $CorePath"
}
if (-not (Test-Path -LiteralPath $ValidationExe -PathType Leaf)) {
    throw "Validation executable missing: $ValidationExe"
}

$result = & $ValidationExe $CorePath $Workspace
if ($LASTEXITCODE -ne 0) {
    throw "Core configuration validation failed with exit code $LASTEXITCODE"
}
$parsed = $result | ConvertFrom-Json
if (-not $parsed.config_check -or
    -not $parsed.acl_current_user_and_system -or
    $parsed.secret_in_args -or
    -not $parsed.temporary_removed) {
    throw "Unexpected validation result: $result"
}
$result
