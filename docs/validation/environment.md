# M0 环境与受控端点记录

检查日期：2026-09-20，最终更新：2026-09-21。变更：windows-proxy-mvp。

## 已观测环境

| 项目 | 结果 |
|---|---|
| 本机系统与架构 | Darwin arm64（macOS，非 Windows x64） |
| Rust | rustc 1.95.0 (59807616e 2026-04-14) |
| Cargo | cargo 1.95.0 (f2d3ce0bd 2026-03-21) |
| 已安装编译目标 | aarch64-apple-darwin、wasm32-unknown-unknown |
| sing-box | PATH 中未发现 |
| Windows/虚拟机 CLI | PATH 中未发现 prlctl、VBoxManage、virsh、qemu-system-x86_64、pwsh |
| 应用目录 | 未发现常见 Parallels、VMware Fusion 或 UTM；存在 Remote Desktop，但未连接或探测任何远程主机 |
| 工程 | 现有 OpenSpec 规划；尚无 Cargo 工程 |

以上只能说明本次检查未找到可直接使用的 Windows 环境，不能证明用户没有远程 Windows 机器。

## 环境与端点结论

任务 1.1 的首个验证环境已建立在 Windows 11 Pro 10.0.26200 x64。Windows 10 的发行验收仍属于 7.1，不以当前 Windows 11 结果替代。受控端点均在验证主机回环地址临时启动，测试后清理：

| 端点 | 实现与用途 | 认证数据 |
|---|---|---|
| SOCKS5 | 锁定 sing-box mixed fixture；验证无认证、合成认证、错误认证、TCP/UDP 和上游故障 | 仅脚本内合成 fixture 值 |
| HTTP CONNECT | 锁定 sing-box mixed/http fixture；验证无认证、合成认证、错误认证、CONNECT 22 拒绝和不支持 UDP | 仅脚本内合成 fixture 值 |
| SSH | `127.0.0.1:22` 的 Windows OpenSSH banner及 `127.0.0.1:18122` 合成 SSH banner fixture；不执行用户登录 | 无登录秘密 |
| HTTP 目标 | PowerShell/TCP 回环监听，返回固定短正文用于核对实际路径 | 无认证 |
| DNS | 回环 UDP hosts fixture，返回固定 A/AAAA 候选用于确定性路由 | 无认证 |

`windows-core-smoke.ps1`、`windows-protocol-smoke.ps1`、`windows-managed-routing-smoke.ps1`、`windows-fakeip-routing-smoke.ps1` 和 Rust Windows smoke 均会检查端口占用、设置超时清理并保存结构化结果。真实 SSH 登录密码和用户提供的 SSH 认证秘密从未写入仓库或测试配置。

## 到达 Windows 后的采集与验证

1. 记录系统版本、构建号、原生架构和有效账户是否提升；Windows ARM 不作为 x64 验收环境。
2. 记录 Rust/MSVC 构建工具及受控 sing-box 版本、来源和校验和。
3. 建立受控 SOCKS5、HTTP CONNECT、SSH 端点，记录地址和端口及是否启用认证，不保存秘密。
4. 记录修改前网络环境；执行最小配置校验及内核启动，再验证停机恢复。
5. 按 tasks.md 的 M0 场景保留实际结果，分别记录通过、失败和未执行。只有完成指定验证才能勾选对应任务。

## 当前执行边界

当前环境已完成锁定内核、受限 TUN、DPAPI、DNS/FakeIP、代理协议、运行时切换及恢复实验。交叉编译用于应用构建，网络结论均来自 Windows 原生执行。桌面托盘、UAC 取消以及 Windows 10 的最终验收仍需交互式桌面或另一验证环境。

## Windows SSH 检查更新

2026-09-20 已成功通过 SSH 连接用户指定的 10.168.1.158，账户 lqy15。认证秘密未写入项目文件。

- 系统：Microsoft Windows 11 专业版，版本 10.0.26200。
- 架构：64-bit、x64-based PC，符合首个 Windows 验证平台要求。
- SSH 会话有效管理员权限：true。
- Git：已在 PATH 中发现。
- rustc、cargo、sing-box：未在当前 SSH 会话 PATH 中发现；不等价于全盘未安装。
- 本轮仅执行读取系统信息与工具发现命令，未修改 Windows 路由、DNS 或防火墙，未启动 TUN。

原“没有 Windows 接入环境”阻塞已解除。后续需准备工具链、内核及受控测试端点；Windows 10 验收仍待执行。桌面、托盘及 UAC 交互测试需桌面会话。TUN 测试前需确认 SSH 中断时的控制台/远程桌面恢复入口。

## 2026-09-20 实测进展

用户已确认可以通过本地或远程桌面恢复机器。已在独立用户测试目录运行 sing-box 1.14.1，完成局部 TUN、代理、DNS 与凭据实验。测试仅使用合成凭据，未写入 SSH 登录秘密。

- 下载包 SHA-256：5197f16d492d93202dc623622149a6ed040f8eca263128f91d603f2b901baa89，与 GitHub release asset digest 一致，Windows 端再次校验。
- 内核输出：go1.26.8 windows/amd64，revision 1ac1a339cb1223e9c70eae14c44411c75033c02d。
- 10 项代理/TUN 检查、2 项 FakeIP 检查、4 项凭据检查通过，另有测试资源清理检查通过。
- TUN 仅路由 198.19.0.0/24，dns_mode=disabled，未接管默认路由或修改系统 DNS。测试使用前检查端口和接口冲突，内核具有 60 秒独立清理期限。
- FakeIP DNS 使用回环 UDP 15353；强制停止后反序查询仍得到原映射。
- 清理验证：测试内核进程、测试路由、TCP/DNS 测试监听均为 0。Windows 测试目录保留日志、包与配置用于复现。
- Rust/MSVC 工具链尚未安装；尚未验证 Windows 10、图形界面、完整 DNS 接管、多地址规则、真实 SSH 转发、UDP、标准用户跨账户提升及完整网络故障恢复。

详细结果见 [M0 实测报告](m0-windows11.md)。此前“未启动 TUN”记录仅描述初次检查时点，以本次更新为准。

第二轮累计 29 项检查通过；损坏缓存检查失败，发现官方内核自动重建缓存与规格冲突。详见 m0-windows11.md 的实测阻塞与修正方案。
