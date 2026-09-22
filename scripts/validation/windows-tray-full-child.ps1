param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation')
)

$ErrorActionPreference = 'Stop'

try {
    & (Join-Path $Workspace 'windows-tray-full-interaction-smoke.ps1') `
        -Workspace $Workspace `
        -Application (Join-Path $Workspace 'socks-proxy.exe') `
        -CorePath (Join-Path $Workspace 'sing-box.exe') `
        -ResultPath (Join-Path $Workspace 'tray-full-results.json')
} catch {
    ($_ | Out-String) | Set-Content -LiteralPath (Join-Path $Workspace 'tray-full-error.txt') -Encoding UTF8
    exit 1
}
