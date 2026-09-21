# Windows 托盘生命周期验证

日期：2026-09-21。平台：Windows 11 Pro 10.0.26200 x64，管理员 SSH。验证对象为 release 模式 Windows GUI 程序。

`scripts/validation/windows-tray-lifecycle-smoke.ps1` 设置 `SOCKS_PROXY_TRAY_REQUIRED=1` 后启动程序，因此托盘创建失败会直接使进程失败。主进程保持运行，第二个应用实例在 5 秒内退出，确认单一托盘进程和命名互斥约束生效。

```json
{"tray_created":true,"single_instance":true,"window_close_observed":false,"window_close_keeps_process":false}
```

最终版本于 2026-09-21 在 Windows 10 Pro 10.0.19045 的交互 Session 1 重新验证。应用 SHA-256 为 `7587f23e4263421a81aac9aa6ff65774495c02e999a29ef270aac57cfd2f1f3d`。

验证通过 `Shell_NotifyIconGetRect`、Windows UI Automation 和来宾会话内的 Win32 输入执行，覆盖：左键隐藏/恢复、关闭窗口后进程与托盘保留、再次恢复窗口、右键顶层菜单、三种模式子菜单、全局直连选中状态和绿色图标。五个页面入口均显示窗口，并产生五个不同的窗口截图哈希。

选择“退出”后应用进程结束，sing-box 进程与 `socks-proxy` TUN 均为 0，路由和 DNS 与退出前相同。最终结果见 `evidence/windows10-final-tray-results.json`，页面/退出原始采集见 `evidence/win10-tray-pages-exit.json`。

同日使用包含关停 DNS 刷新实现的 release GUI（SHA-256 `bb34d31f9388610a7dfd7e221de4bb34c190024ac4c521825717c73cdb8e4dad`）补跑代理运行状态退出。程序以 Rules 和 `Exit fixture` 运行，托盘顶行显示 `Socks Proxy - 规则代理 - Exit fixture`，五个页面入口均可到达；点击“退出”后应用与 sing-box 进程、本程序 TUN/PnP 设备和所有权路由均为 0，配置中的 `last_applied_mode` 仍为 `rules`。证据见 `evidence/windows10-shutdown-dns-results.json`。

该次自动化最初沿用 Direct 绿色图标的像素定位，在 Rules 蓝色图标下未找到图标；将临时验证脚本改为匹配生产代码定义的蓝色代理图标后复跑通过。首次失败没有修改产品或环境状态，复跑后完成了零残留审计。

最终状态同步复跑使用应用 SHA-256 `78274ed9465f6ec0c757b40722728e8bdffe8bca97144538d6c37c8c53c538a9`。从真实托盘依次切换 Error → Direct → Rules，再强制终止 sing-box 进入 Error；主窗口、托盘菜单顶行和通知区 tooltip 分别一致显示“全局直连”“规则代理 - Exit fixture”“运行异常 - Exit fixture”。Direct 时内核为 0，Rules 时为 1；Error 图标检测到 119 个生产红色像素。最后从 Error 状态点击托盘“退出”，应用、内核、TUN/PnP 和所有权路由均为 0，`last_applied_mode` 仍为 Rules。证据见 `evidence/windows10-tray-state-results.json`。

复跑发现 `tray-icon 0.25.1` 在使用固定 GUID 时，Windows 运行期 tooltip 更新没有把 GUID 带入 `NIM_MODIFY`，Explorer 返回 `E_FAIL` 并保留旧 tooltip。项目在 `third_party/tray-icon-0.25.1` 固定该版本并补入同一 GUID；应用把 Explorer 对图标和 tooltip 的运行期返回作为装饰性 best-effort 结果，菜单状态文本仍持续同步，避免 Shell 装饰更新失败污染业务状态。

Windows 11 最终状态同步复跑使用应用 SHA-256 `70a214f1349a26f72f5db94d0f655f633b39ce005184932a506e0ddbcdda6ad7`。验证在交互 console Session 12 中通过 Windows 11 的 `SystemTray.NormalButton` 隐藏图标入口执行；从真实托盘依次切换 Direct 和 Rules，确认 Direct 时内核为 0、Rules 时内核为 1。强制终止内核后，菜单状态变为“运行异常 - Windows 11 fixture”，Error 图标检测到 119 个生产红色像素。最后从真实托盘点击“退出”，应用、内核、TUN/PnP 和所有权路由均为 0，`last_applied_mode` 仍为 Rules。证据见 `evidence/windows11-tray-state-results.json`。

Windows 11 Explorer 在 `NIM_MODIFY` 后的 UI Automation 可访问名称中保留旧 tooltip 并附加新 tooltip，因此 Rules 和 Error 证据记录完整可访问名称；最新状态均存在，且托盘菜单顶行精确显示当前状态。Direct 冷启动后的 tooltip 为精确值。确认切换对话框同时补充 Enter 确认和 Escape 取消，避免键盘操作依赖高 DPI 下的像素坐标。

同一 Windows 11 二进制另行完成菜单清单及关闭窗口验证。顶层菜单包含状态、代理模式、切换代理、五个页面入口和退出；模式子菜单恰好为规则代理、全局代理和全局直连，代理子菜单只有当前 `Windows 11 fixture`。Rules 运行时关闭窗口后应用和内核进程均保持 1，窗口隐藏；左键托盘后窗口恢复。证据见 `evidence/windows11-menu-close-results.json`。
