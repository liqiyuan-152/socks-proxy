param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = '',
    [string]$ValidationExe = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not $ValidationExe) { $ValidationExe = Join-Path $Workspace 'windows_ssh_long_session_smoke.exe' }
$dockerDesktop = 'F:\Docker\Docker\Docker Desktop.exe'
$container = 'socks-proxy-ssh-fixture'
$image = 'lscr.io/linuxserver/openssh-server:latest'
$imageTar = Join-Path $Workspace 'socks-proxy-openssh-server.tar'
$key = Join-Path $Workspace 'ssh-long-session-key'
$dockerConfig = Join-Path $Workspace 'ssh-docker-config'
$upstreams = @()

function Wait-Docker {
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    while ([DateTime]::UtcNow -lt $deadline) {
        & cmd.exe /d /c "docker info --format {{.ServerVersion}} >nul 2>&1"
        if ($LASTEXITCODE -eq 0) { return }
        Start-Sleep -Seconds 2
    }
    throw 'Docker Desktop did not become ready'
}

try {
    New-Item -ItemType Directory -Path $dockerConfig -Force | Out-Null
    [IO.File]::WriteAllText(
        (Join-Path $dockerConfig 'config.json'),
        '{"auths":{}}',
        (New-Object Text.UTF8Encoding($false))
    )
    $env:DOCKER_CONFIG = $dockerConfig
    if (-not (Get-Service com.docker.service).Status.Equals('Running')) {
        Start-Service com.docker.service
    }
    & cmd.exe /d /c "docker info --format {{.ServerVersion}} >nul 2>&1"
    if ($LASTEXITCODE -ne 0) {
        Start-Process -FilePath $dockerDesktop -ArgumentList '--minimized'
        Wait-Docker
    }
    if (-not (Test-Path $imageTar)) { throw "SSH fixture image tar is missing: $imageTar" }
    & docker.exe load -i $imageTar | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'SSH fixture image import failed' }
    & cmd.exe /d /c "docker rm -f $container >nul 2>&1"
    Remove-Item $key, ($key + '.pub') -Force -ErrorAction SilentlyContinue
    $keygen = Start-Process -FilePath 'ssh-keygen.exe' `
        -ArgumentList @('-q', '-t', 'ed25519', '-N', '""', '-f', ('"' + $key + '"')) `
        -PassThru -Wait -NoNewWindow
    if ($keygen.ExitCode -ne 0) { throw 'ssh-keygen failed' }
    $publicKey = (Get-Content ($key + '.pub') -Raw).Trim()
    $containerId = & docker.exe --config $dockerConfig run -d --rm --name $container `
        -e PUID=1000 -e PGID=1000 -e TZ=Etc/UTC `
        -e USER_NAME=fixture -e PASSWORD_ACCESS=false `
        -e "PUBLIC_KEY=$publicKey" `
        $image
    if ($LASTEXITCODE -ne 0) { throw 'SSH fixture container failed to start' }
    $target = (& docker.exe --config $dockerConfig inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' $container).Trim()
    if (-not $target) { throw 'SSH fixture has no container address' }
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
        $reachable = Test-NetConnection -ComputerName $target -Port 2222 -InformationLevel Quiet -WarningAction SilentlyContinue
        if (-not $reachable) { Start-Sleep -Seconds 1 }
    } until ($reachable -or [DateTime]::UtcNow -ge $deadline)
    if (-not $reachable) { throw "SSH fixture is unreachable at ${target}:2222" }

    foreach ($port in 18141, 18142) {
        if (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) {
            throw "Validation port is busy: $port"
        }
        $configPath = Join-Path $Workspace "ssh-upstream-$port.json"
        $logPath = Join-Path $Workspace "ssh-upstream-$port.log"
        $config = @{
            log = @{ level = 'info'; timestamp = $false }
            inbounds = @(@{ type = 'mixed'; tag = 'upstream'; listen = '127.0.0.1'; listen_port = $port })
            outbounds = @(@{ type = 'direct'; tag = 'direct' })
            route = @{ final = 'direct' }
        } | ConvertTo-Json -Depth 10
        [IO.File]::WriteAllText($configPath, $config, (New-Object Text.UTF8Encoding($false)))
        $upstreams += Start-Process $CorePath -ArgumentList @('run', '-c', ('"' + $configPath + '"')) `
            -PassThru -WindowStyle Hidden -RedirectStandardError $logPath
    }
    Start-Sleep -Seconds 1
    $raw = & $ValidationExe $CorePath $Workspace $target $key 'fixture' 2222
    if ($LASTEXITCODE -ne 0) { throw "SSH long-session helper failed with exit code $LASTEXITCODE" }
    $jsonLine = @($raw | Where-Object { $_ -match '^\s*\{' } | Select-Object -Last 1)
    if (-not $jsonLine) { throw "SSH long-session helper did not return JSON: $($raw -join [Environment]::NewLine)" }
    $result = $jsonLine | ConvertFrom-Json
    if (-not $result.rules_switched_without_confirmation -or -not $result.proxy_confirmation -or
        -not $result.long_active_before_switch -or -not $result.new_ssh_after_switch -or
        $result.proxy_rule_events -lt 2 -or -not $result.explicit_direct) {
        throw "Unexpected SSH long-session result: $raw"
    }
    $raw
} finally {
    foreach ($process in $upstreams) {
        $process.Refresh()
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
    & cmd.exe /d /c "docker rm -f $container >nul 2>&1"
    Remove-Item $key, ($key + '.pub') -Force -ErrorAction SilentlyContinue
    Remove-Item $dockerConfig -Recurse -Force -ErrorAction SilentlyContinue
}
