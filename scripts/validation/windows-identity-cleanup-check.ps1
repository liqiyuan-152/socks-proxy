$ErrorActionPreference = 'SilentlyContinue'
$userExists = $null -ne (Get-LocalUser -Name 'socksproxym0' -ErrorAction SilentlyContinue)
& schtasks.exe /Query /TN SocksProxyM0Identity 2>$null | Out-Null
$taskExists = $LASTEXITCODE -eq 0
$paths = @(
    (Join-Path $env:PUBLIC 'sp-i.ps1'),
    (Join-Path $env:PUBLIC 'sp-i-error.txt'),
    (Join-Path $env:PUBLIC 'sp-o'),
    'C:\Users\socksproxym0'
)
$remainingPaths = @($paths | Where-Object { Test-Path $_ })
$passed = (-not $userExists) -and (-not $taskExists) -and $remainingPaths.Count -eq 0
[ordered]@{
    passed = $passed
    userExists = $userExists
    taskExists = $taskExists
    remainingPaths = $remainingPaths
} | ConvertTo-Json -Depth 3
if (-not $passed) { exit 1 }
