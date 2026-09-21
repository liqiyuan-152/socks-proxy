param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_host_crash_smoke.exe' }
$ready = Join-Path $Workspace 'host-crash-ready.json'
$fixtureConfig = Join-Path $Workspace 'host-crash-upstream.json'
$fixtureLog = Join-Path $Workspace 'host-crash-upstream.log'
$hostOut = Join-Path $Workspace 'host-crash-host.out'
$hostErr = Join-Path $Workspace 'host-crash-host.err'
Remove-Item $ready, $hostOut, $hostErr -Force -ErrorAction SilentlyContinue
if (Get-NetTCPConnection -LocalPort 18141 -State Listen -ErrorAction SilentlyContinue) {
    throw 'Port 18141 is busy'
}

$fixture = @{
    log = @{ level = 'info' }
    inbounds = @(@{ type = 'mixed'; tag = 'upstream'; listen = '127.0.0.1'; listen_port = 18141 })
    outbounds = @(@{ type = 'direct'; tag = 'direct' })
    route = @{ final = 'direct' }
} | ConvertTo-Json -Depth 10
[IO.File]::WriteAllText($fixtureConfig, $fixture, (New-Object Text.UTF8Encoding($false)))
$upstream = Start-Process $CorePath -ArgumentList @('run', '-c', ('"' + $fixtureConfig + '"')) -PassThru -WindowStyle Hidden -RedirectStandardError $fixtureLog
$hostProcess = $null
try {
    Start-Sleep -Milliseconds 500
    $hostProcess = Start-Process $ValidationExe -ArgumentList @('run', ('"' + $CorePath + '"'), ('"' + $Workspace + '"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput $hostOut -RedirectStandardError $hostErr
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while (-not (Test-Path $ready)) {
        $hostProcess.Refresh()
        if ($hostProcess.HasExited) { throw "Host exited before ready: $(Get-Content $hostErr -Raw)" }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Timed out waiting for host readiness' }
        Start-Sleep -Milliseconds 100
    }
    $readyState = Get-Content $ready -Raw | ConvertFrom-Json
    $child = Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $hostProcess.Id -and $_.Name -like 'sing-box*' } | Select-Object -First 1
    if (-not $child) { throw 'Managed core child was not found' }
    $childPid = [int]$child.ProcessId
    $tunRoutePresent = [bool](Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | Where-Object InterfaceAlias -eq 'socks-proxy-tun-v1')

    Stop-Process -Id $hostProcess.Id -Force
    $hostProcess.WaitForExit()
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (Get-Process -Id $childPid -ErrorAction SilentlyContinue) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Managed core survived host termination' }
        Start-Sleep -Milliseconds 100
    }
    $tunRemoved = -not (Get-NetAdapter -Name 'socks-proxy-tun-v1' -ErrorAction SilentlyContinue)
    $recoveryRaw = & $ValidationExe recover-direct $CorePath $Workspace
    if ($LASTEXITCODE -ne 0) { throw "Recovery launch failed with exit code $LASTEXITCODE" }
    $recovery = $recoveryRaw | ConvertFrom-Json
    $result = [pscustomobject]@{
        hostTerminated = $true
        childTerminatedByJob = $true
        tunWasActive = $tunRoutePresent
        tunRemovedAfterCrash = $tunRemoved
        trafficMayBeDirectBoundary = $true
        nextLaunchRestoredRules = [bool]$recovery.next_launch_restored_rules
        explicitDirect = [bool]$recovery.explicit_direct
        lastModeDirect = [bool]$recovery.last_mode_direct
    }
    if (-not $result.tunWasActive -or -not $result.tunRemovedAfterCrash -or
        -not $result.nextLaunchRestoredRules -or -not $result.explicitDirect -or
        -not $result.lastModeDirect) {
        throw "Unexpected host crash result: $($result | ConvertTo-Json -Compress)"
    }
    $result | ConvertTo-Json -Compress
} finally {
    if ($hostProcess) { $hostProcess.Refresh(); if (-not $hostProcess.HasExited) { Stop-Process -Id $hostProcess.Id -Force } }
    $upstream.Refresh(); if (-not $upstream.HasExited) { Stop-Process -Id $upstream.Id -Force }
}
