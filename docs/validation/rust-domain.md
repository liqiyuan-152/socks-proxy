# Rust 领域模型验证

2026-09-20，macOS ARM64，Rust/Cargo 1.95.0。

完成 tasks 2.3、2.4：

- 五种规则目标使用类型区分；域名按严格 IDNA、大小写和尾点规范化，后缀匹配遵守标签边界。
- 目标与端口 AND；禁用规则不匹配。匹配 API 仅接受真实 IP，不承担 FakeIP 还原和 DNS 解析。
- 端口空输入为 1–65535；混合表达式合并重叠和相邻区间；拒绝空项、反向范围、零和越界。
- IPv4/IPv6 含端点范围生成等价且互不重叠的 CIDR；处理完整 IPv6 空间及最大地址，不逐地址展开，块数量上界 2 × 地址位宽。

验证结果：cargo check --locked、cargo test --locked（6 个集成测试）、cargo fmt --check、cargo clippy --locked --all-targets -- -D warnings 全通过。小范围穷举测试检查 2080 个闭区间，每个区间检查 65 个候选地址的覆盖次数；另测 IPv6 范围、两族完整空间、单点、非法输入和国际域名。OpenSpec 严格校验通过。

Cargo 工程和模块边界已建立，Cargo.lock 锁定当前领域模型依赖；其他模块目前为空，尚无 GUI 或网络控制器。2.1 保持未完成，不能把本机库测试当 Windows 应用构建通过，也未锁定尚未引入的桌面依赖。

本机用户级 Cargo 配置中的 USTC Git 索引返回 404，本轮未修改全局配置。复现本次隔离官方源验证：

```sh
cd /tmp
export CARGO_HOME=/tmp/socks-proxy-cargo
cargo test --locked --manifest-path /Users/liqiyuan/WebstormProjects/socks-proxy/Cargo.toml
cargo check --locked --manifest-path /Users/liqiyuan/WebstormProjects/socks-proxy/Cargo.toml
cargo clippy --locked --all-targets --manifest-path /Users/liqiyuan/WebstormProjects/socks-proxy/Cargo.toml -- -D warnings
cargo fmt --manifest-path /Users/liqiyuan/WebstormProjects/socks-proxy/Cargo.toml --check
```

常规 Cargo 源可用的环境可直接运行 scripts/validation/rust-check.sh。

后续实现增加了代理 CRUD、三模式路由策略和版本化配置存储：

- 代理使用 UUID 稳定 ID，仅保存不透明凭据引用；单选失败保留旧选择，代理运行时禁止删除当前代理，已恢复直连后可显式删除。
- 基础控制端点及上游端点例外先于用户规则；规则内目标与端口为 AND，规则间为 OR；规则模式允许远程内网代理。
- 全局模式仅在真实候选地址全部为内网时直连，混合候选代理；尚需真实地址才能判定时返回 `NeedsDirectDns`，不把解析失败解释为直连。
- 配置 schema 版本为 1，首次启动为 Direct 且不开机启动；保存使用同目录临时文件、落盘同步及原子替换，替换前保存最近有效备份。主配置损坏时读取备份。
- 故障注入在备份成功、主文件替换失败的位置中止，验证旧主配置及备份均可读取。
- 无密码备份仅包含代理非秘密字段、认证启用标记、规则和偏好；不含凭据引用及运行模式。导入先完整预览，认证代理标记为待补凭据，提交强制 Direct 并重新核对网络恢复、切换状态和配置修订；损坏/未知版本/悬空引用/写入失败均不替换旧配置。
- 凭据编辑先创建不可变的新引用，再原子记录旧/新配置和事务阶段，随后应用内核并提交配置。故障注入覆盖凭据创建、内核应用、配置保存和崩溃恢复；内核或保存失败恢复旧修订。恢复根据磁盘提交修订选择旧/新配置，记录不含明文秘密；凭据回收集合同时包含当前配置、恢复记录和保留备份的引用。

2026-09-20 新增验证结果：本机 32 个测试通过，`cargo check --locked`、`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings` 通过；安装 `x86_64-pc-windows-msvc` 标准库后，`cargo check --locked --target x86_64-pc-windows-msvc` 通过。Windows 分支使用 `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`。

M0 网络门槛仍未完成；本轮纯模型和存储工作没有接管或修改 Windows 网络。浏览器/SSH 完整路径、跨账户凭据、Windows 10 与交付验收均未据此宣称通过。
