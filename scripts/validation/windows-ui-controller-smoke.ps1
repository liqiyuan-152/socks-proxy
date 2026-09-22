param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_ui_controller_smoke.exe' }
foreach ($port in 18121, 18122, 18124, 18125) {
    if (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) {
        throw "Validation port is already in use: $port"
    }
}
$result = & $ValidationExe $CorePath $Workspace
if ($LASTEXITCODE -ne 0) { throw "UI controller smoke failed with exit code $LASTEXITCODE" }
$parsed = $result | ConvertFrom-Json
if (-not $parsed.rules_switched_without_confirmation -or -not $parsed.rules_path_a -or
    -not $parsed.rule_change_confirmation -or
    -not $parsed.proxy_confirmation -or -not $parsed.path_b -or
    -not $parsed.global_switched_without_confirmation -or -not $parsed.global_path_b -or
    -not $parsed.rule_log -or -not $parsed.unknown_log -or -not $parsed.failure_log -or
    -not $parsed.clear_kept_proxy -or
    -not $parsed.failure_visible -or -not $parsed.failure_kept_direct) {
    throw "Unexpected UI controller result: $result"
}
$result
