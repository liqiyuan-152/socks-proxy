$ErrorActionPreference = 'Continue'
$profilePath = 'C:\Users\socksproxym0'
$profile = Get-CimInstance Win32_UserProfile | Where-Object { $_.LocalPath -eq $profilePath }
$results = @()
if ($profile) {
    foreach ($key in @("HKU\$($profile.SID)_Classes", "HKU\$($profile.SID)")) {
        $text = (& reg.exe unload $key 2>&1 | Out-String).Trim()
        $results += [ordered]@{ key = $key; exitCode = $LASTEXITCODE; output = $text }
    }
}
[ordered]@{
    profile = $profile | Select-Object LocalPath, Loaded, SID, Status
    unloadResults = $results
} | ConvertTo-Json -Depth 5
