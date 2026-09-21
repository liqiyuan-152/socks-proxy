# Windows-only, isolated loopback fixtures. Does not change system proxy or DNS.
param([string]$Workspace=(Join-Path $env:USERPROFILE 'socks-proxy-validation'), [string]$CorePath='', [switch]$StrictCache)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$exe=if($CorePath){$CorePath}else{Join-Path $Workspace 'sing-box-1.14.1-windows-amd64\sing-box.exe'}
$results=[Collections.Generic.List[object]]::new()
$owned=[Collections.Generic.List[object]]::new()
$utf8=New-Object Text.UTF8Encoding($false)
function Save-Config($Name,$Config){
 $path=Join-Path $Workspace "$Name.json"
 [IO.File]::WriteAllText($path,($Config|ConvertTo-Json -Depth 20),$utf8)
 & $exe check -c $path
 if($LASTEXITCODE -ne 0){throw "Invalid config: $Name"}
 return $path
}
function Start-Core($Name,$Config){
 $path=Save-Config $Name $Config
 $proc=Start-Process $exe -ArgumentList @('run','-c',('"'+$path+'"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Workspace "$Name.stdout.log") -RedirectStandardError (Join-Path $Workspace "$Name.stderr.log")
 $owned.Add($proc)
 # Independent deadline also cleans up if SSH or this script is interrupted.
 $watch="Start-Sleep -Seconds 60; Get-Process -Id $($proc.Id) -ErrorAction SilentlyContinue | Where-Object { `$_.StartTime.ToUniversalTime().Ticks -eq $($proc.StartTime.ToUniversalTime().Ticks) } | Stop-Process -Force"
 $enc=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($watch))
 Start-Process powershell -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand',$enc) -WindowStyle Hidden | Out-Null
 Start-Sleep -Milliseconds 800
 $proc.Refresh();if($proc.HasExited){throw "Core exited: $Name (see stderr log)"}
 return $proc
}
function Probe($Name,$Proxy,$Url,$Expected){
 # Windows PowerShell 5 drops empty native arguments: use a non-matching hostname.
 $args=@('--silent','--show-error','--fail','--max-time','5','--noproxy','no-bypass.invalid')
 if($Proxy){$args+=@('--proxy',$Proxy)}else{$args+=@('--noproxy','*')}
 $args+=$Url
 $old=$ErrorActionPreference;$ErrorActionPreference='Continue'
 $body=& curl.exe @args 2>$null
 $exit=$LASTEXITCODE;$ErrorActionPreference=$old
 $ok=($exit -eq 0 -and ($body -join '') -eq 'socks-proxy-fixture')
 $results.Add([pscustomobject]@{Name=$Name;ExpectedSuccess=$Expected;ActualSuccess=$ok;CurlExit=$exit;Pass=($ok -eq $Expected)})
}
# Fail instead of hijacking an existing listener or test interface.
foreach($port in @(18080,18081,18082,18083)){
 if(Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue){throw "Port in use: $port"}
}
if(Get-NetAdapter -Name 'socks-proxy-m0' -ErrorAction SilentlyContinue){throw 'Test adapter already exists'}
$before=@(Get-NetRoute | Select-Object DestinationPrefix,NextHop,InterfaceIndex)
$server=Start-Job -ScriptBlock {
 $listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,18080);$listener.Start()
 try{while($true){if(-not $listener.Pending()){Start-Sleep -Milliseconds 100;continue};$c=$listener.AcceptTcpClient();try{$s=$c.GetStream();$s.ReadTimeout=3000;$buf=New-Object byte[] 8192;$null=$s.Read($buf,0,$buf.Length);$body='socks-proxy-fixture';$reply="HTTP/1.1 200 OK`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n$body";$data=[Text.Encoding]::ASCII.GetBytes($reply);$s.Write($data,0,$data.Length)}finally{$c.Dispose()}}}finally{$listener.Stop()}
}
try{
 Start-Sleep -Seconds 2
 Probe 'direct-baseline' '' 'http://127.0.0.1:18080/' $true
 $fixture=@{log=@{level='info'};inbounds=@(@{type='mixed';tag='no-auth';listen='127.0.0.1';listen_port=18081},@{type='mixed';tag='auth';listen='127.0.0.1';listen_port=18082;users=@(@{username='fixture';password='fixture-only'})});outbounds=@(@{type='direct';tag='direct'});route=@{final='direct'}}
 $upstream=Start-Core 'fixture' $fixture
 foreach($protocol in @('socks','http')){
  foreach($auth in @('none','valid','wrong')){
   $out=@{type=$protocol;tag='proxy';server='127.0.0.1';server_port=18081}
   if($protocol -eq 'socks'){$out.version='5'}
   if($auth -ne 'none'){$out.server_port=18082;$out.username='fixture';$out.password=if($auth -eq 'valid'){'fixture-only'}else{'wrong-fixture'}}
   $cfg=@{log=@{level='debug'};inbounds=@(@{type='mixed';listen='127.0.0.1';listen_port=18083});outbounds=@($out);route=@{final='proxy'}}
   $client=Start-Core "client-$protocol-$auth" $cfg
   Probe "$protocol-$auth" 'http://127.0.0.1:18083' 'http://127.0.0.1:18080/' ($auth -ne 'wrong')
   Stop-Process -Id $client.Id -Force; $client.WaitForExit()
  }
 }
 # A documentation-only subnet is the sole TUN route. SSH/LAN/default routes stay outside it.
 $tun=@{log=@{level='debug'};inbounds=@(@{type='tun';tag='tun-in';interface_name='socks-proxy-m0';address=@('172.30.255.1/30');auto_route=$true;strict_route=$false;dns_mode='disabled';route_address=@('198.19.0.0/24');stack='mixed'});outbounds=@(@{type='direct';tag='direct'});route=@{auto_detect_interface=$true;rules=@(@{ip_cidr=@('198.19.0.1/32');action='route';outbound='direct';override_address='127.0.0.1';override_port=18080});final='direct'}}
 $tunProc=Start-Core 'tun-minimal' $tun
 $tunDeadline=(Get-Date).AddSeconds(10)
 do{
  $tunAdapter=Get-NetAdapter -Name 'socks-proxy-m0' -ErrorAction SilentlyContinue
  $tunRoute=Get-NetRoute -DestinationPrefix '198.19.0.0/24' -ErrorAction SilentlyContinue
  if($tunAdapter -and $tunAdapter.Status -eq 'Up' -and $tunRoute){break}
  Start-Sleep -Milliseconds 250
 }while((Get-Date) -lt $tunDeadline)
 if(-not $tunAdapter -or $tunAdapter.Status -ne 'Up' -or -not $tunRoute){
  throw 'TUN adapter or route did not become ready within 10 seconds'
 }
 Probe 'tun-tcp-forward' '' 'http://198.19.0.1:18080/' $true
 Stop-Process -Id $tunProc.Id -Force;$tunProc.WaitForExit()
 Start-Sleep -Seconds 2
 $residual=@(Get-NetRoute -DestinationPrefix '198.19.0.0/24' -ErrorAction SilentlyContinue)
 $results.Add([pscustomobject]@{Name='tun-route-cleanup';Pass=($residual.Count -eq 0)})
 Probe 'direct-after-tun' '' 'http://127.0.0.1:18080/' $true
}finally{
 foreach($proc in $owned){$proc.Refresh();if(-not $proc.HasExited){Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue}}
 Stop-Job $server -ErrorAction SilentlyContinue;Remove-Job $server -Force -ErrorAction SilentlyContinue
 $before | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $Workspace 'routes-before.json')
 $results | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $Workspace 'smoke-results.json')
}
$results | ConvertTo-Json -Depth 5
if(@($results | Where-Object {-not $_.Pass}).Count -gt 0){throw 'One or more smoke checks failed'}
