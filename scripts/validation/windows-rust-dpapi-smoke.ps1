$ErrorActionPreference = 'Stop'
$root = Join-Path $env:USERPROFILE 'socks-proxy-validation'
$exe = Join-Path $root 'dpapi_smoke.exe'
$directory = Join-Path $root 'rust-dpapi-smoke'
if (-not (Test-Path $exe)) { throw "Missing executable: $exe" }
try {
    $output = & $exe $directory
    $exitCode = $LASTEXITCODE
    $output
    if ($exitCode -ne 0) { exit $exitCode }
} finally {
    Remove-Item -Recurse -Force $directory -ErrorAction SilentlyContinue
}
