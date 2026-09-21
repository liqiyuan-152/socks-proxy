param([string]$Workspace=(Join-Path $env:USERPROFILE 'socks-proxy-validation'), [string]$CorePath='', [switch]$StrictCache)
$ErrorActionPreference='Stop'
$exe=if($CorePath){$CorePath}else{Join-Path $Workspace 'sing-box-1.14.1-windows-amd64\sing-box.exe'}
$path=Join-Path $Workspace 'dns-smoke.json'
$cache=Join-Path $Workspace 'dns-smoke-cache.db'
if((Get-NetUDPEndpoint -LocalPort 15353 -ErrorAction SilentlyContinue) -or (Get-NetTCPConnection -LocalPort 18094 -State Listen -ErrorAction SilentlyContinue)){throw 'DNS test port already in use'}
$cfg=@{log=@{level='debug';timestamp=$false};inbounds=@(@{type='direct';tag='dns-in';listen='127.0.0.1';listen_port=15353},@{type='mixed';tag='probe-in';listen='127.0.0.1';listen_port=18094});outbounds=@(@{type='direct';tag='direct'});dns=@{servers=@(@{type='fakeip';tag='fake';inet4_range='198.18.0.0/15';inet6_range='fc00::/18'},@{type='udp';tag='unused-real';server='127.0.0.1';server_port=15354});rules=@(@{query_type=@('A','AAAA');action='route';server='fake'});final='unused-real'};route=@{default_domain_resolver='unused-real';rules=@(@{inbound=@('dns-in');action='hijack-dns'})};experimental=@{cache_file=@{enabled=$true;path=$cache;store_fakeip=$true}}}
if($StrictCache){$cfg.experimental.cache_file.strict_mode=$true}
[IO.File]::WriteAllText($path,($cfg|ConvertTo-Json -Depth 20),(New-Object Text.UTF8Encoding($false)))
& $exe check -c $path;if($LASTEXITCODE -ne 0){throw 'DNS config invalid'}
function Start-DnsCore($Name){
 $p=Start-Process $exe -ArgumentList @('run','-c',('"'+$path+'"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Workspace "$Name.stdout.log") -RedirectStandardError (Join-Path $Workspace "$Name.stderr.log")
 $watch="Start-Sleep -Seconds 45; Get-Process -Id $($p.Id) -ErrorAction SilentlyContinue | Where-Object { `$_.StartTime.ToUniversalTime().Ticks -eq $($p.StartTime.ToUniversalTime().Ticks) } | Stop-Process -Force"
 Start-Process powershell -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand',([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($watch)))) -WindowStyle Hidden | Out-Null
 Start-Sleep -Seconds 1;$p.Refresh();if($p.HasExited){throw 'DNS core exited'};return $p
}
function Query-A($Name){
 $q=[Collections.Generic.List[byte]]::new();$q.AddRange([byte[]]@(18,52,1,0,0,1,0,0,0,0,0,0))
 foreach($label in $Name.Split('.')){$q.Add([byte]$label.Length);$q.AddRange([Text.Encoding]::ASCII.GetBytes($label))}
 $q.AddRange([byte[]]@(0,0,1,0,1))
 $u=[Net.Sockets.UdpClient]::new();$u.Client.ReceiveTimeout=3000
 try{$u.Connect('127.0.0.1',15353);$null=$u.Send($q.ToArray(),$q.Count);$ep=[Net.IPEndPoint]::new([Net.IPAddress]::Any,0);$b=$u.Receive([ref]$ep)}finally{$u.Dispose()}
 if(($b[3] -band 15) -ne 0 -or $b[7] -lt 1){throw 'DNS response has no answer'}
 # Skip question, then compressed or full owner name of the first answer.
 $i=12;while($b[$i] -ne 0){$i+=1+[int]$b[$i]};$i+=5
 if(($b[$i] -band 192) -eq 192){$i+=2}else{while($b[$i] -ne 0){$i+=1+[int]$b[$i]};$i++}
 $type=256*[int]$b[$i]+[int]$b[$i+1];$len=256*[int]$b[$i+8]+[int]$b[$i+9];$i+=10
 if($type -ne 1 -or $len -ne 4){throw 'Expected A answer'}
 return ($b[$i..($i+3)] -join '.')
}
$p=$null
try{
 $p=Start-DnsCore 'dns-first'
 $a=Query-A 'alpha.fixture.invalid';$b=Query-A 'beta.fixture.invalid'
 Stop-Process -Id $p.Id -Force;$p.WaitForExit();$p=$null
 $p=Start-DnsCore 'dns-restart'
 # Reverse query order detects accidental address reassignment after restart.
 $b2=Query-A 'beta.fixture.invalid';$a2=Query-A 'alpha.fixture.invalid'
 $result=@([pscustomobject]@{Name='fakeip-distinct';Pass=($a -ne $b -and $a.StartsWith('198.18.') -and $b.StartsWith('198.18.'));Alpha=$a;Beta=$b},[pscustomobject]@{Name='fakeip-persist-after-forced-stop';Pass=($a -eq $a2 -and $b -eq $b2);Alpha=$a2;Beta=$b2})
 $old=$ErrorActionPreference;$ErrorActionPreference='Continue'
 $null=& curl.exe --silent --show-error --fail --max-time 3 --noproxy no-bypass.invalid --proxy http://127.0.0.1:18094 http://198.18.255.254:18080/ 2>$null
 $unknownExit=$LASTEXITCODE;$ErrorActionPreference=$old
 $unknownLog=Get-Content (Join-Path $Workspace 'dns-restart.stderr.log') -Raw
 $result+= [pscustomobject]@{Name='unknown-fakeip-rejected';Pass=($unknownExit -ne 0 -and $unknownLog -match 'missing FakeIP|missing fakeip|fakeip.*not found|FakeIP.*not found');CurlExit=$unknownExit}
 $result|ConvertTo-Json|Set-Content (Join-Path $Workspace 'dns-results.json');$result|ConvertTo-Json
 if(@($result|Where-Object {-not $_.Pass}).Count){throw 'DNS smoke failed'}
}finally{if($p){$p.Refresh();if(-not $p.HasExited){Stop-Process -Id $p.Id -Force}}}
