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
function Read-Header($Stream){
 $bytes=[Collections.Generic.List[byte]]::new()
 while($bytes.Count -lt 8192){$v=$Stream.ReadByte();if($v -lt 0){break};$bytes.Add([byte]$v);if($bytes.Count -ge 4 -and [Text.Encoding]::ASCII.GetString($bytes.ToArray()).EndsWith("`r`n`r`n")){break}}
 return [Text.Encoding]::ASCII.GetString($bytes.ToArray())
}
function Probe-Ssh($Name,$Port,$Expected){
 $c=[Net.Sockets.TcpClient]::new();$ok=$false;$header='';$banner=''
 try{$c.Connect('127.0.0.1',$Port);$s=$c.GetStream();$s.ReadTimeout=3000;$q=[Text.Encoding]::ASCII.GetBytes("CONNECT 127.0.0.1:22 HTTP/1.1`r`nHost: 127.0.0.1:22`r`n`r`n");$s.Write($q,0,$q.Length);$header=Read-Header $s
 if($header -match '^HTTP/1.[01] 200'){$reader=[IO.StreamReader]::new($s);$banner=$reader.ReadLine();$ok=$banner.StartsWith('SSH-2.0-')}
 }catch{$header=$_.Exception.Message}finally{$c.Dispose()}
 $results.Add([pscustomobject]@{Name=$Name;Pass=($ok -eq $Expected);SshBanner=$banner;Response=($header -split "`r`n")[0]})
}
foreach($port in @(18091,18092,18093,18400,18401)){
 if((Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) -or (Get-NetUDPEndpoint -LocalPort $port -ErrorAction SilentlyContinue)){throw "Port busy $port"}
}
$job=Start-Job -ScriptBlock {
 $u=[Net.Sockets.UdpClient]::new([Net.IPEndPoint]::new([Net.IPAddress]::Loopback,18401));$deadline=[DateTime]::UtcNow.AddSeconds(90)
 try{while([DateTime]::UtcNow -lt $deadline){if($u.Available -eq 0){Start-Sleep -Milliseconds 50;continue};$ep=[Net.IPEndPoint]::new([Net.IPAddress]::Any,0);$b=$u.Receive([ref]$ep);$null=$u.Send($b,$b.Length,$ep)}}finally{$u.Dispose()}
}
Start-Sleep -Seconds 2
try{
 $fixture=@{log=@{level='debug'};inbounds=@(@{type='mixed';tag='allow';listen='127.0.0.1';listen_port=18091},@{type='mixed';tag='deny-ssh';listen='127.0.0.1';listen_port=18092});outbounds=@(@{type='direct';tag='direct'});route=@{rules=@(@{inbound=@('deny-ssh');port=@(22);action='reject'});final='direct'}}
 $null=Start-Core 'protocol-fixture' $fixture
 foreach($protocol in @('socks','http')){
  $out=@{type=$protocol;tag='proxy';server='127.0.0.1';server_port=18091};if($protocol -eq 'socks'){$out.version='5'}
  $cfg=@{log=@{level='debug'};inbounds=@(@{type='mixed';listen='127.0.0.1';listen_port=18093},@{type='direct';listen='127.0.0.1';listen_port=18400;network='udp';override_address='127.0.0.1';override_port=18401});outbounds=@($out);route=@{final='proxy'}}
  $p=Start-Core "protocol-$protocol" $cfg
  Probe-Ssh "ssh-banner-via-$protocol" 18093 $true
  $u=[Net.Sockets.UdpClient]::new();$u.Client.ReceiveTimeout=2000;$ok=$false
  try{$u.Connect('127.0.0.1',18400);$b=[Text.Encoding]::ASCII.GetBytes('udp-fixture');$null=$u.Send($b,$b.Length);$ep=[Net.IPEndPoint]::new([Net.IPAddress]::Any,0);$answer=$u.Receive([ref]$ep);$ok=([Text.Encoding]::ASCII.GetString($answer) -eq 'udp-fixture')}catch{}finally{$u.Dispose()}
  $udpLog=Get-Content (Join-Path $Workspace "protocol-$protocol.stderr.log") -Raw
  $explicitReject=($protocol -eq 'socks' -or $udpLog.Contains('UDP is not supported by outbound: proxy'))
  $results.Add([pscustomobject]@{Name="udp-via-$protocol";Pass=($ok -eq ($protocol -eq 'socks') -and $explicitReject);EchoReceived=$ok;ProtocolEvidence=$explicitReject})
  Stop-Process -Id $p.Id -Force;$p.WaitForExit()
 }
 Probe-Ssh 'http-connect-22-rejected' 18092 $false
 $rejectLog=Get-Content (Join-Path $Workspace 'protocol-fixture.stderr.log') -Raw
 $results[$results.Count-1].Pass=($results[$results.Count-1].Pass -and $rejectLog.Contains('inbound=deny-ssh port=22 => reject'))
}finally{
 foreach($p in $owned){$p.Refresh();if(-not $p.HasExited){Stop-Process -Id $p.Id -Force}}
 Stop-Job $job;Remove-Job $job -Force
 $results|ConvertTo-Json -Depth 5|Set-Content (Join-Path $Workspace 'protocol-results.json')
}
$results|ConvertTo-Json -Depth 5
if(@($results|Where-Object {-not $_.Pass}).Count){throw 'Protocol smoke failed'}
