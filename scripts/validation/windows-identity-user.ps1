param(
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [string]$ForeignBlob,
    [string]$ErrorOutput
)

$ErrorActionPreference = 'Stop'
trap {
    if ($ErrorOutput) { ($_ | Out-String) | Set-Content -Encoding UTF8 $ErrorOutput }
    exit 1
}
Add-Type -AssemblyName System.Security
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

$identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$plain = [Text.Encoding]::UTF8.GetBytes('controlled-dpapi-test-value')
$cipher = [Security.Cryptography.ProtectedData]::Protect(
    $plain,
    $null,
    [Security.Cryptography.DataProtectionScope]::CurrentUser
)
$blobPath = Join-Path $OutputDirectory 'standard-user.blob'
[IO.File]::WriteAllBytes($blobPath, $cipher)

$foreignDecryptSucceeded = $null
if ($ForeignBlob) {
    try {
        $foreign = [IO.File]::ReadAllBytes($ForeignBlob)
        [void][Security.Cryptography.ProtectedData]::Unprotect(
            $foreign,
            $null,
            [Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $foreignDecryptSucceeded = $true
    } catch {
        $foreignDecryptSucceeded = $false
    }
}

$temporary = Join-Path $OutputDirectory 'restricted-temp.json'
if (-not $ForeignBlob) {
    [IO.File]::WriteAllText($temporary, '{"credential":"controlled-test-only"}')
    $acl = New-Object Security.AccessControl.FileSecurity
    $acl.SetAccessRuleProtection($true, $false)
    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $systemSid = New-Object Security.Principal.SecurityIdentifier('S-1-5-18')
    $rights = [Security.AccessControl.FileSystemRights]::FullControl
    $allow = [Security.AccessControl.AccessControlType]::Allow
    $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($currentSid, $rights, $allow)))
    $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($systemSid, $rights, $allow)))
    Set-Acl -Path $temporary -AclObject $acl
}
$appliedAcl = Get-Acl -Path $temporary

[ordered]@{
    identity = $identity
    blobPath = $blobPath
    dpapiRoundTrip = (
        [Text.Encoding]::UTF8.GetString(
            [Security.Cryptography.ProtectedData]::Unprotect(
                $cipher,
                $null,
                [Security.Cryptography.DataProtectionScope]::CurrentUser
            )
        ) -eq 'controlled-dpapi-test-value'
    )
    foreignDecryptSucceeded = $foreignDecryptSucceeded
    tempOwner = $appliedAcl.Owner
    tempSddl = $appliedAcl.Sddl
    tempPrincipals = @($appliedAcl.Access | ForEach-Object { $_.IdentityReference.Value })
} | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'result.json')
