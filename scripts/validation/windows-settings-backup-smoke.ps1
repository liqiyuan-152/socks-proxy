param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
if (-not $ValidationExe) {
    $ValidationExe = Join-Path $Workspace 'windows_settings_backup_smoke.exe'
}
$result = & $ValidationExe
if ($LASTEXITCODE -ne 0) {
    throw "Settings backup smoke failed with exit code $LASTEXITCODE"
}
$parsed = $result | ConvertFrom-Json
if (-not $parsed.first_launch_direct -or -not $parsed.startup_disabled -or
    -not $parsed.secret_free -or -not $parsed.preview_missing_credentials -or
    -not $parsed.import_applied_direct) {
    throw "Unexpected settings backup result: $result"
}
$result
