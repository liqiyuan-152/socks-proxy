# socks-proxy

Windows 10/11 x64 桌面代理管理器，使用 Rust + egui/eframe 与 sing-box TUN，支持 SOCKS5/HTTP 配置、按目标分流、连接日志和无秘密配置备份。OpenSpec `windows-proxy-mvp` 的 40 项实施与验收任务已经完成并归档。

## 文档入口

- [首版提案](openspec/changes/archive/2026-09-21-windows-proxy-mvp/proposal.md)
- [需求来源与决策记录](openspec/changes/archive/2026-09-21-windows-proxy-mvp/requirements-source.md)
- [技术设计](openspec/changes/archive/2026-09-21-windows-proxy-mvp/design.md)
- [实施任务与验收步骤](openspec/changes/archive/2026-09-21-windows-proxy-mvp/tasks.md)
- [能力规范目录](openspec/specs/)
- [使用与离线恢复说明](docs/user-guide.md)
- [Windows 验证记录](docs/validation/)

## OpenSpec

使用本机 OpenSpec 1.12.0 初始化，已生成 Codex 项目技能，文档默认中文。

```sh
openspec list
openspec validate --all --strict
python3 scripts/validation/build-spec-matrix.py
```

归档记录位于 `openspec/changes/archive/2026-09-21-windows-proxy-mvp/`，当前能力规范位于 `openspec/specs/`。Windows 10 和 Windows 11 的 67 项场景矩阵见 `docs/validation/windows-spec-matrix.md`。
