## Why

用户已有 SOCKS5 和 HTTP 代理，需要一个 Windows 桌面工具集中管理代理地址，并让浏览器、SSH 及不同端口的连接按目标规则分流。当前项目为空，先建立可验收的首版契约，避免沿用早期已被否决的 Web 界面和多代理并行方案。

## What Changes

- 使用 Rust + egui/eframe 构建非 Web 桌面界面，以 sing-box CLI + TUN 负责网络转发；基于锁定版本维护已获用户批准的严格缓存模式补丁，禁止损坏缓存自动重建。
- 保存多个 SOCKS5 / HTTP 代理及可选认证，一次仅选一个代理。
- 提供规则代理、全局代理、全局直连；规则代理命中才代理，其余直连。
- 支持精确域名、域名及子域名、单个 IP、CIDR、IP 范围，附加单端口、多端口及端口范围。
- 提供一个托盘图标、模式和代理子菜单，以及首页、代理、规则、日志、设置五个页面。
- 每次启动申请管理员权限；关闭窗口留在托盘，退出停止内核并恢复本程序修改的网络设置。
- 首次直连，后续恢复上次成功模式；凭据受保护存储、默认无密码导出、本地可清理日志。
- 将 DNS、内核正常运行时上游故障不回退直连、网络恢复和 Windows 实测设为交付门槛。系统级崩溃阻断保护移至后续独立增强提案，不阻塞 MVP。

## Capabilities

### New Capabilities

- `proxy-profiles`: 代理配置、单选、认证与输入校验。
- `traffic-routing`: 三种模式、五种规则、端口条件与 DNS 分流。
- `core-lifecycle`: 提权、内核管理、配置切换、异常与网络恢复。
- `desktop-shell`: 非 Web 主窗口、托盘操作与状态同步。
- `configuration-storage`: 配置持久化、凭据保护、导入导出。
- `connection-logs`: 本地连接结果、命中规则、脱敏与保留限制。

### Modified Capabilities

无。当前没有既有代码或主规格。

## Impact

新增 Windows 桌面程序、配置模型、sing-box 适配层及安装交付流程；依赖 Windows 提权、凭据保护、路由/DNS/TUN 能力。当前 macOS 工作目录只能进行文档及后续跨平台逻辑验证，不能代替 Windows 10/11 的验收。

会话依据与决策变更见 `requirements-source.md`。本变更仅创建规划；规范保留在 change 中，实现并验收后再归档到主 specs。
