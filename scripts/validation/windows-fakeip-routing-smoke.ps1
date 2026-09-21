param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$CorePath = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if (-not $CorePath) { $CorePath = Join-Path $Workspace 'sing-box-1.14.1-socks-proxy.2.exe' }
if (-not (Test-Path -LiteralPath $CorePath -PathType Leaf)) { throw "Locked core missing: $CorePath" }

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
    $watch = "Start-Sleep -Seconds 90; Get-Process -Id $($process.Id) -ErrorAction SilentlyContinue | Where-Object { `$_.StartTime.ToUniversalTime().Ticks -eq $($process.StartTime.ToUniversalTime().Ticks) } | Stop-Process -Force"
    Start-Process powershell -ArgumentList @('-NoProfile', '-NonInteractive', '-EncodedCommand', ([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($watch)))) -WindowStyle Hidden | Out-Null
    Start-Sleep -Milliseconds 900
    $process.Refresh()
    if ($process.HasExited) { throw "Core exited: $Name" }
    return $process
}

function Query-A($Name) {
    $query = [Collections.Generic.List[byte]]::new()
    $query.AddRange([byte[]]@(18, 52, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0))
    foreach ($label in $Name.Split('.')) {
        $query.Add([byte]$label.Length)
        $query.AddRange([Text.Encoding]::ASCII.GetBytes($label))
    }
    $query.AddRange([byte[]]@(0, 0, 1, 0, 1))
    $udp = [Net.Sockets.UdpClient]::new()
    $udp.Client.ReceiveTimeout = 3000
    try {
        $udp.Connect('127.0.0.1', 15358)
        $null = $udp.Send($query.ToArray(), $query.Count)
        $endpoint = [Net.IPEndPoint]::new([Net.IPAddress]::Any, 0)
        $bytes = $udp.Receive([ref]$endpoint)
    } finally { $udp.Dispose() }
    if (($bytes[3] -band 15) -ne 0 -or $bytes[7] -lt 1) { throw 'DNS response has no answer' }
    $index = 12
    while ($bytes[$index] -ne 0) { $index += 1 + [int]$bytes[$index] }
    $index += 5
    if (($bytes[$index] -band 192) -eq 192) { $index += 2 } else {
        while ($bytes[$index] -ne 0) { $index += 1 + [int]$bytes[$index] }
        $index++
    }
    $type = 256 * [int]$bytes[$index] + [int]$bytes[$index + 1]
    $length = 256 * [int]$bytes[$index + 8] + [int]$bytes[$index + 9]
    $index += 10
    if ($type -ne 1 -or $length -ne 4) { throw 'Expected A response' }
    return ($bytes[$index..($index + 3)] -join '.')
}

function Probe-Http($FakeIp, $Domain) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $body = & curl.exe --silent --show-error --fail --max-time 5 --noproxy '*' --header "Host: $Domain" "http://${FakeIp}:18100/" 2>$null
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $old
    return [pscustomobject]@{ Success = ($exitCode -eq 0 -and ($body -join '') -eq 'fakeip-routing'); ExitCode = $exitCode }
}

function Probe-Ssh($FakeIp) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $client.ReceiveTimeout = 4000
        $client.Connect($FakeIp, 22)
        $reader = [IO.StreamReader]::new($client.GetStream())
        return $reader.ReadLine().StartsWith('SSH-2.0-fakeip-routing')
    } catch { return $false } finally { $client.Dispose() }
}

foreach ($port in @(18100, 18101, 18122, 15356, 15358)) {
    if ((Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) -or (Get-NetUDPEndpoint -LocalPort $port -ErrorAction SilentlyContinue)) { throw "Port busy: $port" }
}
if (Get-NetAdapter -Name 'socks-proxy-fakeip' -ErrorAction SilentlyContinue) { throw 'Test adapter already exists' }

$httpJob = Start-Job -ScriptBlock {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 18100); $listener.Start()
    try { while ($true) {
        if (-not $listener.Pending()) { Start-Sleep -Milliseconds 50; continue }
        $client = $listener.AcceptTcpClient()
        try {
            $stream = $client.GetStream(); $buffer = New-Object byte[] 8192; $null = $stream.Read($buffer, 0, $buffer.Length)
            $body = 'fakeip-routing'; $reply = "HTTP/1.1 200 OK`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n$body"
            $bytes = [Text.Encoding]::ASCII.GetBytes($reply); $stream.Write($bytes, 0, $bytes.Length)
        } finally { $client.Dispose() }
    }} finally { $listener.Stop() }
}
$sshJob = Start-Job -ScriptBlock {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 18122); $listener.Start()
    try { while ($true) {
        if (-not $listener.Pending()) { Start-Sleep -Milliseconds 50; continue }
        $client = $listener.AcceptTcpClient()
        try { $bytes = [Text.Encoding]::ASCII.GetBytes("SSH-2.0-fakeip-routing`r`n"); $client.GetStream().Write($bytes, 0, $bytes.Length) } finally { $client.Dispose() }
    }} finally { $listener.Stop() }
}

