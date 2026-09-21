param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (-not $CorePath) {
    $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe'
}
if (-not (Test-Path -LiteralPath $CorePath -PathType Leaf)) {
    throw "Locked core missing: $CorePath"
}

$owned = [Collections.Generic.List[object]]::new()
$results = [Collections.Generic.List[object]]::new()
$utf8 = New-Object Text.UTF8Encoding($false)

function Start-Core($Name, $Config) {
    $path = Join-Path $Workspace "$Name.json"
    [IO.File]::WriteAllText($path, ($Config | ConvertTo-Json -Depth 20), $utf8)
    & $CorePath check -c $path
    if ($LASTEXITCODE -ne 0) { throw "Invalid config: $Name" }
    $process = Start-Process $CorePath -ArgumentList @('run', '-c', ('"' + $path + '"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Workspace "$Name.stdout.log") -RedirectStandardError (Join-Path $Workspace "$Name.stderr.log")
    $owned.Add($process)
    $watch = "Start-Sleep -Seconds 120; Get-Process -Id $($process.Id) -ErrorAction SilentlyContinue | Where-Object { `$_.StartTime.ToUniversalTime().Ticks -eq $($process.StartTime.ToUniversalTime().Ticks) } | Stop-Process -Force"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($watch))
    Start-Process powershell -ArgumentList @('-NoProfile', '-NonInteractive', '-EncodedCommand', $encoded) -WindowStyle Hidden | Out-Null
    Start-Sleep -Milliseconds 700
    $process.Refresh()
    if ($process.HasExited) { throw "Core exited: $Name" }
    return $process
}

function Read-Headers($Stream) {
    $bytes = [Collections.Generic.List[byte]]::new()
    while ($bytes.Count -lt 8192) {
        $value = $Stream.ReadByte()
        if ($value -lt 0) { break }
        $bytes.Add([byte]$value)
        if ($bytes.Count -ge 4 -and [Text.Encoding]::ASCII.GetString($bytes.ToArray()).EndsWith("`r`n`r`n")) { break }
    }
    return [Text.Encoding]::ASCII.GetString($bytes.ToArray())
}

function Probe-Http($Domain) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $body = & curl.exe --silent --show-error --fail --max-time 4 --noproxy no-bypass.invalid --proxy http://127.0.0.1:18103 "http://${Domain}:18100/" 2>$null
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $old
    return [pscustomobject]@{ Success = ($exitCode -eq 0 -and ($body -join '') -eq 'managed-routing'); ExitCode = $exitCode }
}

function Probe-Ssh($Domain) {
    $client = [Net.Sockets.TcpClient]::new()
    $success = $false
    try {
        $client.Connect('127.0.0.1', 18103)
        $stream = $client.GetStream()
        $stream.ReadTimeout = 3000
        $request = [Text.Encoding]::ASCII.GetBytes("CONNECT ${Domain}:22 HTTP/1.1`r`nHost: ${Domain}:22`r`n`r`n")
        $stream.Write($request, 0, $request.Length)
        $headers = Read-Headers $stream
        if ($headers -match '^HTTP/1.[01] 200') {
            $reader = [IO.StreamReader]::new($stream)
            $success = $reader.ReadLine().StartsWith('SSH-2.0-managed-routing')
        }
    } catch {
        $success = $false
    } finally {
        $client.Dispose()
    }
    return $success
}

foreach ($port in @(18100, 18101, 18103, 18109, 18122, 15356, 15357)) {
    if ((Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) -or (Get-NetUDPEndpoint -LocalPort $port -ErrorAction SilentlyContinue)) {
        throw "Port busy: $port"
    }
}

$httpJob = Start-Job -ScriptBlock {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 18100)
    $listener.Start()
    try {
        while ($true) {
            if (-not $listener.Pending()) { Start-Sleep -Milliseconds 50; continue }
            $client = $listener.AcceptTcpClient()
            try {
                $stream = $client.GetStream()
                $stream.ReadTimeout = 3000
                $buffer = New-Object byte[] 8192
                $null = $stream.Read($buffer, 0, $buffer.Length)
                $body = 'managed-routing'
                $reply = "HTTP/1.1 200 OK`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n$body"
                $bytes = [Text.Encoding]::ASCII.GetBytes($reply)
                $stream.Write($bytes, 0, $bytes.Length)
            } finally { $client.Dispose() }
        }
    } finally { $listener.Stop() }
}
$sshJob = Start-Job -ScriptBlock {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 18122)
    $listener.Start()
    try {
        while ($true) {
            if (-not $listener.Pending()) { Start-Sleep -Milliseconds 50; continue }
            $client = $listener.AcceptTcpClient()
            try {
                $stream = $client.GetStream()
                $bytes = [Text.Encoding]::ASCII.GetBytes("SSH-2.0-managed-routing`r`n")
                $stream.Write($bytes, 0, $bytes.Length)
            } finally { $client.Dispose() }
        }
    } finally { $listener.Stop() }
}
Start-Sleep -Seconds 2

