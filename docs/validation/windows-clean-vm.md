# Windows 干净虚拟机验证尝试

日期：2026-09-21。宿主：Windows 11 Pro 10.0.26200 x64，Hyper-V 已启用。

通过 Fido 查询并下载了微软官方 Windows 10 22H2 x64 与 Windows 11 25H2 x64 ISO。Windows 10 Gen2 VM 使用 72 GiB 动态 VHDX、4 GiB 内存、Secure Boot、vTPM 和断开的虚拟网卡。Windows Setup 从官方 ISO 启动，读取 seed ISO 中的 `Autounattend.xml`，自动选择 Windows 10 Pro、分区并应用系统镜像。

Panther 记录显示 windowsPE 与 offlineServicing 成功，`setuperr.log` 为空，Windows 10 Pro 镜像完整写入 VHDX。后续 `oobeSystem` 未执行，本地验证账户未创建，PowerShell Direct 因无有效凭据无法进入来宾。宿主仅能通过 SSH 管理，当前没有可用的交互式 Hyper-V 控制台来完成 OOBE。为避免把供应问题误记为产品通过，未在该 VM 执行安装包，失败 VM 已删除。

机器可读结果：`docs/validation/evidence/windows-clean-vm-attempt.json`。官方 ISO 保留在 Windows 宿主 `G:\socks-proxy-clean-vm`，可在获得交互式 VM 控制台后继续。任务 7.1 与 7.3 保持未完成。
