# Windows 身份、DPAPI 与 ACL 验证

日期：2026-09-20 至 2026-09-21。环境：Windows 11 x64 `10.0.26200`，以及 Windows 10 Pro x64 `10.0.19045` Hyper-V VM。测试数据均为固定无效字符串，不使用真实代理凭据。

## 方法

`windows-identity-smoke.ps1` 创建临时标准账户 `socksproxym0`，仅在测试期间通过 LSA 授予 `SeBatchLogonRight`，以计划任务启动 `windows-identity-user.ps1`。子进程使用 CurrentUser DPAPI 生成密文并创建受限临时文件。管理员与标准账户再分别尝试解密对方密文。脚本最终撤销登录权并删除账户、任务和公共测试文件。

## 结果

最终完整功能运行返回 `passed: true`：

- 提升后有效身份：`LIQIYUAN\lqy15`。
- 标准身份：`LIQIYUAN\socksproxym0`。
- 标准账户自身 DPAPI 往返成功。
- 管理员无法解密标准账户密文；标准账户无法解密管理员密文。
- 临时文件所有者为标准账户，ACL 仅包含标准账户和 `NT AUTHORITY\SYSTEM`，没有额外主体。

结论：用户范围 DPAPI 归属有效进程账户。标准桌面用户输入另一个管理员账户完成 UAC/凭据启动时，应用进程、配置目录和 DPAPI 数据归属该管理员账户，不得在界面中冒认原标准桌面用户，也不能静默读取原用户凭据。

Rust `DpapiCredentialVault` 使用 `CryptProtectData` / `CryptUnprotectData` 的 CurrentUser 默认范围及 `CRYPTPROTECT_UI_FORBIDDEN`，凭据采用不可变 UUID 引用，普通配置不保存密文或明文；缺失和当前账户无法解密分别报告。Windows MSVC 目标交叉检查通过。

项目代码使用 `cargo-xwin 0.23.1` 构建 `examples/dpapi_smoke.rs` 并在上述 Windows 主机运行，返回：`passed=true`、`roundTrip=true`、`containsPlaintext=false`、`deleted=true`、`missingReported=true`。测试目录由包装脚本清理。该结果验证实际 Rust 实现，而不只验证 PowerShell 的 DPAPI 语义。

## 清理边界

账户、计划任务、`SeBatchLogonRight` 和公共测试文件已清理。Windows User Profile Service 在批处理任务结束后仍把测试 SID `S-1-5-21-2116990933-238765365-2118519739-1009` 标记为 Loaded；管理员及一次性 SYSTEM 清理任务执行 `reg unload` 均无法卸载，因此 `C:\Users\socksproxym0` 配置目录尚待系统重启后删除。没有该 SID 的存活进程。为避免中断现有会话，本轮未重启 User Profile Service 或机器。

复现脚本：

- `scripts/validation/windows-identity-user.ps1`
- `scripts/validation/windows-identity-smoke.ps1`
- `scripts/validation/windows-identity-cleanup-check.ps1`
- `scripts/validation/windows-identity-system-cleanup.ps1`

后续测试应复用固定测试账户或在测试 VM 快照内运行，避免重复生成待重启清理的用户配置。

Windows 10 复跑同时验证当前用户 DPAPI 往返、密文不含明文、损坏数据拒绝和受限 ACL。跨账户结果与 Windows 11 一致。测试后重启 VM 释放标准用户配置单元，再由 SYSTEM 删除受限公共目录；最终临时用户、用户配置、公共目录和计划任务均不存在。机器可读结果见 `evidence/windows10-identity-results.json`。
