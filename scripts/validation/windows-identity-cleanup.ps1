$ErrorActionPreference = 'SilentlyContinue'
$profilePath = 'C:\Users\socksproxym0'
& schtasks.exe /Delete /TN SocksProxyM0Identity /F 2>$null | Out-Null
Remove-LocalUser -Name 'socksproxym0' -ErrorAction SilentlyContinue
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
Remove-Item -Force (Join-Path $env:PUBLIC 'sp-i.ps1') -ErrorAction SilentlyContinue
Remove-Item -Force (Join-Path $env:PUBLIC 'sp-i-error.txt') -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force (Join-Path $env:PUBLIC 'sp-o') -ErrorAction SilentlyContinue
[ordered]@{
    profile = @(Get-CimInstance Win32_UserProfile | Where-Object { $_.LocalPath -eq $profilePath } | Select-Object LocalPath, Loaded, SID, Status)
    directoryExists = Test-Path $profilePath
    directoryAcl = if (Test-Path $profilePath) { (Get-Acl $profilePath).Sddl } else { $null }
    ownedProcesses = @(
        Get-CimInstance Win32_Process | ForEach-Object {
            $owner = Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid -ErrorAction SilentlyContinue
            if ($profile -and $owner.Sid -eq $profile.SID) {
                [ordered]@{ processId = $_.ProcessId; name = $_.Name; commandLine = $_.CommandLine }
            }
        }
    )
    registryHives = @(
        Get-ChildItem Registry::HKEY_USERS | Where-Object { $profile -and $_.PSChildName -like "$($profile.SID)*" } | Select-Object -ExpandProperty PSChildName
    )
} | ConvertTo-Json -Depth 4
