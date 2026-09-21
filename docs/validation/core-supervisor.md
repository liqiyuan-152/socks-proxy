# 内核监管状态验证

日期：2026-09-20。

`src/core/mod.rs` 固定内核版本 `1.14.1-socks-proxy.2`，状态快照分别保存期望模式、实际应用模式、Direct/Starting/Running/Error、错误类型、可能直连提示和缓存初始化状态。

启动顺序为：检查历史初始化缓存是否仍存在、读取并严格比较内核版本、启动进程、等待就绪、确认持久缓存文件实际存在，随后才进入 Running 并把缓存标为已初始化。`AppConfig.cache_initialized` 由版本化配置原子持久化。历史标记为 true 但文件丢失时，在启动进程前进入 Error；首次启动时，即使进程声称就绪，缓存文件未创建也不报告 Running。

运行期退出码 78 或输出包含 `STRICT_CACHE_ERROR` 均映射为 StrictCache 错误。其他意外退出映射为 UnexpectedExit；实际应用模式清空，最后成功配置不改成 Direct，并设置“流量可能直连”提示。

故障注入覆盖：缺失二进制、版本不匹配、启动/就绪失败、历史缓存丢失、首次缓存未创建、健康缓存启动、严格退出码和严格错误标记。监管、事务和网络恢复测试通过；本机完整 59 个 Rust 测试、clippy `-D warnings` 和 Windows MSVC `cargo check --all-targets` 通过。

实际 Windows 子进程隐藏启动、临时配置 ACL 和 sing-box 配置编译分别属于任务 4.1/4.2，未由本记录宣称完成。
