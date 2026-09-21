$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security

$user = 'socksproxym0'
$password = 'S0cksProxy-M0-2026!'
$task = 'SocksProxyM0Identity'
$root = Join-Path $env:USERPROFILE 'socks-proxy-validation'
$payload = Join-Path $root 'windows-identity-user.ps1'
$publicPayload = Join-Path $env:PUBLIC 'sp-i.ps1'
$output = Join-Path $env:PUBLIC 'sp-o'
$adminBlob = Join-Path $output 'admin-user.blob'
$childError = Join-Path $env:PUBLIC 'sp-i-error.txt'
$batchRightGranted = $false

Add-Type @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;

public static class LsaRights {
    [StructLayout(LayoutKind.Sequential)]
    struct LSA_OBJECT_ATTRIBUTES {
        public int Length;
        public IntPtr RootDirectory;
        public IntPtr ObjectName;
        public int Attributes;
        public IntPtr SecurityDescriptor;
        public IntPtr SecurityQualityOfService;
    }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct LSA_UNICODE_STRING {
        public ushort Length;
        public ushort MaximumLength;
        public IntPtr Buffer;
    }
    [DllImport("advapi32.dll")]
    static extern uint LsaOpenPolicy(IntPtr system, ref LSA_OBJECT_ATTRIBUTES attributes, uint access, out IntPtr handle);
    [DllImport("advapi32.dll")]
    static extern uint LsaAddAccountRights(IntPtr handle, IntPtr sid, LSA_UNICODE_STRING[] rights, uint count);
    [DllImport("advapi32.dll")]
    static extern uint LsaRemoveAccountRights(IntPtr handle, IntPtr sid, bool all, LSA_UNICODE_STRING[] rights, uint count);
    [DllImport("advapi32.dll")]
    static extern uint LsaClose(IntPtr handle);
    [DllImport("advapi32.dll")]
    static extern int LsaNtStatusToWinError(uint status);

    public static void Set(string sidText, string right, bool add) {
        var attributes = new LSA_OBJECT_ATTRIBUTES();
        attributes.Length = Marshal.SizeOf(attributes);
        IntPtr policy;
        uint status = LsaOpenPolicy(IntPtr.Zero, ref attributes, 0x810, out policy);
        if (status != 0) throw new Win32Exception(LsaNtStatusToWinError(status));
        IntPtr rightBuffer = Marshal.StringToHGlobalUni(right);
        byte[] sidBytes = new byte[new SecurityIdentifier(sidText).BinaryLength];
        new SecurityIdentifier(sidText).GetBinaryForm(sidBytes, 0);
        var sidHandle = GCHandle.Alloc(sidBytes, GCHandleType.Pinned);
        try {
            var rights = new [] { new LSA_UNICODE_STRING {
                Buffer = rightBuffer,
                Length = (ushort)(right.Length * 2),
                MaximumLength = (ushort)((right.Length + 1) * 2)
            }};
            status = add
                ? LsaAddAccountRights(policy, sidHandle.AddrOfPinnedObject(), rights, 1)
                : LsaRemoveAccountRights(policy, sidHandle.AddrOfPinnedObject(), false, rights, 1);
            if (status != 0) throw new Win32Exception(LsaNtStatusToWinError(status));
        } finally {
            sidHandle.Free();
            Marshal.FreeHGlobal(rightBuffer);
            LsaClose(policy);
        }
    }
}
'@

function Wait-Result {
    $result = Join-Path $output 'result.json'
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        if (Test-Path $result) { return $result }
        Start-Sleep -Milliseconds 500
    }
    $taskStatus = (& schtasks.exe /Query /TN $task /V /FO LIST 2>&1 | Out-String)
    $childDetails = if (Test-Path $childError) { Get-Content -Raw $childError } else { 'no child error file' }
    throw "Timed out waiting for standard-user task result. Child: $childDetails Task: $taskStatus"
}

function Invoke-StandardUser([string]$ForeignBlob) {
    $identity = "$env:COMPUTERNAME\$user"
    Remove-Item -Force (Join-Path $output 'result.json') -ErrorAction SilentlyContinue
    Remove-Item -Force $childError -ErrorAction SilentlyContinue
    $arguments = "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$publicPayload`" -OutputDirectory `"$output`" -ErrorOutput `"$childError`""
    if ($ForeignBlob) { $arguments += " -ForeignBlob `"$ForeignBlob`"" }
    & schtasks.exe /Create /TN $task /SC ONCE /ST 23:59 /TR $arguments /RU $identity /RP $password /F | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "schtasks create failed: $LASTEXITCODE" }
    & schtasks.exe /Run /TN $task | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "schtasks run failed: $LASTEXITCODE" }
    [void](Wait-Result)
    & schtasks.exe /Delete /TN $task /F | Out-Null
}