try {
    $hosts = @{
        'domain.fakeip.invalid' = @('127.0.0.1')
        'ssh.fakeip.invalid' = @('127.0.0.1')
        'multi.fakeip.invalid' = @('127.0.0.1', '203.0.113.44')
        'private.fakeip.invalid' = @('10.44.55.66')
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
    $null = Start-Core 'fakeip-routing-fixture' $fixture

    $cases = @(
        @{ Name = 'fakeip-browser-domain'; Domain = 'domain.fakeip.invalid'; Kind = 'domain'; Port = 18100 },
        @{ Name = 'fakeip-ssh-domain'; Domain = 'ssh.fakeip.invalid'; Kind = 'ssh'; Port = 22 },
        @{ Name = 'fakeip-multi-any-ip'; Domain = 'multi.fakeip.invalid'; Kind = 'public-ip'; Port = 18100 },
        @{ Name = 'fakeip-remote-private-ip'; Domain = 'private.fakeip.invalid'; Kind = 'private-ip'; Port = 18100 }
    )
    foreach ($case in $cases) {
        $cache = Join-Path $Workspace "$($case.Name)-cache.db"
        Remove-Item $cache -Force -ErrorAction SilentlyContinue
        $rules = [Collections.Generic.List[object]]::new()
        if ($case.Kind -eq 'domain' -or $case.Kind -eq 'ssh') {
            $targetPort = if ($case.Kind -eq 'ssh') { 18122 } else { 18100 }
            $rules.Add(@{ domain = @($case.Domain); port = @($case.Port); action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = $targetPort })
        }
        $rules.Add(@{ network = @('tcp', 'udp'); action = 'resolve'; server = 'real'; timeout = '2s' })
        if ($case.Kind -eq 'public-ip') {
            $rules.Add(@{ ip_cidr = @('203.0.113.0/24'); action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = 18100 })
        }
        if ($case.Kind -eq 'private-ip') {
            $rules.Add(@{ ip_cidr = @('10.44.55.66/32'); action = 'route'; outbound = 'proxy'; override_address = '127.0.0.1'; override_port = 18100 })
        }
        $rules.Add(@{ action = 'route'; outbound = 'direct'; override_address = '127.0.0.1'; override_port = 18100 })
        $config = @{
            log = @{ level = 'debug' }
            dns = @{
                servers = @(
                    @{ type = 'fakeip'; tag = 'fakeip'; inet4_range = '198.18.0.0/15'; inet6_range = 'fc00::/18' },
                    @{ type = 'udp'; tag = 'real'; server = '127.0.0.1'; server_port = 15356 }
                )
                rules = @(@{ query_type = @('A', 'AAAA'); action = 'route'; server = 'fakeip' })
                final = 'real'
                reverse_mapping = $true
            }
            inbounds = @(
                @{ type = 'direct'; tag = 'dns-in'; listen = '127.0.0.1'; listen_port = 15358 },
                @{ type = 'tun'; tag = 'tun-in'; interface_name = 'socks-proxy-fakeip'; address = @('172.30.254.1/30'); auto_route = $true; strict_route = $false; route_address = @('198.18.0.0/15'); dns_mode = 'hijack'; stack = 'mixed' }
            )
            outbounds = @(
                @{ type = 'direct'; tag = 'direct' },
                @{ type = 'socks'; tag = 'proxy'; server = '127.0.0.1'; server_port = 18101; version = '5' }
            )
            route = @{ auto_detect_interface = $true; default_domain_resolver = @{ server = 'real' }; rules = @(@{ inbound = @('dns-in'); action = 'hijack-dns' }) + $rules.ToArray(); final = 'direct' }
            experimental = @{ cache_file = @{ enabled = $true; path = $cache; store_fakeip = $true; strict_mode = $true } }
        }
        $process = Start-Core $case.Name $config
        $fakeIp = Query-A $case.Domain
        if ($case.Kind -eq 'ssh') {
            $actual = Probe-Ssh $fakeIp; $exitCode = if ($actual) { 0 } else { 1 }
        } else {
            $probe = Probe-Http $fakeIp $case.Domain; $actual = $probe.Success; $exitCode = $probe.ExitCode
        }
        Stop-Process -Id $process.Id -Force; $process.WaitForExit(); Start-Sleep -Milliseconds 500
        $log = Get-Content (Join-Path $Workspace "$($case.Name).stderr.log") -Raw
        $routeEvidence = $log.Contains('outbound/socks[proxy]: outbound connection')
        $results.Add([pscustomobject]@{ Name = $case.Name; Pass = ($actual -and $routeEvidence -and $fakeIp.StartsWith('198.18.')); FakeIp = $fakeIp; ActualSuccess = $actual; ProxyEvidence = $routeEvidence; ExitCode = $exitCode })
    }
} finally {
    foreach ($process in $owned) { $process.Refresh(); if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue } }
    Stop-Job $httpJob, $sshJob -ErrorAction SilentlyContinue
    Remove-Job $httpJob, $sshJob -Force -ErrorAction SilentlyContinue
    $results | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $Workspace 'fakeip-routing-results.json')
}

$results | ConvertTo-Json -Depth 5
if (@($results | Where-Object { -not $_.Pass }).Count) { throw 'FakeIP routing smoke failed' }
