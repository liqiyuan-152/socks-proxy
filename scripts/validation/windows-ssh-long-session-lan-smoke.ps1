param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = '',
    [string]$Target = '10.20.30.40',
    [string]$FixtureTarget = '10.168.1.136',
    [string]$TargetUser = 'liqiyuan',
    [int]$TargetPort = 22,
    [string]$PrivateKey = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_ssh_long_session_smoke.exe' }
if (-not $PrivateKey) { $PrivateKey = Join-Path $Workspace 'ssh-long-session-key' }
$upstreams = @()

try {
    if (-not (Test-Path $PrivateKey)) { throw "SSH fixture key is missing: $PrivateKey" }
    $baselineOut = Join-Path $Workspace 'ssh-baseline.out'
    $baselineErr = Join-Path $Workspace 'ssh-baseline.err'
    Remove-Item $baselineOut,$baselineErr -Force -ErrorAction SilentlyContinue
    $baseline = Start-Process ssh.exe -ArgumentList @(
        '-i', ('"' + $PrivateKey + '"'), '-o', 'BatchMode=yes',
        '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=NUL',
        '-o', 'ConnectTimeout=5', '-p', $TargetPort,
        "${TargetUser}@${FixtureTarget}", 'echo BASELINE'
    ) -PassThru -RedirectStandardOutput $baselineOut -RedirectStandardError $baselineErr
    if (-not $baseline.WaitForExit(10000)) {
        Stop-Process -Id $baseline.Id -Force
        throw 'Direct SSH fixture baseline timed out'
    }
    $baseline.WaitForExit()
    if ((Get-Content $baselineOut -Raw) -notmatch 'BASELINE') {
        throw "Direct SSH fixture baseline failed: $(Get-Content $baselineErr -Raw)"
    }
    foreach ($port in 18141, 18142) {
        if (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) {
            throw "Validation port is busy: $port"
        }
        $configPath = Join-Path $Workspace "ssh-upstream-$port.json"
        $logPath = Join-Path $Workspace "ssh-upstream-$port.log"
        Remove-Item $configPath,$logPath -Force -ErrorAction SilentlyContinue
        $config = @{
            log = @{ level = 'info'; timestamp = $false }
            inbounds = @(@{ type = 'mixed'; tag = 'upstream'; listen = '127.0.0.1'; listen_port = $port })
            outbounds = @(@{ type = 'direct'; tag = 'direct' })
            route = @{
                rules = @(@{
                    inbound = @('upstream')
                    action = 'route'
                    outbound = 'direct'
                    override_address = $FixtureTarget
                    override_port = $TargetPort
                })
                final = 'direct'
            }
        } | ConvertTo-Json -Depth 10
        [IO.File]::WriteAllText($configPath, $config, (New-Object Text.UTF8Encoding($false)))
        $upstreams += Start-Process $CorePath -ArgumentList @('run', '-c', ('"' + $configPath + '"')) `
            -PassThru -WindowStyle Hidden -RedirectStandardError $logPath
    }
    Start-Sleep -Seconds 1
    $raw = & $ValidationExe $CorePath $Workspace $Target $PrivateKey $TargetUser $TargetPort
    if ($LASTEXITCODE -ne 0) { throw "SSH long-session helper failed with exit code $LASTEXITCODE" }
    $result = $raw | ConvertFrom-Json
    if (-not $result.rules_confirmation -or -not $result.proxy_confirmation -or
        -not $result.long_active_before_switch -or -not $result.new_ssh_after_switch -or
        -not $result.upstream_a_seen -or -not $result.upstream_b_seen -or
        -not $result.explicit_direct) {
        throw "Unexpected SSH long-session result: $raw"
    }
    $raw
} finally {
    foreach ($process in $upstreams) {
        $process.Refresh()
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
}
