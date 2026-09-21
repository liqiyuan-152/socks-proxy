$ErrorActionPreference = 'SilentlyContinue'
Get-NetAdapter -Physical | Where-Object {
    $_.InterfaceDescription -in @(
        'Intel(R) Wi-Fi 6 AX201 160MHz',
        'Realtek Gaming 2.5GbE Family Controller'
    )
} | Enable-NetAdapter -Confirm:$false
