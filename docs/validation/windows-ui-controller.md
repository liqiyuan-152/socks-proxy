# Windows 共享 UI 控制器验证

验证日期：2026-09-21。平台：Windows 11 Pro 10.0.26200 x64，以及 Windows 10 Pro 10.0.19045 x64 Hyper-V VM；内核：`1.14.1-socks-proxy.2`。

首页与代理管理现在只读取 `SharedController` 的 `UiState`。模式、当前代理、配置修订和错误状态均来自同一个 `ApplicationController` 事务；UI 不再维护独立演示状态。每次读取状态先轮询 `ManagedCoreRuntime` 的受管进程；意外退出会清除已应用模式、进入 Error 并显示“流量可能直连”，同时保留最后成功模式。Windows 入口依次执行 DPAPI 凭据读取、配置编译、锁定内核 `check`、进程启动和就绪确认。启动恢复失败时保留持久化的期望模式，但不会把它显示为已应用或 Running。

`scripts/validation/windows-ui-controller-smoke.ps1` 运行交叉链接的 `windows_ui_controller_smoke.exe`。程序启动两个真实本地 SOCKS5 上游 A/B，各自把同一公共测试目标转到返回 `A`/`B` 的独立 HTTP fixture。共享 UI 控制器依次执行规则代理、当前代理切换、全局代理和全局直连，响应正文证明每个新连接的实际路径。最后破坏严格 FakeIP 缓存并再次请求规则代理，验证错误可见且已应用状态仍为直连。

实际结果：

```json
变更前历史结果：`{"rules_confirmation":true,"rules_path_a":true,"rule_change_confirmation":true,"proxy_confirmation":true,"path_b":true,"global_confirmation":true,"global_path_b":true,"rule_log":true,"unknown_log":true,"failure_log":true,"clear_kept_proxy":true,"failure_visible":true,"failure_kept_direct":true}`。重新验收必须改为验证 `rules_switched_without_confirmation` 与 `global_switched_without_confirmation`，同时保留规则和代理变更确认检查。
```

同一最终验证程序在 Windows 10 连续执行两次，结果均与上方一致。验证程序 SHA-256 为 `d0fefe4ab17554a6c2ec8fc84419b9835914a0ce82ab76dd132ee82eaf6f69a63`，内核 SHA-256 为 `40a64f2973858203468da544db40c6d546f14fd8f02ffa389544e16bb776927a1`。每次生产 TUN 停止后，监管器按连接名删除本程序创建的 Wintun PnP 设备；最终 `sing-box` 进程、`socks-proxy-tun-v1`/旧 `socks-proxy` 网络连接和 Wintun PnP 设备计数均为 0。机器可读结果见 `docs/validation/evidence/windows10-ui-controller-results.json`。

验证同时确认：

1. 规则、全局和代理 A/B 切换在重启内核前均要求中断确认。
2. 编辑当前运行代理复用同一确认流程；确认前配置和运行状态不变。
3. 新建或编辑非运行代理不触发不必要的内核重启。
4. 凭据只以不可变引用进入普通配置，代理草稿关闭后覆盖密码内存。
5. 真实内核拒绝配置或启动失败时，控制器不提交候选配置。
6. `cargo test --locked` 共 74 个测试通过；格式、Clippy `-D warnings` 和 Windows MSVC 全目标检查通过。

macOS 上还实际启动了 eframe 原生窗口，复查五页面导航、首页三模式、当前代理、修订号、错误区域和底部状态栏。窗口正常渲染，无 WebView、空白画布、文本截断或控件重叠。
