# Windows 网络所有权与恢复验证

日期：2026-09-20 至 2026-09-21。平台：Windows 11 Pro 10.0.26200 x64，以及 Windows 10 Pro 10.0.19045 x64 Hyper-V VM。

`NetworkRecovery` 在修改前读取资源原值，将资源、原值和本程序将写入的值原子保存到版本化恢复记录，然后才执行写入。恢复时重新读取当前值：仍等于本程序写入值才恢复原值；已经等于原值视为幂等完成；与两者均不同则认定为其他网络工具的新设置并保留。失败资源继续留在记录中，进程重启后可重试。

所有资源恢复完成后单独刷新 Windows DNS 缓存。刷新失败保留仅含刷新状态的恢复记录，不宣称恢复全部完成；下一次重试无需再次改写网络资源。报告始终把系统恢复和“应用可能仍持有私有 DNS/FakeIP 缓存，需要重新解析或重启应用”的提示分开。

Windows 验证覆盖原子记录替换、模拟恢复写入失败、进程重建后的幂等重试、外部 VPN 式 DNS 新值保留、模拟 DNS 刷新失败及随后真实执行 `ipconfig /flushdns`：

```json
{"external_setting_preserved":true,"restore_retry":true,"dns_flush_retry":true,"application_cache_notice":true,"journal_cleared":true,"last_mode_unchanged":true}
```

FakeIP 兼容重启、原映射保留、未知合成地址拒绝及损坏缓存不改写由 `.2` 完整严格缓存回归验证；见 `strict-cache-patch.md` 和 `windows-managed-routing.md`。受限 TUN 测试强制终止后的测试路由清除、显式直连基线恢复和最终零残留检查均已通过。

Windows 10 进一步使用生产 `WindowsRuntime` 验证 FakeIP 后的正常关停路径：关停前解析得到 `198.18.0.2` 和 `fc00::2`，确认中断提示后停止接管并执行系统 DNS 刷新，再次解析得到真实地址 `172.66.147.243` 和 `104.20.23.154`。系统已恢复与应用可能保留私有缓存的提示分别报告，持久化的 Rules 最后成功模式没有被关停改写。

随后在交互 Session 1 中以 Rules 和 `Exit fixture` 启动最终 release GUI，通过真实托盘“退出”。退出后应用、sing-box、本程序 TUN/PnP 设备及所有权路由计数均为 0，`last_applied_mode` 仍为 `rules`。

证据：`evidence/windows11-network-recovery-results.json`、`evidence/windows10-storage-recovery-results.json`、`evidence/windows10-shutdown-dns-results.json`。Windows 10 复跑覆盖外部设置保留、恢复重试、DNS 刷新重试、应用缓存提示、记录清理、最后模式不变、FakeIP 后重新解析和真实代理模式托盘退出。
