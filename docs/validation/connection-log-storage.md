# 连接日志脱敏、保留与清空验证

验证日期：2026-09-21。连接事件写入应用本地 runtime 目录的 `connection-events.jsonl`，不发送到远程服务。

内核 stderr 读线程在任何显示、错误缓冲或事件适配前调用统一脱敏器。脱敏覆盖大小写不敏感的 `password`、`passwd`、`username`、`Authorization`、`Proxy-Authorization` 字段，以及 URL userinfo。失败事件保存前再次脱敏；已有 JSONL 在加载时也重新脱敏，避免旧内容直接进入界面。

日志存储每次加载和写入均执行两项限制：移除超过 7 天的事件，并从最旧事件开始清理，直至 JSONL 不超过 20 MiB。测试使用较小容量注入超限记录，验证保留最新事件、删除过期事件且文件不超过限制；常量断言同时锁定生产阈值为 20 MiB 和 7 天。

敏感文本测试注入了表单、引号字段、HTTP 认证头和带 userinfo 的 URL。内存事件与磁盘 JSONL 均不包含测试秘密，重新加载仍为脱敏内容。

Windows 真实运行时在全局代理状态产生成功和失败事件后执行“清空”。结果 `clear_kept_proxy=true`：共享事件快照清空、本地 JSONL 删除，实际已应用模式仍为全局代理；之后可正常切回直连。完整结果见 `windows-connection-logs.md`。
