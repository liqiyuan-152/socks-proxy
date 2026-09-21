param([string]$Workspace=(Join-Path $env:USERPROFILE 'socks-proxy-validation'))
$ErrorActionPreference='Stop'
$exe=Join-Path $Workspace 'sing-box.exe'
$evidence=Join-Path $Workspace 'strict-evidence';New-Item -ItemType Directory -Force $evidence | Out-Null
& $exe version
& (Join-Path $Workspace 'cachefile-tests.exe') '-test.run=TestStrict|TestLegacy' '-test.v' 2>&1 | Tee-Object (Join-Path $evidence 'cache-tests.txt')
if($LASTEXITCODE -ne 0){throw 'Windows cache unit tests failed'}
foreach($script in @('windows-core-smoke.ps1','windows-protocol-smoke.ps1','windows-routing-smoke.ps1','windows-dns-smoke.ps1','windows-cache-corruption.ps1')){
 & (Join-Path $Workspace $script) -Workspace $Workspace -CorePath $exe -StrictCache
 if(-not $?){throw "Regression failed: $script"}
}
foreach($name in @('smoke-results.json','protocol-results.json','routing-results.json','dns-results.json','cache-corruption-results.json','cache-corrupt.stderr.log')){Copy-Item (Join-Path $Workspace $name) $evidence -Force}
Get-FileHash $exe -Algorithm SHA256 | Select-Object Algorithm,Hash | ConvertTo-Json | Set-Content (Join-Path $evidence 'binary-hash.json')
