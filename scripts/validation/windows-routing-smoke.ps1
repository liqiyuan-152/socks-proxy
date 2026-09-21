param([string]$Workspace=(Join-Path $env:USERPROFILE 'socks-proxy-validation'), [string]$CorePath='', [switch]$StrictCache)
$ErrorActionPreference='Stop'
$exe=if($CorePath){$CorePath}else{Join-Path $Workspace 'sing-box-1.14.1-windows-amd64\sing-box.exe'}
$owned=[Collections.Generic.List[object]]::new();$results=[Collections.Generic.List[object]]::new()
function Start-Core($Name,$Config){
 $path=Join-Path $Workspace "$Name.json"
 [IO.File]::WriteAllText($path,($Config|ConvertTo-Json -Depth 20),(New-Object Text.UTF8Encoding($false)))
 & $exe check -c $path;if($LASTEXITCODE -ne 0){throw "Invalid $Name"}
 $p=Start-Process $exe -ArgumentList @('run','-c',('"'+$path+'"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Workspace "$Name.stdout.log") -RedirectStandardError (Join-Path $Workspace "$Name.stderr.log")
 $owned.Add($p)
 $watch="Start-Sleep -Seconds 90; Get-Process -Id $($p.Id) -ErrorAction SilentlyContinue | Where-Object { `$_.StartTime.ToUniversalTime().Ticks -eq $($p.StartTime.ToUniversalTime().Ticks) } | Stop-Process -Force"
 Start-Process powershell -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand',([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($watch)))) -WindowStyle Hidden | Out-Null
 Start-Sleep -Milliseconds 700;$p.Refresh();if($p.HasExited){throw "Exited $Name"};return $p
}
foreach($port in @(18080,18091,18093,15354,15355,18099)){
 if((Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) -or (Get-NetUDPEndpoint -LocalPort $port -ErrorAction SilentlyContinue)){throw "Port busy $port"}
}
$job=Start-Job -ScriptBlock {
 $l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,18080);$l.Start();$until=[DateTime]::UtcNow.AddSeconds(90)
 try{while([DateTime]::UtcNow -lt $until){if(-not $l.Pending()){Start-Sleep -Milliseconds 50;continue};$c=$l.AcceptTcpClient();try{$s=$c.GetStream();$s.ReadTimeout=3000;$b=New-Object byte[] 8192;$null=$s.Read($b,0,$b.Length);$data=[Text.Encoding]::ASCII.GetBytes("HTTP/1.1 200 OK`r`nContent-Length: 2`r`nConnection: close`r`n`r`nOK");$s.Write($data,0,$data.Length)}finally{$c.Dispose()}}}finally{$l.Stop()}
}
try{
 $hosts=@{'direct.fixture.invalid'=@('127.0.0.1');'cidr.fixture.invalid'=@('203.0.113.10');'multi.fixture.invalid'=@('127.0.0.1','203.0.113.11');'ipv6.fixture.invalid'=@('2001:db8::10');'domain.fixture.invalid'=@('127.0.0.1')}
 $fixture=@{log=@{level='debug'};inbounds=@(@{type='mixed';tag='proxy-in';listen='127.0.0.1';listen_port=18091},@{type='direct';tag='dns-in';listen='127.0.0.1';listen_port=15354});outbounds=@(@{type='direct';tag='direct'});dns=@{servers=@(@{type='hosts';tag='hosts';predefined=$hosts});final='hosts'};route=@{default_domain_resolver='hosts';rules=@(@{inbound=@('dns-in');action='hijack-dns'});final='direct'}}
 $null=Start-Core 'routing-fixture' $fixture
 $cases=@(
 @{Name='dns-direct-with-dead-proxy';Domain='direct.fixture.invalid';DeadProxy=$true;DnsPort=15354;Success=$true;Route='direct'},
 @{Name='dns-cidr';Domain='cidr.fixture.invalid';DeadProxy=$false;DnsPort=15354;Success=$true;Route='proxy'},
 @{Name='dns-multi-any';Domain='multi.fixture.invalid';DeadProxy=$false;DnsPort=15354;Success=$true;Route='proxy'},
 @{Name='dns-ipv6-cidr';Domain='ipv6.fixture.invalid';DeadProxy=$false;DnsPort=15354;Success=$true;Route='proxy'},
 @{Name='dns-domain-dead-proxy';Domain='domain.fixture.invalid';DeadProxy=$true;DnsPort=15354;Success=$false;Route='proxy'},
 @{Name='dns-cidr-dead-proxy';Domain='cidr.fixture.invalid';DeadProxy=$true;DnsPort=15354;Success=$false;Route='proxy'},
 @{Name='dns-upstream-unavailable';Domain='direct.fixture.invalid';DeadProxy=$false;DnsPort=15355;Success=$false;Route=''}
 )
 foreach($case in $cases){
 $proxyPort=if($case.DeadProxy){18099}else{18091}
 $cfg=@{log=@{level='debug'};inbounds=@(@{type='mixed';listen='127.0.0.1';listen_port=18093});outbounds=@(@{type='direct';tag='direct'},@{type='socks';tag='proxy';server='127.0.0.1';server_port=$proxyPort;version='5'});dns=@{servers=@(@{type='udp';tag='real';server='127.0.0.1';server_port=$case.DnsPort});final='real'};route=@{default_domain_resolver='real';rules=@(@{domain=@('domain.fixture.invalid');action='route';outbound='proxy';override_address='127.0.0.1';override_port=18080},@{action='resolve';server='real';timeout='1s'},@{ip_cidr=@('203.0.113.0/24','2001:db8::/32');port=@(18080);action='route';outbound='proxy';override_address='127.0.0.1';override_port=18080});final='direct'}}
 $p=Start-Core $case.Name $cfg
 $old=$ErrorActionPreference;$ErrorActionPreference='Continue'
 $body=& curl.exe --silent --show-error --fail --max-time 4 --noproxy no-bypass.invalid --proxy http://127.0.0.1:18093 "http://$($case.Domain):18080/" 2>$null
 $exit=$LASTEXITCODE;$ErrorActionPreference=$old
 $ok=($exit -eq 0 -and ($body -join '') -eq 'OK')
 Stop-Process -Id $p.Id -Force;$p.WaitForExit()
 $log=Get-Content (Join-Path $Workspace "$($case.Name).stderr.log") -Raw
 $routeOk=if($case.Route){$log.Contains("outbound/$($case.Route -replace 'proxy','socks')[$($case.Route)]: outbound connection")}else{-not $log.Contains('outbound/socks[proxy]: outbound connection')}
 $results.Add([pscustomobject]@{Name=$case.Name;Pass=($ok -eq $case.Success -and $routeOk);ExpectedSuccess=$case.Success;ActualSuccess=$ok;ExpectedRoute=$case.Route;RouteEvidence=$routeOk;CurlExit=$exit})
 }
}finally{
 foreach($p in $owned){$p.Refresh();if(-not $p.HasExited){Stop-Process -Id $p.Id -Force}}
 Stop-Job $job;Remove-Job $job -Force
 $results|ConvertTo-Json -Depth 5|Set-Content (Join-Path $Workspace 'routing-results.json')
}
$results|ConvertTo-Json -Depth 5
if(@($results|Where-Object {-not $_.Pass}).Count){throw 'Routing smoke failed'}
