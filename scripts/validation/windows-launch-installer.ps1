param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation')
)

$ErrorActionPreference = 'Stop'
$installer = Join-Path $Workspace 'installer-output\socks-proxy-0.1.0-windows-x64-setup.exe'

if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
    throw "Installer is missing: $installer"
}

$process = Start-Process -FilePath $installer -WorkingDirectory (Split-Path $installer) -PassThru -Wait
exit $process.ExitCode
