param(
    [string]$Installer = (Join-Path $env:USERPROFILE 'socks-proxy-installer-build\output\socks-proxy-0.1.0-windows-x64-setup.exe'),
    [string]$ExpectedCoreHash = '40a64f2973858203468da544db40c6d546f14fd8f02ffa38954e16bb776927a1'
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$installDir = Join-Path $env:ProgramFiles 'Socks Proxy'
$application = Join-Path $installDir 'socks-proxy.exe'
$core = Join-Path $installDir 'sing-box.exe'
$startMenu = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Socks Proxy\Socks Proxy.lnk'
$uninstaller = Join-Path $installDir 'unins000.exe'
$fixtureData = Join-Path $env:TEMP 'socks-proxy-installer-smoke-data'

function Invoke-Setup([string[]]$Arguments) {
    $process = Start-Process -FilePath $Installer -ArgumentList $Arguments -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Installer exited with $($process.ExitCode)" }
}

function Assert-Installed {
    foreach ($path in @(
        $application,
        $core,
        (Join-Path $installDir 'docs\user-guide.md'),
        (Join-Path $installDir 'licenses\dependency-licenses.md'),
        (Join-Path $installDir 'licenses\sing-box\manifest.json'),
        (Join-Path $installDir 'source\sing-box\patched-source.tar.gz'),
        $startMenu,
        $uninstaller
    )) {
        if (-not (Test-Path $path)) { throw "Installed file is missing: $path" }
    }
    $actualCoreHash = (Get-FileHash $core -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualCoreHash -ne $ExpectedCoreHash) { throw "Core hash mismatch: $actualCoreHash" }
    $bytes = [IO.File]::ReadAllBytes($application)
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    $optionalHeader = $peOffset + 24
    $subsystem = [BitConverter]::ToUInt16($bytes, $optionalHeader + 68)
    if ($subsystem -ne 2) { throw "Application PE subsystem is not Windows GUI: $subsystem" }
    $binaryText = [Text.Encoding]::UTF8.GetString($bytes)
    if (-not $binaryText.Contains('requireAdministrator')) { throw 'Application UAC manifest is missing' }
    if ($binaryText.Contains('WebView2Loader')) { throw 'Application unexpectedly imports WebView2' }
    if ($binaryText.Contains('VCRUNTIME140.dll') -or $binaryText.Contains('api-ms-win-crt-')) {
        throw 'Application unexpectedly depends on a preinstalled dynamic MSVC runtime'
    }
}

if (-not (Test-Path $Installer)) { throw "Installer is missing: $Installer" }
Remove-Item $fixtureData -Recurse -Force -ErrorAction SilentlyContinue
Invoke-Setup @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-')
Assert-Installed
$firstHash = (Get-FileHash $application -Algorithm SHA256).Hash.ToLowerInvariant()

$oldLocalAppData = $env:LOCALAPPDATA
$oldDirectDns = $env:SOCKS_PROXY_DIRECT_DNS
try {
    $env:LOCALAPPDATA = $fixtureData
    $env:SOCKS_PROXY_DIRECT_DNS = '1.1.1.1'
    $recovery = Start-Process -FilePath $application -ArgumentList '--recover-direct' -PassThru -Wait
    if ($recovery.ExitCode -ne 0) { throw "Recovery command exited with $($recovery.ExitCode)" }
} finally {
    $env:LOCALAPPDATA = $oldLocalAppData
    $env:SOCKS_PROXY_DIRECT_DNS = $oldDirectDns
}

Invoke-Setup @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-')
Assert-Installed
$upgradeHash = (Get-FileHash $application -Algorithm SHA256).Hash.ToLowerInvariant()
if ($upgradeHash -ne $firstHash) { throw 'Same-version upgrade changed the application payload' }

$uninstall = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -PassThru -Wait
if ($uninstall.ExitCode -ne 0) { throw "Uninstaller exited with $($uninstall.ExitCode)" }
if (Test-Path $application) { throw 'Application remains after uninstall' }
if (Test-Path $startMenu) { throw 'Start menu shortcut remains after uninstall' }
$coreProcesses = @(Get-Process -Name 'sing-box' -ErrorAction SilentlyContinue).Count
$tunAdapters = @(Get-NetAdapter -Name 'socks-proxy-tun-v1' -ErrorAction SilentlyContinue).Count
if ($coreProcesses -ne 0 -or $tunAdapters -ne 0) { throw 'Uninstall left network resources behind' }

[pscustomobject]@{
    installedPayload = $true
    startMenuEntry = $true
    coreHashVerified = $true
    uacManifest = $true
    guiSubsystem = $true
    webViewAbsent = $true
    dynamicMsvcRuntimeAbsent = $true
    recoveryCommand = $true
    sameVersionUpgrade = $true
    uninstallRemovedFiles = $true
    uninstallRemovedShortcut = $true
    coreProcessesAfterUninstall = $coreProcesses
    tunAdaptersAfterUninstall = $tunAdapters
} | ConvertTo-Json -Compress
