# Windows 网络切换与 SSH 长连接验收

日期：2026-09-21。平台：Windows 11 Pro 10.0.26200 x64 实机和 Windows 10 Pro 10.0.19045 x64 Hyper-V VM。Windows 11 当前物理网络为 Wi-Fi，Realtek 有线网卡未连接。

## SSH 长连接切换

`scripts/validation/windows-ssh-long-session-lan-smoke.ps1` 使用一次性 Ed25519 密钥连接独立 LAN SSH 端点。客户端目标使用规格中的远程内网地址 `10.20.30.40:22`；Rules/TUN 将其交给当前 SOCKS 上游，上游再把受控 fixture 目标覆写到实际 LAN SSH 服务。两套上游分别代表代理 A/B。

验证先确认 Direct 基线，再通过生产 `ManagedCoreRuntime` 启用 Rules 和代理 A，建立输出 `READY` 后保持 30 秒的 SSH 会话。未确认切换会返回中断确认要求；确认切到代理 B 后，内核和 TUN 重启，现有 SSH 被远端重置。随后新 SSH 会话输出 `AFTER` 并正常退出。上游 A/B 的实际日志分别出现 `inbound connection to 10.20.30.40:22`，证明切换前后路径；最后显式 Direct 清理。

```json
{"explicit_direct":true,"long_active_before_switch":true,"long_disconnected_on_restart":true,"long_survived_restart":false,"new_ssh_after_switch":true,"proxy_confirmation":true,"proxy_rule_events":0,"rules_confirmation":true,"upstream_a_seen":true,"upstream_b_seen":true}
```

实际边界是：当前实现的代理/有效配置切换采用受控重启，现有 SSH 不保证存活；确认提示准确。切换成功只保证新连接使用新路径。此次 SSH 行为由真实客户端和两套真实上游日志确认；`proxy_rule_events=0` 说明连接日志适配器没有为这两条 SSH 生命周期形成完整事件，不把该字段伪报为规则日志证据。规则日志能力由独立的连接日志验收覆盖。

测试结束后已撤销 Mac 的一次性授权公钥，删除 Windows 私钥、临时容器镜像和容器，停止本轮启动的 Docker Desktop；网络清理检查为零残留。

Windows 10 使用同一生产 `ManagedCoreRuntime`、真实 TUN、两套 SOCKS 上游和独立 LAN SSH 端点复跑。首次运行在 A→B 后已到达 B 入站，但 B 到 SSH fixture 的直连拨号在 5 秒后超时；清理后 VM 到 fixture 的 TCP 22 立即恢复。未修改代码或环境，随后三次连续运行均完整通过：旧 SSH 在受控重启时断开，新 SSH 经 B 输出 `AFTER`，最后显式 Direct，且每轮结束后内核、helper 和 Wintun 残留均为 0。该瞬时失败保留在证据中，不作为连续成功结果的一部分。

Win10 机器可读结果见 `evidence/windows10-ssh-long-session-results.json`。验证后已从 VM、Windows 宿主和 Mac 删除一次性私钥，恢复 Mac 原 `authorized_keys`。

## VPN 共存

Windows 已安装 OpenVPN 及 `OpenVPN TAP-Windows6`。Mac OpenVPN 2.6.14 在 LAN 高端口建立一次性静态测试通道，本机使用 `dev null`、`ifconfig-noexec` 和 `route-noexec`，不修改 Mac 网络；Windows OpenVPN 创建真实 TAP 地址和 `203.0.113.0/24` 专用测试路由。

在 VPN 进程、适配器和路由均活动时运行生产共享控制器的完整 Windows smoke：Rules 通过代理 A、切换代理 B、Global、上游故障及显式 Direct。全过程中 VPN 进程未退出、TAP 保持 Up、VPN 路由保持存在；应用结束后自身 `socks-proxy` TUN 已移除，证明应用恢复没有覆盖 VPN 拥有的资源。

```json
{"vpnConnectedBeforeApp":true,"vpnProcessSurvived":true,"vpnAdapterSurvived":true,"vpnRouteSurvived":true,"rulesPath":true,"proxySwitchPath":true,"globalPath":true,"applicationTunRemoved":true}
```

停止测试 VPN 后 OpenVPN 进程为 0；强制终止客户端遗留一条测试路由，本轮按测试所有权显式删除，最终测试路由和应用 TUN 均为 0。一次性静态密钥已从两端删除。该遗留属于测试 VPN 自身，不是 Socks Proxy 删除或创建的资源。

## Wi-Fi 断开与重连

Windows 11 实机没有可用有线网络，因此使用 `scripts/validation/windows-wifi-reconnect-transition.ps1` 在本机计划任务中执行 Wi-Fi 断开与重连，避免 SSH 管理链路中断导致测试流程失去控制。测试前注册独立恢复任务，异常时自动重新启用 Wi-Fi；正常完成后恢复任务自行删除。

断开前 Wi-Fi 为 `Up`，地址为 `10.168.1.158`，到受控 SSH 端点 `10.168.1.136:22` 的 TCP 探测成功。断开阶段适配器为 `Disabled`、地址消失、TCP 探测失败；重连后适配器恢复 `Up`、原地址恢复、TCP 探测重新成功。全过程 Socks Proxy 应用进程保持为 1，证明网络丢失与恢复没有重复启动应用。测试结束后临时计划任务为 0，应用内核、TUN 和所有权路由均无残留。

机器可读结果见 `evidence/windows11-wifi-reconnect-transition-results.json`。

## 环境限制

- Wi-Fi 到有线及有线到 Wi-Fi 漫游：验证机当前没有已连接的有线网络，未执行、未宣称通过；禁用唯一管理链路不能替代真实的双网络漫游。
- 恢复失败：模拟写入失败、DNS 刷新失败、进程重建重试和外部修改保留已完成，见 `windows-network-recovery.md`。

已新增 `scripts/validation/windows-physical-network-transition.ps1` 和独立兜底脚本 `windows-physical-network-recovery.ps1`。执行器要求 Wi-Fi、有线均已连接且恰有一个 Socks Proxy 进程，随后在 Windows 本机计划任务内依次验证仅有线、仅 Wi-Fi 和双网卡恢复状态；每阶段记录实际选路、TCP 22 探测、应用/内核/TUN/所有权路由计数。执行前注册 SYSTEM 级三分钟兜底任务，异常时重新启用两块物理网卡。当前实机预检因 Realtek 网卡为 `Disconnected` 正确拒绝，未修改网卡且未遗留恢复任务。

任务 7.2 已完成 SSH 长连接切换、当前可用物理网络的断开/重连、活动 VPN 共存与恢复失败验收。有线漫游作为当前验收环境的明确限制保留，不计为已验证能力。
