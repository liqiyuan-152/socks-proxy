# Windows 设置与备份验证

验证日期：2026-09-21。平台：Windows 11 Pro 10.0.26200 x64，以及 Windows 10 Pro 10.0.19045 x64 Hyper-V VM；构建目标：`x86_64-pc-windows-msvc`。

设置页使用 Windows 原生文件对话框选择导入和导出路径。导入读取后先显示代理数、规则数和待补充认证数，确认按钮仅在运行状态已恢复为全局直连时可用。导出复用 storage 层的版本化备份格式，不包含用户名、密码、凭据引用或上次运行模式。开机启动复选框展示已保存偏好；MVP 默认关闭且暂不允许修改。

交叉链接并在 Windows 上执行：

```text
cargo-xwin build --locked --target x86_64-pc-windows-msvc --example windows_settings_backup_smoke
scripts/validation/windows-settings-backup-smoke.ps1
```

实际结果：

```json
{"first_launch_direct":true,"startup_disabled":true,"secret_free":true,"preview_missing_credentials":true,"import_applied_direct":true}
```

验证覆盖：

1. 新配置的持久模式和实际模式均为全局直连。
2. `start_with_windows` 默认值为 `false`。
3. 认证代理的导出保留 `auth_enabled`，但不含测试用户名、测试密码、`credential_ref` 或 `last_applied_mode`。
4. 导入预览正确报告 1 个代理、1 条规则和 1 个待补充认证。
5. 整体导入更新代理和规则后仍保持全局直连，不从备份自动启动代理。
6. 控制器回归测试额外验证代理模式中拒绝导入，以及损坏备份不会改变旧配置。

验证程序：`examples/windows_settings_backup_smoke.rs`。PowerShell 驱动：`scripts/validation/windows-settings-backup-smoke.ps1`。

Windows 10 使用 SHA-256 `837977bf417bba207a73ada122775b1f12d11139b2f999be791d22cb68abb9af` 的验证程序复跑，五项结果全部为 `true`。机器可读结果见 `evidence/windows10-storage-recovery-results.json`。
