param(
    [Parameter(Mandatory = $true)][string]$Sid,
    [Parameter(Mandatory = $true)][string]$ProfilePath,
    [Parameter(Mandatory = $true)][string]$Marker
)

$ErrorActionPreference = 'SilentlyContinue'
& reg.exe unload "HKU\${Sid}_Classes" 2>$null | Out-Null
& reg.exe unload "HKU\$Sid" 2>$null | Out-Null
Start-Sleep -Seconds 1
Get-CimInstance Win32_UserProfile | Where-Object {
    $_.SID -eq $Sid -and $_.LocalPath -eq $ProfilePath -and -not $_.Loaded
} | Remove-CimInstance
Remove-Item -Recurse -Force $ProfilePath -ErrorAction SilentlyContinue
[ordered]@{
    profileExists = $null -ne (Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $Sid })
    directoryExists = Test-Path $ProfilePath
} | ConvertTo-Json | Set-Content -Encoding UTF8 $Marker
