# Windows 规则编辑验证

验证日期：2026-09-21。规则页面和编辑窗口已在 macOS eframe 原生窗口实际渲染复查；Windows 行为验证使用 Windows 11 Pro 10.0.26200 x64 和 `1.14.1-socks-proxy.2` 真内核。

规则草稿支持精确域名、域名及子域名、单个 IPv4/IPv6、CIDR 和包含端点的 IP 范围。空端口表示全部，混合端口由领域层同一 `PortSet` 解析器处理。名称、目标和端口错误分别映射到对应字段。新建、编辑、启停和删除均通过 `SharedController` 事务应用；规则模式运行中改变有效规则时先显示中断确认，确认前配置和运行状态不变。

自动测试覆盖五类目标、`22,443,8000-9000`、空端口、禁用和删除、逐字段错误及确认前不提交。`windows_ui_controller_smoke.exe` 则通过 `save_rule` 创建 `198.51.100.10:80` 规则，确认规则模式后发起真实连接，响应正文为上游 A fixture 的 `A`；随后请求禁用规则，控制器返回确认要求且未提前改变状态。

Windows 实际结果包含：

```json
{"rules_confirmation":true,"rules_path_a":true,"rule_change_confirmation":true,"proxy_confirmation":true,"path_b":true,"global_confirmation":true,"global_path_b":true,"failure_visible":true,"failure_kept_direct":true}
```

原生界面复查确认规则空状态、表头、新建按钮和编辑窗口无重叠或截断；编辑窗口包含名称、启用、目标类型、目标、端口、备注、保存和取消控件。
