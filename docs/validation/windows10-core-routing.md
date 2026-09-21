# Windows 10 内核路由与协议验证

验证日期：2026-09-21。平台：Windows 10 Pro 10.0.19045 x64 Hyper-V VM。内核：`1.14.1-socks-proxy.2`，SHA-256 `40a64f2973858203468da544db40c6d546f14fd8f02ffa389544e16bb776927a1`。

`windows-routing-smoke.ps1` 使用本机受控 DNS、HTTP 和代理夹具验证路由判定。七项均通过：代理不可达时未命中域名继续使用独立直连 DNS 和直连出站；真实 IPv4 CIDR、多地址任一候选和 IPv6 CIDR 命中代理；已命中域名或 CIDR 时代理故障保持失败；直连 DNS 不可用时连接失败且不改变分流结果。

`windows-dns-smoke.ps1` 确认两个域名获得不同 FakeIP，强制停止并重启后映射保持不变，未知 FakeIP 被拒绝。`windows-cache-corruption.ps1` 确认损坏的严格缓存使内核退出且原文件保持不变。

`windows-core-smoke.ps1` 的 10 项全部通过：SOCKS5/HTTP 无认证和正确认证成功，错误密码失败；TUN TCP 转发、路由清理和停止后的直连恢复成功。Win10 强制停止会把测试接口保留为 Unknown PnP 设备，验证后按测试专用连接名删除，最终残留为 0。

`windows-managed-routing-smoke.ps1` 的 15 项最终全部通过，包括浏览器与 SSH 域名优先、单 IP/范围/IPv4/IPv6 规则、远程私网规则、未命中直连、代理故障无回退、DNS 故障，以及全局模式的全私网直连、公网代理和公私网混合候选代理。Win10 的 PowerShell 后台 Job 冷启动较慢，脚本加入 2 秒夹具就绪等待后消除了前三项的启动竞态。

不可见域名边界由同一组真实路径共同覆盖：`browser-single-ip-rule` 在只提供目标 IP/端口时按 IP 规则代理，`unmatched-direct-with-dead-proxy` 在没有域名和 IP 命中时即使代理不可达仍直连成功；Win10 原生路由测试对 `domain: None` 执行相同决策。连接事件在缺少内核域名或规则标识时记录未知，不反推或伪造域名命中；应用自有 DoH/缓存导致域名不可恢复的限制已在用户指南明确说明。

协议验证先暴露环境缺口：VM 的 `127.0.0.1:22` 没有 SSH 服务。加入只发送 `SSH-2.0-OpenSpecFixture` 的受控 TCP banner 夹具和相同就绪等待后，最终一轮 5 项全部通过：SOCKS5/HTTP 均传递 SSH banner，SOCKS5 UDP 回显成功，HTTP UDP 明确失败，HTTP CONNECT 22 拒绝没有直连回退。

机器可读结果见 `evidence/windows10-core-routing-results.json`。所有目标、DNS 和代理均为 VM 本机临时夹具，不包含真实秘密。
