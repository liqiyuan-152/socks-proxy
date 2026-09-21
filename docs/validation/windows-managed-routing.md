# Windows 受管理 DNS 与路由验证

日期：2026-09-20。平台：Windows 11 Pro 10.0.26200 x64。内核：`1.14.1-socks-proxy.2`，SHA-256 `40a64f2973858203468da544db40c6d546f14fd8f02ffa38954e16bb776927a1`。

## 实现结论

Rust 配置编译器按以下顺序生成路由：基础防回环例外、域名规则、仅在存在 IP 类规则时执行的固定直连 DNS 解析、真实 IP/CIDR/范围规则、最终直连。域名规则已经命中时不强制直连解析。全局模式先固定直连解析，再用补丁字段 `ip_all_private` 仅豁免全部候选均为私网的连接，最后代理。

上游 sing-box 的 `ip_is_private` 对候选集合采用“任一私网即命中”，无法满足混合私网/公网仍代理的规格。项目补丁新增 `ip_all_private`，空候选不命中，全部私网才命中；纯公网和混合候选均不命中。Go 单元测试、本机 Rust 结构测试及 Windows 实际连接共同验证此语义。

## Windows 矩阵

`windows-managed-routing-smoke.ps1` 使用回环 SOCKS5、UDP DNS、HTTP 和 SSH banner 夹具，检查实际出站日志。15/15 通过：

- 浏览器式 HTTP 与 SSH 域名规则优先走代理。
- IPv4 CIDR、单 IPv4、闭区间转换、IPv6 CIDR、多地址任一命中及远程私网规则走代理。
- 代理不可达时未命中域名仍经独立 DNS 直连；已命中域名失败且不回退直连。
- 代理端目标域名解析失败时连接失败且不回退；直连 DNS 不可达时需要解析的连接明确失败。
- 全局模式下全部私网候选直连，纯公网及混合私网/公网候选走代理。

`windows-fakeip-routing-smoke.ps1` 仅向 `198.18.0.0/15` 安装临时 TUN 路由。4/4 通过：浏览器域名、SSH 域名、多地址任一 IP 命中和远程私网 IP 命中均先由受管理 DNS 获取 FakeIP，再由 TUN 反向恢复域名、取得真实候选并走代理。测试后 TUN 接口和路由均删除。

`.2` 还重跑严格缓存完整回归：4 个顶层 Go 测试、10 项基础代理/TUN、5 项 SSH/UDP/HTTP、7 项 DNS 路由、3 项 FakeIP 与损坏缓存拒绝均通过。最终清理结果为内核进程、测试 TCP/UDP 监听、测试适配器和测试路由各 0。

结果文件：

- `evidence/windows11-managed-routing-results.json`
- `evidence/windows11-fakeip-routing-results.json`
- `evidence/windows11-core-v2-hash.json`
- `evidence/windows11-core-v2-cache-tests.txt`

## 边界

测试 DNS 上游和目标均为受控夹具，不使用真实代理秘密。应用自带 DoH/DoT 若绕过系统 DNS 且无法提供域名映射，只能按可见 IP/端口决策；这属于规格中已声明的 MVP 边界。Windows 10、实际浏览器私有 DNS 设置组合、系统 DNS 缓存刷新失败恢复及应用私有缓存提示仍由后续验收任务覆盖。
