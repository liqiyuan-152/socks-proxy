param([string]$Workspace=(Join-Path $env:USERPROFILE 'socks-proxy-validation'))
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Security
$dir=Join-Path $Workspace ('credential-test-'+[guid]::NewGuid().ToString('N'))
$null=New-Item -ItemType Directory $dir
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$acl=New-Object Security.AccessControl.DirectorySecurity
$acl.SetAccessRuleProtection($true,$false)
foreach($sid in @($identity.User,[Security.Principal.SecurityIdentifier]::new('S-1-5-18'))){
 $rule=[Security.AccessControl.FileSystemAccessRule]::new($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow');$acl.AddAccessRule($rule)
}
Set-Acl -Path $dir -AclObject $acl
$results=[Collections.Generic.List[object]]::new()
try{
 $plain=[Text.Encoding]::UTF8.GetBytes('synthetic-fixture-secret')
 $encrypted=[Security.Cryptography.ProtectedData]::Protect($plain,$null,[Security.Cryptography.DataProtectionScope]::CurrentUser)
 $file=Join-Path $dir 'secret.bin';[IO.File]::WriteAllBytes($file,$encrypted)
 $decoded=[Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($file),$null,[Security.Cryptography.DataProtectionScope]::CurrentUser)
 $results.Add([pscustomobject]@{Name='dpapi-current-user-roundtrip';Pass=([Convert]::ToBase64String($plain) -eq [Convert]::ToBase64String($decoded))})
 $results.Add([pscustomobject]@{Name='ciphertext-not-plaintext';Pass=(-not [Text.Encoding]::UTF8.GetString($encrypted).Contains('synthetic-fixture-secret'))})
 $actual=Get-Acl $file
 $allowed=@($identity.User.Value,'S-1-5-18')
 $unexpected=@($actual.Access | Where-Object {$_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -notin $allowed})
 $results.Add([pscustomobject]@{Name='restricted-file-acl';Pass=($unexpected.Count -eq 0);OwnerSid=$identity.User.Value})
 $encrypted[20]=$encrypted[20] -bxor 1
 $rejected=$false
 try{$null=[Security.Cryptography.ProtectedData]::Unprotect($encrypted,$null,[Security.Cryptography.DataProtectionScope]::CurrentUser)}catch{$rejected=$true}
 $results.Add([pscustomobject]@{Name='dpapi-corrupt-data-rejected';Pass=$rejected})
}finally{Remove-Item -Recurse -Force $dir}
$results | ConvertTo-Json | Set-Content (Join-Path $Workspace 'credential-results.json')
$results | ConvertTo-Json
if(@($results|Where-Object {-not $_.Pass}).Count){throw 'Credential smoke failed'}
