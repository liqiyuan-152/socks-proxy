# Windows 启动验证

最终版本验证于 2026-09-21 在全新 Windows 10 Pro 10.0.19045 x64 Hyper-V VM 完成。应用 SHA-256 为 `7587f23e4263421a81aac9aa6ff65774495c02e999a29ef270aac57cfd2f1f3d`。

Observed result from `scripts/validation/windows-startup-smoke.ps1`:

```json
{"manifestRequiresAdministrator":true,"singleInstance":true,"consoleWindow":false,"validationNetworkUnchanged":true}
```

The main executable embeds a `requireAdministrator` manifest as an RT_MANIFEST
resource. The dedicated GUI-subsystem smoke executable held the application's
named mutex, launched a second copy, and observed the dedicated already-running
result. `GetConsoleWindow` returned null. The script also compared sing-box
process counts and the Windows route table before and after validation.

最终安装器另从非提升的交互 Session 1 启动，确认 `consent.exe` 正在安全桌面等待后，通过 Hyper-V 合成键盘发送 Escape。Windows 返回 “The operation was canceled by the user.”，应用、内核和 TUN 前后均为 0，路由和 DNS 不变。

同一 Session 1 中启动主实例后再次启动应用，第二实例在 5 秒内以退出码 0 结束；主实例和托盘始终只有一个，内核仍为 0，网络不变。完整结果见 `evidence/windows10-final-startup-results.json`。

Windows 11 实机从非提升交互 Session 12 启动最终应用，并在安全桌面真实按 Escape。启动方收到系统本地化的“操作已被用户取消”及 Win32 错误码 1223；应用、内核、TUN 和所有权路由前后均为 0，路由与 DNS 不变。机器可读结果见 `evidence/windows11-uac-cancel-results.json`。此前通过 SSH 自动注入或强制终止 `consent.exe` 的尝试均未计入证据。
