# Windows 连接事件与规则映射验证

验证日期：2026-09-21。平台：Windows 11 Pro 10.0.26200 x64；内核：`1.14.1-socks-proxy.2`。

运行配置现在使用 debug 级别输出，并在编译时保存每个 sing-box route index 对应的稳定业务规则 ID。基础例外、解析动作和 final 路径的映射为空。stderr 读线程按连接 ID 将原目标、实际 outbound、真实 `match[index]`、完成事件和错误事件聚合为连接事件；事件绑定首次出现时的配置修订及索引映射，配置切换后不会用新规则猜测旧连接。

`windows_ui_controller_smoke.exe` 在 Windows 上运行真实 SOCKS5 上游、TUN 运行时和 curl 连接，覆盖三种日志路径：

1. 规则代理成功：存在同连接 ID 的 `match[index]`，事件映射到创建规则的稳定 ID。
2. 全局代理成功：实际 outbound 为代理，但没有用户规则 match，规则保持“未知”。
3. 上游停止后的失败：实际 outbound 为代理，结果包含内核报告的连接失败。

实际结果：

```json
{"rules_confirmation":true,"rules_path_a":true,"rule_change_confirmation":true,"proxy_confirmation":true,"path_b":true,"global_confirmation":true,"global_path_b":true,"rule_log":true,"unknown_log":true,"failure_log":true,"clear_kept_proxy":true,"failure_visible":true,"failure_kept_direct":true}
```

单元测试直接重放 `windows11-tun.log` 和 `windows11-http-auth-failure.log` 的实际行形状，验证 ANSI 清理、成功/失败聚合、未知规则和跨修订映射。连接日志页读取共享控制器的事件快照，不维护演示数据。
