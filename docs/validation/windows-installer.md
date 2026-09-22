# Windows 安装包与离线恢复验证

日期：2026-09-21。最终干净机平台：Windows 10 Pro 10.0.19045 x64 Hyper-V VM；安装前确认应用文件与卸载注册记录均不存在，并创建 `Before-Socks-Proxy-0.1.0-Final` 快照。安装器：Inno Setup 6.7.3。最终安装包：`dist/windows/socks-proxy-0.1.0-windows-x64-setup.exe`，SHA-256 `6e7d0fbb81a36d4b0e7d3cc9ebfda4fa7c0d3590a2b1f6543fbea8efd9d0d11e`；内含应用 SHA-256 `70a214f1349a26f72f5db94d0f655f633b39ce005184932a506e0ddbcdda6ad7`。

安装包包含 release 模式的 `socks-proxy.exe`、固定的 `sing-box.exe`、开始菜单入口、使用与恢复说明、67 项内核依赖许可证清单，以及对应的 sing-box 源码包、补丁、构建脚本和版本锁。内核安装后 SHA-256 与锁定值 `40a64f2973858203468da544db40c6d546f14fd8f02ffa38954e16bb776927a1` 一致。

## 本变更新的候选安装输入

当前变更的 Windows x64 release 由 `scripts/release/prepare-windows-installer-input.sh` 组装为 Inno Setup 输入目录。默认输出为 `.work/windows-installer/package`；在 Windows 上将该目录复制为 `windows/package` 后，用 Inno Setup 6.7.3 编译 `windows/installer.iss`。

```text
scripts/release/prepare-windows-installer-input.sh
# 将 .work/windows-installer/package 复制为 Windows 工作树中的 windows/package
ISCC.exe windows\installer.iss
PowerShell -ExecutionPolicy Bypass -File scripts\validation\windows-installer-smoke.ps1 -Installer windows\output\socks-proxy-0.1.0-windows-x64-setup.exe
```

准备脚本在输入目录生成 `SHA256SUMS`，其中必须包含应用、内核、文档、许可证清单和对应 sing-box 源码材料。本次候选应用 SHA-256 为 `821909fb6c8963aa5045d85b586ce65ccaccc81aa972faf13f6af086726677f9`。该候选尚未完成 Windows 安装、升级与卸载验收，不能替代下述历史结果。

`scripts/validation/windows-installer-smoke.ps1` 在 Windows 上完成以下检查：

1. 静默安装并核对程序、内核、文档、许可证、源码材料、卸载器和开始菜单快捷方式。
2. 解析 PE 头确认应用使用 Windows GUI 子系统；检查嵌入的 `requireAdministrator` 清单，并确认没有 WebView2 loader。
3. 在隔离的 `%LOCALAPPDATA%` 下运行 `socks-proxy.exe --recover-direct`，不依赖 Rust/Cargo 或源代码完成显式直连恢复。
4. 再次运行同一安装包，验证同版本升级保留并替换为相同应用负载。
5. 静默卸载，确认程序文件、开始菜单入口、内核进程和 `socks-proxy` TUN 适配器均为 0。

```json
{"installedPayload":true,"startMenuEntry":true,"coreHashVerified":true,"uacManifest":true,"guiSubsystem":true,"webViewAbsent":true,"dynamicMsvcRuntimeAbsent":true,"recoveryCommand":true,"sameVersionUpgrade":true,"uninstallRemovedFiles":true,"uninstallRemovedShortcut":true,"coreProcessesAfterUninstall":0,"tunAdaptersAfterUninstall":0}
```

最终干净机结果保存在 `evidence/windows10-final-installer-results.json`。托盘同步修复后的最终安装包又在 Windows 11 实机完整重跑安装、恢复命令、同版本升级和卸载；应用、内核、TUN/PnP、所有权路由和临时任务最终均为 0，结果见 `evidence/windows11-final-installer-results.json`。

Windows 11 解锁桌面后从开始菜单启动最终安装版，UI Automation 观测到一个应用进程、一个 `Socks Proxy` 窗口和一个托盘图标，控制台窗口为 0，WebView2 进程数保持 18。`PrintWindow` 取得的真实窗口画面显示五页面导航、三种模式和 Direct 运行状态均正常渲染；结果见 `evidence/windows11-installed-launch-results.json` 和 `evidence/windows11-installed-launch.png`。

Windows 10 验证机没有 Cargo，恢复步骤只使用已安装程序和 Windows 内置命令，满足使用文档的无开发环境恢复演练。最终应用使用静态 MSVC CRT，并在 Hyper-V 基础显示驱动上通过 wgpu 正常显示，不依赖 OpenGL 2.0。安装包尚未签名，UAC 会显示未知发布者。
