$ErrorActionPreference = 'Stop'
$profilePath = 'C:\Users\socksproxym0'
$profile = Get-CimInstance Win32_UserProfile | Where-Object { $_.LocalPath -eq $profilePath }
if ($profile) {
    $source = Join-Path $env:USERPROFILE 'socks-proxy-validation\windows-identity-system-cleanup.ps1'
    $payload = Join-Path $env:PUBLIC 'c.ps1'
    $marker = Join-Path $env:PUBLIC 'c.json'
    $task = 'SocksProxyM0SystemCleanup'
    Copy-Item -Force $source $payload
    Remove-Item -Force $marker -ErrorAction SilentlyContinue
    try {
        $action = "powershell.exe -NoP -NonI -EP Bypass -File $payload -Sid $($profile.SID) -ProfilePath $profilePath -Marker $marker"
        & schtasks.exe /Create /TN $task /SC ONCE /ST 23:59 /TR $action /RU SYSTEM /RL HIGHEST /F | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "SYSTEM cleanup task create failed: $LASTEXITCODE" }
        & schtasks.exe /Run /TN $task | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "SYSTEM cleanup task run failed: $LASTEXITCODE" }
        for ($attempt = 0; $attempt -lt 30; $attempt++) {
            if (Test-Path $marker) { break }
            Start-Sleep -Milliseconds 500
        }
        if (-not (Test-Path $marker)) { throw 'SYSTEM cleanup task timed out' }
        Get-Content -Raw $marker
    } finally {
        $ErrorActionPreference = 'SilentlyContinue'
        & schtasks.exe /Delete /TN $task /F 2>$null | Out-Null
        Remove-Item -Force $payload, $marker -ErrorAction SilentlyContinue
    }
} else {
    '{"profileExists":false,"directoryExists":false}'
}
