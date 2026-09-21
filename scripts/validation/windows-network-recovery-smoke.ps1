param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_network_recovery_smoke.exe' }
if (-not (Test-Path -LiteralPath $ValidationExe -PathType Leaf)) { throw "Validation executable missing: $ValidationExe" }
$result = & $ValidationExe $Workspace
if ($LASTEXITCODE -ne 0) { throw "Network recovery validation failed with exit code $LASTEXITCODE" }
$parsed = $result | ConvertFrom-Json
if (-not $parsed.external_setting_preserved -or -not $parsed.restore_retry -or
    -not $parsed.dns_flush_retry -or -not $parsed.application_cache_notice -or
    -not $parsed.journal_cleared -or -not $parsed.last_mode_unchanged) {
    throw "Unexpected validation result: $result"
}
$result | Set-Content (Join-Path $Workspace 'network-recovery-results.json')
$result
