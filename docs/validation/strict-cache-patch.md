# 严格缓存补丁交付与实测

2026-09-20 用户批准的缓存补丁已实现并在 Windows 11 x64 验证。

## 交付

- third_party/sing-box/strict-cache.patch：配置开关、只读预检、禁止重建、运行期 fail-stop 与测试。
- third_party/sing-box/source-lock.json：上游 commit、源包校验和、Go 版本及构建标签。
- third_party/sing-box/build.py：从固定源包干净应用补丁、测试、构建 Windows 二进制及对应源码包。
- dist/sing-box：1.14.1-socks-proxy.2 二进制、Windows 测试程序、对应源码包、构建材料及 SHA-256 清单（生成物，不纳入源码版本控制）。

Windows 二进制 SHA-256：40a64f2973858203468da544db40c6d546f14fd8f02ffa38954e16bb776927a1

本机 Go 1.26.7 与 Windows 原生测试程序均通过 4 个顶层测试（运行期含 view/batch/update 三个子测试）：损坏/空文件不改写、首次创建及健康重启、默认模式兼容、运行期损坏标记与退出 78。运行期采用故障注入，不代表所有物理磁盘损坏场景。

Windows 补丁二进制重跑 10 项代理/TUN、5 项 SSH banner/UDP、7 项 DNS 路由、3 项 FakeIP 检查，25 项均通过。损坏缓存另测通过：启动退出 1、包含 STRICT_CACHE_ERROR、原文件 SHA-256 不变。Windows 进程/监听/测试路由清理通过。完整证据见 evidence/strict-cache。

项目版本 `.2` 在严格缓存补丁之上增加 `ip_all_private` 规则字段，用于全局模式“仅当全部真实候选均为私网时直连”。同一构建重跑上述完整回归均通过；补丁单元测试和 Windows 二进制校验记录见 `evidence/windows11-core-v2-cache-tests.txt` 与 `evidence/windows11-core-v2-hash.json`。

修正测试脚本保留进程句柄，避免 Windows PowerShell 对快速退出进程返回 null ExitCode；最终证据要求退出码存在且非零，结果为 1。原官方二进制失败证据保留在上级 evidence，未覆盖。

## 边界

仅解决已检测到的缓存损坏自动重建。未初始化文件允许创建，历史初始化标记、缺失缓存检测、显式重建 UI、网络恢复监管仍由尚未实现的 Rust 控制器承担。完整 M0、Windows 10、桌面/UAC、跨账户与许可证依赖清单尚未全部完成。补丁版仅编入本项目所需 gvisor/clash-api 标签，不宣称与官方全特性包等价。
