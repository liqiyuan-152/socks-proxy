param(
    [ValidateSet('Windows 10', 'Windows 11')]
    [string]$Windows = 'Windows 11',
    [string]$Fido = (Join-Path $env:USERPROFILE 'socks-proxy-validation\Fido.ps1'),
    [string]$DestinationRoot = 'G:\socks-proxy-clean-vm',
    [string]$SourceUrl
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$release = if ($Windows -eq 'Windows 10') { '22H2' } else { 'Latest' }
$fileName = if ($Windows -eq 'Windows 10') { 'windows-10-22h2-x64.iso' } else { 'windows-11-latest-x64.iso' }
$displayName = "socks-proxy-$($Windows.Replace(' ', '-').ToLowerInvariant())-iso"
$destination = Join-Path $DestinationRoot $fileName

New-Item -ItemType Directory -Path $DestinationRoot -Force | Out-Null
if (Test-Path $destination) {
    [pscustomobject]@{ state = 'Transferred'; path = $destination; existing = $true } | ConvertTo-Json -Compress
    exit 0
}

$existing = Get-BitsTransfer -AllUsers -ErrorAction SilentlyContinue |
    Where-Object DisplayName -eq $displayName |
    Select-Object -First 1
if ($existing) {
    if ($existing.JobState -eq 'Transferred') { Complete-BitsTransfer -BitsJob $existing }
    [pscustomobject]@{
        state = [string]$existing.JobState
        path = $destination
        bytesTransferred = $existing.BytesTransferred
        bytesTotal = $existing.BytesTotal
        existing = $true
    } | ConvertTo-Json -Compress
    exit 0
}

$url = $SourceUrl
if (-not $url) {
    $url = & $Fido -Win $Windows -Rel $release -Ed Pro -Lang English -Arch x64 -GetUrl
}
if (-not $url) { throw "Fido did not return a URL for $Windows" }
$job = Start-BitsTransfer -Source $url.Trim() -Destination $destination -DisplayName $displayName -Asynchronous
[pscustomobject]@{
    state = [string]$job.JobState
    path = $destination
    bytesTransferred = $job.BytesTransferred
    bytesTotal = $job.BytesTotal
    existing = $false
} | ConvertTo-Json -Compress
