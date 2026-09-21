param([string]$Workspace=(Join-Path $env:USERPROFILE 'socks-proxy-validation'), [string]$CorePath='', [switch]$StrictCache)
$ErrorActionPreference='Stop'
$exe=if($CorePath){$CorePath}else{Join-Path $Workspace 'sing-box-1.14.1-windows-amd64\sing-box.exe'}
$original=Join-Path $Workspace 'dns-smoke.json'
$config=Get-Content $original -Raw | ConvertFrom-Json
$cache=Join-Path $Workspace ('corrupt-'+[guid]::NewGuid().ToString('N')+'.db')
$path=$cache+'.json';$config.experimental.cache_file.path=$cache
if($StrictCache){$config.experimental.cache_file | Add-Member -NotePropertyName strict_mode -NotePropertyValue $true -Force}
[IO.File]::WriteAllBytes($cache,[Text.Encoding]::ASCII.GetBytes('deliberately-corrupted-test-cache'))
[IO.File]::WriteAllText($path,($config|ConvertTo-Json -Depth 20),(New-Object Text.UTF8Encoding($false)))
$originalHash=(Get-FileHash $cache -Algorithm SHA256).Hash
$p=$null
try{
 $p=Start-Process $exe -ArgumentList @('run','-c',('"'+$path+'"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Workspace 'cache-corrupt.stdout.log') -RedirectStandardError (Join-Path $Workspace 'cache-corrupt.stderr.log')
 $processHandle=$p.Handle
 $exited=$p.WaitForExit(5000)
 if($exited){$p.WaitForExit();$p.Refresh()}
 $log=Get-Content (Join-Path $Workspace 'cache-corrupt.stderr.log') -Raw
 $result=[pscustomobject]@{Name='corrupt-cache-fails-startup';Pass=($exited -and $null -ne $p.ExitCode -and $p.ExitCode -ne 0 -and $log -match 'STRICT_CACHE_ERROR' -and (Get-FileHash $cache -Algorithm SHA256).Hash -eq $originalHash);Exited=$exited;ExitCode=if($exited){$p.ExitCode}else{$null}}
 $result|ConvertTo-Json|Set-Content (Join-Path $Workspace 'cache-corruption-results.json');$result|ConvertTo-Json
}finally{
 if($p){$p.Refresh();if(-not $p.HasExited){Stop-Process -Id $p.Id -Force}}
 Remove-Item $cache,$path -Force -ErrorAction SilentlyContinue
}
if(-not $result.Pass){throw 'Core did not reject corrupted cache; review startup recovery contract'}
