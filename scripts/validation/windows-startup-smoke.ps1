param(
    [string]$Workspace = (Join-Path $env:USERPROFILE 'socks-proxy-validation'),
    [string]$Application = '',
    [string]$StartupSmoke = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (-not $Application) { $Application = Join-Path $Workspace 'socks-proxy.exe' }
if (-not $StartupSmoke) { $StartupSmoke = Join-Path $Workspace 'windows_startup_smoke.exe' }
foreach ($path in @($Application, $StartupSmoke)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing executable: $path" }
}

$beforeCore = @(Get-Process -Name 'sing-box*' -ErrorAction SilentlyContinue).Count
$beforeRoutes = @(Get-NetRoute | ForEach-Object { "$($_.DestinationPrefix)|$($_.NextHop)|$($_.InterfaceIndex)" })
$startup = & $StartupSmoke | ConvertFrom-Json
if (-not $startup.single_instance -or $startup.console_window) {
    throw 'Startup smoke returned an invalid result'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ManifestReader {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr LoadLibraryEx(string file, IntPtr fileHandle, uint flags);
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr FindResource(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint SizeofResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr LockResource(IntPtr resource);
    [DllImport("kernel32.dll")] static extern bool FreeLibrary(IntPtr module);
    public static string Read(string path) {
        IntPtr module = LoadLibraryEx(path, IntPtr.Zero, 2);
        if (module == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
        try {
            IntPtr found = FindResource(module, (IntPtr)1, (IntPtr)24);
            if (found == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
            uint size = SizeofResource(module, found);
            IntPtr loaded = LoadResource(module, found);
            IntPtr locked = LockResource(loaded);
            byte[] bytes = new byte[size];
            Marshal.Copy(locked, bytes, 0, (int)size);
            return System.Text.Encoding.UTF8.GetString(bytes);
        } finally { FreeLibrary(module); }
    }
}
'@
$manifest = [ManifestReader]::Read($Application)
$manifestOk = $manifest.Contains('level="requireAdministrator"') -and $manifest.Contains('uiAccess="false"')
if (-not $manifestOk) { throw 'Application manifest does not require administrator elevation' }

$afterCore = @(Get-Process -Name 'sing-box*' -ErrorAction SilentlyContinue).Count
$afterRoutes = @(Get-NetRoute | ForEach-Object { "$($_.DestinationPrefix)|$($_.NextHop)|$($_.InterfaceIndex)" })
$networkUnchanged = $beforeCore -eq $afterCore -and -not (Compare-Object $beforeRoutes $afterRoutes)
if (-not $networkUnchanged) { throw 'Startup validation unexpectedly changed core processes or routes' }

[pscustomobject]@{
    manifestRequiresAdministrator = $manifestOk
    singleInstance = [bool]$startup.single_instance
    consoleWindow = [bool]$startup.console_window
    validationNetworkUnchanged = $networkUnchanged
} | ConvertTo-Json -Compress