$failure = $null
try {
    if (-not (Test-Path $payload)) { throw "Missing payload: $payload" }
    Copy-Item -Force $payload $publicPayload

    $securePassword = ConvertTo-SecureString $password -AsPlainText -Force
    New-LocalUser -Name $user -Password $securePassword -PasswordNeverExpires -UserMayNotChangePassword | Out-Null
    $userSid = (Get-LocalUser -Name $user).Sid.Value
    [LsaRights]::Set($userSid, 'SeBatchLogonRight', $true)
    $batchRightGranted = $true
    if (Test-Path $output) { Remove-Item -Recurse -Force $output }
    New-Item -ItemType Directory -Path $output | Out-Null
    $directoryAcl = New-Object Security.AccessControl.DirectorySecurity
    $directoryAcl.SetAccessRuleProtection($true, $false)
    $inheritance = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    $propagation = [Security.AccessControl.PropagationFlags]::None
    $rights = [Security.AccessControl.FileSystemRights]::FullControl
    $allow = [Security.AccessControl.AccessControlType]::Allow
    foreach ($sid in @(
        (New-Object Security.Principal.SecurityIdentifier($userSid)),
        [Security.Principal.WindowsIdentity]::GetCurrent().User,
        (New-Object Security.Principal.SecurityIdentifier('S-1-5-18'))
    )) {
        $directoryAcl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid, $rights, $inheritance, $propagation, $allow)))
    }
    Set-Acl -Path $output -AclObject $directoryAcl

    Invoke-StandardUser ''
    $first = Get-Content -Raw (Wait-Result) | ConvertFrom-Json

    $adminCouldDecryptStandard = $false
    try {
        [void][Security.Cryptography.ProtectedData]::Unprotect(
            [IO.File]::ReadAllBytes($first.blobPath),
            $null,
            [Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $adminCouldDecryptStandard = $true
    } catch {}

    $adminCipher = [Security.Cryptography.ProtectedData]::Protect(
        [Text.Encoding]::UTF8.GetBytes('admin-controlled-value'),
        $null,
        [Security.Cryptography.DataProtectionScope]::CurrentUser
    )
    [IO.File]::WriteAllBytes($adminBlob, $adminCipher)
    Invoke-StandardUser $adminBlob
    $second = Get-Content -Raw (Wait-Result) | ConvertFrom-Json

    $identity = "$env:COMPUTERNAME\$user"
    $allowedPrincipals = @($identity, 'NT AUTHORITY\SYSTEM')
    $unexpectedPrincipals = @($second.tempPrincipals | Where-Object { $_ -notin $allowedPrincipals })
    $passed = $first.dpapiRoundTrip -and
        (-not $adminCouldDecryptStandard) -and
        ($second.foreignDecryptSucceeded -eq $false) -and
        ($unexpectedPrincipals.Count -eq 0)

    [ordered]@{
        passed = $passed
        elevatedIdentity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        standardIdentity = $second.identity
        standardRoundTrip = $first.dpapiRoundTrip
        adminCouldDecryptStandard = $adminCouldDecryptStandard
        standardCouldDecryptAdmin = $second.foreignDecryptSucceeded
        tempOwner = $second.tempOwner
        tempPrincipals = $second.tempPrincipals
        unexpectedPrincipals = $unexpectedPrincipals
        ownershipConclusion = 'Data belongs to the effective process account. Starting with alternate administrator credentials therefore uses that administrator DPAPI scope, not the unelevated desktop user scope.'
    } | ConvertTo-Json -Depth 5
    if (-not $passed) { throw 'Identity validation assertions failed' }
} catch {
    $failure = [ordered]@{
        passed = $false
        error = $_.Exception.Message
        details = ($_ | Out-String)
    } | ConvertTo-Json -Depth 4
} finally {
    $ErrorActionPreference = 'SilentlyContinue'
    & schtasks.exe /Delete /TN $task /F 2>$null | Out-Null
    if ($batchRightGranted) { [LsaRights]::Set($userSid, 'SeBatchLogonRight', $false) }
    Remove-Item -Force $publicPayload -ErrorAction SilentlyContinue
    Remove-Item -Force $childError -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $output -ErrorAction SilentlyContinue
    Remove-LocalUser -Name $user -ErrorAction SilentlyContinue
    $profilePath = "C:\Users\$user"
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        $profile = Get-CimInstance Win32_UserProfile | Where-Object { $_.LocalPath -eq $profilePath }
        if (-not $profile) { break }
        if (-not $profile.Loaded) {
            $profile | Remove-CimInstance
            break
        }
        Start-Sleep -Milliseconds 500
    }
    $profile = Get-CimInstance Win32_UserProfile | Where-Object { $_.LocalPath -eq $profilePath }
    if ($profile -and $profile.Loaded) {
        & reg.exe unload "HKU\$($profile.SID)_Classes" 2>$null | Out-Null
        & reg.exe unload "HKU\$($profile.SID)" 2>$null | Out-Null
        Start-Sleep -Seconds 1
    }
    Get-CimInstance Win32_UserProfile | Where-Object { $_.LocalPath -eq $profilePath -and -not $_.Loaded } | Remove-CimInstance
    Remove-Item -Recurse -Force $profilePath -ErrorAction SilentlyContinue
}
if ($failure) {
    $failure
    exit 1
}