try {
    $hosts = @{
        'domain.fixture.invalid' = @('127.0.0.1')
        'direct.fixture.invalid' = @('127.0.0.1')
        'ipv4.fixture.invalid' = @('203.0.113.10')
        'single-ip.fixture.invalid' = @('203.0.113.30')
        'range.fixture.invalid' = @('203.0.113.35')
        'multi.fixture.invalid' = @('127.0.0.1', '203.0.113.11')
        'ipv6.fixture.invalid' = @('2001:db8::10')
        'private.fixture.invalid' = @('10.23.45.67')
        'all-private.fixture.invalid' = @('10.1.2.3', 'fd00::1234')
        'public.fixture.invalid' = @('203.0.113.20')
        'mixed.fixture.invalid' = @('10.1.2.3', '203.0.113.21')
        'ssh.fixture.invalid' = @('127.0.0.1')
    }
    $fixture = @{
        log = @{ level = 'debug' }
        inbounds = @(
            @{ type = 'mixed'; tag = 'proxy-in'; listen = '127.0.0.1'; listen_port = 18101 },
            @{ type = 'direct'; tag = 'dns-in'; listen = '127.0.0.1'; listen_port = 15356 }
        )
        outbounds = @(@{ type = 'direct'; tag = 'direct' })
        dns = @{ servers = @(@{ type = 'hosts'; tag = 'hosts'; predefined = $hosts }); final = 'hosts' }
        route = @{ default_domain_resolver = 'hosts'; rules = @(@{ inbound = @('dns-in'); action = 'hijack-dns' }); final = 'direct' }
    }
    $null = Start-Core 'managed-routing-fixture' $fixture

    $cases = @(
        @{ Name = 'browser-domain-priority'; Domain = 'domain.fixture.invalid'; Kind = 'domain'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'ssh-domain-priority'; Domain = 'ssh.fixture.invalid'; Kind = 'ssh-domain'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'browser-ipv4-rule'; Domain = 'ipv4.fixture.invalid'; Kind = 'ip'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'browser-single-ip-rule'; Domain = 'single-ip.fixture.invalid'; Kind = 'single-ip'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'browser-range-rule'; Domain = 'range.fixture.invalid'; Kind = 'range'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'browser-multi-any-rule'; Domain = 'multi.fixture.invalid'; Kind = 'ip'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'browser-ipv6-rule'; Domain = 'ipv6.fixture.invalid'; Kind = 'ip'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'browser-remote-private-rule'; Domain = 'private.fixture.invalid'; Kind = 'private-ip'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'unmatched-direct-with-dead-proxy'; Domain = 'direct.fixture.invalid'; Kind = 'direct'; Expected = $true; Route = 'direct'; DeadProxy = $true; DnsPort = 15356 },
        @{ Name = 'domain-no-fallback-dead-proxy'; Domain = 'domain.fixture.invalid'; Kind = 'domain'; Expected = $false; Route = 'proxy'; DeadProxy = $true; DnsPort = 15356 },
        @{ Name = 'proxy-resolution-failure-no-fallback'; Domain = 'proxy-resolution-fail.fixture.invalid'; Kind = 'proxy-resolution-fail'; Expected = $false; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'direct-dns-unavailable'; Domain = 'direct.fixture.invalid'; Kind = 'direct'; Expected = $false; Route = ''; DeadProxy = $false; DnsPort = 15357 },
        @{ Name = 'global-all-private-direct'; Domain = 'all-private.fixture.invalid'; Kind = 'global'; Expected = $true; Route = 'direct'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'global-public-proxy'; Domain = 'public.fixture.invalid'; Kind = 'global'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 },
        @{ Name = 'global-mixed-proxy'; Domain = 'mixed.fixture.invalid'; Kind = 'global'; Expected = $true; Route = 'proxy'; DeadProxy = $false; DnsPort = 15356 }
    )

    foreach ($case in $cases) {
        $proxyPort = if ($case.DeadProxy) { 18109 } else { 18101 }
        $routeRules = [Collections.Generic.List[object]]::new()
        if ($case.Kind -eq 'domain' -or $case.Kind -eq 'ssh-domain' -or $case.Kind -eq 'proxy-resolution-fail') {
            $targetPort = if ($case.Kind -eq 'ssh-domain') { 18122 } else { 18100 }
            if ($case.Kind -eq 'proxy-resolution-fail') {
                $routeRules.Add(@{ domain = @($case.Domain); action = 'route'; outbound = 'proxy' })
            } else {
                $routeRules.Add(@{ domain = @($case.Domain); action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = $targetPort })
            }
        } else {
            $routeRules.Add(@{ network = @('tcp', 'udp'); action = 'resolve'; server = 'real'; timeout = '1s' })
            if ($case.Kind -eq 'global') {
                $routeRules.Add(@{ ip_all_private = $true; action = 'route'; outbound = 'direct'; override_address = '127.0.0.1'; override_port = 18100 })
                $routeRules.Add(@{ action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = 18100 })
            } else {
                if ($case.Kind -eq 'ip') {
                    $routeRules.Add(@{ ip_cidr = @('203.0.113.0/24', '2001:db8::/32'); action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = 18100 })
                }
                if ($case.Kind -eq 'single-ip') {
                    $routeRules.Add(@{ ip_cidr = @('203.0.113.30/32'); action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = 18100 })
                }
                if ($case.Kind -eq 'range') {
                    # 203.0.113.32-203.0.113.47 is the exact CIDR expansion of this fixture range.
                    $routeRules.Add(@{ ip_cidr = @('203.0.113.32/28'); action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = 18100 })
                }
                if ($case.Kind -eq 'private-ip') {
                    $routeRules.Add(@{ ip_cidr = @('10.23.45.67/32'); action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = 18100 })
                }
                $routeRules.Add(@{ action = 'route'; outbound = 'direct'; override_address = '127.0.0.1'; override_port = 18100 })
            }
        }
        $config = @{
            log = @{ level = 'debug' }
            inbounds = @(@{ type = 'mixed'; listen = '127.0.0.1'; listen_port = 18103 })
            outbounds = @(
                @{ type = 'direct'; tag = 'direct' },
                @{ type = 'socks'; tag = 'proxy'; server = '127.0.0.1'; server_port = $proxyPort; version = '5' }
            )
            dns = @{ servers = @(@{ type = 'udp'; tag = 'real'; server = '127.0.0.1'; server_port = $case.DnsPort }); final = 'real' }
            route = @{ default_domain_resolver = @{ server = 'real' }; rules = $routeRules.ToArray(); final = 'direct' }
        }
        $process = Start-Core $case.Name $config
        if ($case.Kind -eq 'ssh-domain') {
            $actual = Probe-Ssh $case.Domain
            $exitCode = if ($actual) { 0 } else { 1 }
        } else {
            $probe = Probe-Http $case.Domain
            $actual = $probe.Success
            $exitCode = $probe.ExitCode
        }
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
        $log = Get-Content (Join-Path $Workspace "$($case.Name).stderr.log") -Raw
        $routeEvidence = if ($case.Route -eq 'proxy') {
            $log.Contains('outbound/socks[proxy]: outbound connection')
        } elseif ($case.Route -eq 'direct') {
            $log.Contains('outbound/direct[direct]: outbound connection')
        } else {
            -not $log.Contains('outbound/socks[proxy]: outbound connection')
        }
        $results.Add([pscustomobject]@{
            Name = $case.Name
            Pass = ($actual -eq $case.Expected -and $routeEvidence)
            ExpectedSuccess = $case.Expected
            ActualSuccess = $actual
            ExpectedRoute = $case.Route
            RouteEvidence = $routeEvidence
            ExitCode = $exitCode
        })
    }
} finally {
    foreach ($process in $owned) {
        $process.Refresh()
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
    }
    Stop-Job $httpJob, $sshJob -ErrorAction SilentlyContinue
    Remove-Job $httpJob, $sshJob -Force -ErrorAction SilentlyContinue
    $results | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $Workspace 'managed-routing-results.json')
}

$results | ConvertTo-Json -Depth 5
if (@($results | Where-Object { -not $_.Pass }).Count) { throw 'Managed routing smoke failed' }
