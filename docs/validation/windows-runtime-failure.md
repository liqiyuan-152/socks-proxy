# Windows 内核运行故障与恢复验证

日期：2026-09-20 至 2026-09-21。平台：Windows 11 Pro 10.0.26200 x64，以及 Windows 10 Pro 10.0.19045 x64 Hyper-V VM。内核：`1.14.1-socks-proxy.2`。

`CoreSupervisor::poll` 直接监管 `ProcessCoreBackend`。运行中进程退出后清除实际应用模式，进入 `Error`，保留期望模式及持久化的最后成功模式，并设置“流量可能直连”。生产 `SharedController` 每次生成 UI 状态时调用运行时轮询，因此窗口和托盘读取到的是受管内核的实际存活状态。用户可重试最后成功模式；显式完成直连恢复后可调用直连恢复入口清除错误和告警。应用事务控制器提供相同的退出观察与重试语义，异常观察及重试不写配置。

Windows 验证程序启动真实内核并确认就绪，随后从外部强制终止进程。监管轮询捕获退出后验证 Error、无已应用模式、Rules 期望模式保留及可能直连提示；再使用同一严格缓存重启，恢复 Rules；最后停止内核并标记显式直连恢复。

```json
{"exit_detected":true,"error_visible":true,"traffic_may_be_direct":true,"last_mode_preserved":true,"retry_rules":true,"direct_recovery":true}
```

Windows 10 使用 SHA-256 `5a25342e23a83917c6ddefdaf6d12a08558a17a7d4a3d7613ac736f5c2012430` 的验证程序复跑，结果一致。进程层验证同时确认锁定版本、隐藏启动、就绪、停止和受限临时配置清理；机器可读结果见 `evidence/windows10-storage-recovery-results.json`。

## 宿主进程崩溃

2026-09-21 在同一 Windows 11 x64 环境运行 `scripts/validation/windows-host-crash-smoke.ps1`。验证程序 SHA-256 为 `fdc458426b713fa318053af2c7b47da89e48f4bccc5088073f5af4cd382f26ab`，受管内核 SHA-256 为 `40a64f2973858203468da544db40c6d546f14fd8f02ffa38954e16bb776927a1`。

`ProcessCoreBackend` 将内核加入启用 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 Windows Job Object。脚本启动真实 SOCKS 上游和 Rules 模式 TUN，按父进程 ID 找到受管内核并确认默认路由经过 `socks-proxy`，随后从外部强制终止宿主。宿主句柄关闭后受管内核同步终止，TUN 适配器消失。再次启动读取持久化的最后成功 Rules 模式并成功恢复；最后由用户语义的显式 Direct 操作停止接管并持久化 Direct。

```json
{"hostTerminated":true,"childTerminatedByJob":true,"tunWasActive":true,"tunRemovedAfterCrash":true,"trafficMayBeDirectBoundary":true,"nextLaunchRestoredRules":true,"explicitDirect":true,"lastModeDirect":true}
```

结果保存在 `evidence/windows11-host-crash-results.json`。Job Object 负责避免宿主退出后遗留内核和 TUN，但不构成系统级流量阻断；从宿主退出到 TUN 清理及重启恢复期间仍遵循“流量可能直连”的 MVP 边界。

上游故障无直连 fallback 由 `windows-managed-routing-smoke.ps1` 验证：命中域名在上游不可达或代理端解析失败时均失败并保留代理出站证据；同一时刻未命中域名仍通过独立 DNS 直连。结果见 `evidence/windows11-managed-routing-results.json`。

最终资源清理检查确认验证目录内核进程、测试监听、测试适配器和测试路由均为 0。MVP 不提供内核或宿主退出后的系统级阻断，故障窗口明确提示可能直连。
