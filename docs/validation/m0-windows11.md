# M0 Windows 11 实测报告

日期：2026-09-20。平台：Windows 11 Pro 10.0.26200 x64，管理员 SSH。最初以 sing-box 1.14.1 验证，当前项目锁定补丁版本 `1.14.1-socks-proxy.2`；M0 仍有独立恢复场景未完成。

## 已通过检查

| 分类 | 实际结果 | 证据 |
|---|---|---|
| 基础代理及 TUN | 10/10；SOCKS5/HTTP 无认证及正确认证成功，错误认证失败；局部 TUN TCP 转发成功，强制终止后测试路由移除，直连基线恢复 | evidence/smoke-results.json |
| FakeIP | 2/2；不同域名分配不同地址，强制终止并反序查询后仍恢复原映射 | evidence/dns-results.json |
| Windows 凭据 | 4/4；当前用户 DPAPI 加解密、非明文密文、受限 ACL、损坏密文拒绝 | evidence/credential-results.json |
| 清理 | 测试内核、路由、TCP/DNS 监听均为 0 | evidence/cleanup-results.json |

初次 core smoke 的 PowerShell Job 停止等待需要手工终止该测试监听进程；脚本随后改为可取消的 Pending 轮询并完整重跑，10 项仍全部通过且自动清理成功。结果来自修正后的运行。

## 配置接口验证结论

- `tun.address`、`auto_route`、`route_address`、`dns_mode=disabled` 和 `stack=mixed` 已实际启动，不依赖全局 TUN 接管。
- SOCKS5/HTTP 出站认证字段和 HTTP CONNECT 转发已通过受控回环 HTTP 目标验证；这不等价于 SSH/UDP 验收。
- FakeIP 不能作为默认 DNS 服务器，须用 A/AAAA 查询规则显式选择；还需配置 `route.default_domain_resolver`。本实验真实 DNS 占位上游不可达且未用于任何测试查询，不代表真实地址解析已验证。
- `experimental.cache_file.store_fakeip` 在本次强制停止后恢复测试通过；缓存损坏/兼容升级、未知地址拒绝仍未验证。

## 连接日志能力（任务 1.7）

见 evidence/windows11-tun.log 和 evidence/windows11-http-auth-failure.log。debug 事件包含连接 ID、inbound 原目标、`router: match[0]` 的规则索引、实际 outbound 标签；HTTP 错误认证包含明确 authentication required。规则命中不是稳定业务 ID，适配层必须使用本次内核配置修订的索引映射，不使用当前编辑草稿推测历史命中。

pre-match 和真实连接可能使用不同连接 ID，不能仅按时间邻近拼接；只使用同连接 ID 的实际 match 事件。没有实际 match 事件（例如 final 出站）的规则字段标为未知，原目标与 override 后实际拨号目标分开存储。配置版本变化后的旧连接保留对应修订映射。成功样本具有连接下载完成记录；不能把仅创建 outbound 的事件当作成功。

目前可以确认该版本具备所需字段来源，同时存在上述明确缺失边界；日志适配与脱敏代码尚未实现。

## 许可初查

官方 v1.14.1 LICENSE 声明 GPL-3.0-or-later，另含名称/关联声明限制。项目实际构建不包含官方包的 `libcronet.dll`，只使用 `with_gvisor,with_clash_api`。实际 Windows 依赖图、TUN/Wintun 校验和与 67 项许可证清单已由 `third_party/sing-box/license_inventory.py` 生成，结果和发行条件见 `docs/validation/dependency-licenses.md`；任务 1.8 已完成。

来源：
- https://github.com/SagerNet/sing-box/releases/tag/v1.14.1
- https://github.com/SagerNet/sing-box/blob/v1.14.1/LICENSE
- https://github.com/SagerNet/sing-box/blob/v1.14.1/docs/configuration/inbound/tun.md
- https://github.com/SagerNet/sing-box/blob/v1.14.1/docs/configuration/experimental/cache-file.md

## 复现

脚本位于 scripts/validation；windows-ssh.py 将 PowerShell 以 UTF-16LE EncodedCommand 传递，避免远程 shell 展开变量，不存储密码。使用既有 SSH 密钥或已认证的控制连接运行，host/socket 均由调用者显式指定。

```sh
python3 scripts/validation/windows-ssh.py WINDOWS_SSH_ALIAS scripts/validation/windows-core-smoke.ps1
python3 scripts/validation/windows-ssh.py WINDOWS_SSH_ALIAS scripts/validation/windows-dns-smoke.ps1
python3 scripts/validation/windows-ssh.py WINDOWS_SSH_ALIAS scripts/validation/windows-credentials-smoke.ps1
```

前置：已将校验过的官方 zip 解压到用户目录 socks-proxy-validation，管理员权限且测试端口空闲。脚本拒绝已存在的测试接口，测试不使用生产代理账户。网络实验应保有控制台恢复入口。

## 当时尚待完成

以下是第一轮 smoke 完成时的状态，不代表当前任务清单。任务 1.1、1.2、1.3、1.4、1.5、1.6、1.8、1.9、1.10 当时均只有部分或尚无证据。后续验证分别记录在本目录的专项文档中；任务 1.5 的完整内核及宿主异常恢复证据见 `windows-runtime-failure.md`。

## 第二轮实测更新

新增 5 项协议、7 项 DNS 路由以及 1 项未知 FakeIP 检查均通过，累计 29 项通过；另有缓存损坏检查失败。当前任务 1.4、1.7 完成（2/38），此前未完成清单中的 1.4 已被此更新替代。

- protocol-results.json：SOCKS5/HTTP 均取得本机 OpenSSH banner（未测试用户登录及长会话）；SOCKS5 UDP 回显成功；HTTP UDP 无回显且日志明确显示 `UDP is not supported by outbound: proxy`；CONNECT 22 被拒时服务端日志确认 reject，未回退直连。
- routing-results.json：独立 UDP DNS 固定记录验证 IPv4 CIDR、多地址任一命中及 IPv6 CIDR；代理不可达时未命中域名可直连，命中连接失败；DNS 上游不可达时明确失败。所有成功出站有对应日志证据。
- dns-results.json：追加未知 FakeIP 拒绝检查通过。
- cleanup-results.json：测试进程、TCP/UDP 监听及测试路由均为 0。

限制：DNS 实验使用 mixed 入站，不代表浏览器/SSH 经过系统 DNS 与 TUN 的全链路已验证。域名命中实验覆盖到固定目标，不代表远程域名解析已验证。第三方代理服务的端口及 UDP 能力仍由服务端决定。

接口修正：1.14.1 新 UDP DNS 使用自身 direct dialer；显式 detour 到无参数 direct 出站会启动失败。测试配置已移除该字段，保持独立直连解析语义。

### 实测阻塞：损坏缓存被自动重建

windows-cache-corruption.ps1 使用独立损坏缓存启动内核，结果内核仍正常启动，且没有记录缓存错误。健康的原缓存未改动。证据见 evidence/cache-corruption-results.json 和 windows11-cache-corrupt.log。

官方源码 https://github.com/SagerNet/sing-box/blob/v1.14.1/experimental/cachefile/cache.go 的 start() 在 ErrInvalid / ErrChecksum / ErrVersionMismatch 时删除数据库后重试；完整性检查失败及运行期错误恢复也存在自动重建路径。仅判断进程就绪或扫描日志无法保证“映射损坏进入 Error、显式重建前不复用旧地址”。

### 待决定的修正方案

推荐保持现有规范，为分发的 sing-box 增加最小严格缓存模式补丁：

1. 严格模式下禁止启动及运行期自动重置损坏的 FakeIP 缓存，仍允许正常首次创建。
2. 损坏时停止接管、明确报错并保留原缓存，由 Rust 控制器进入 Error。
3. 仅在用户显式重建后生成新缓存，补充旧映射防误用测试。
4. 固定官方版本和补丁，补充 Go 构建、升级回归及对应源码分发记录。

这新增第三方内核补丁维护工作，超出当前直接管理官方 CLI 的实现方案，需要用户决定后才修改设计/任务并构建内核。替代方案是放宽缓存损坏保证，但会改变已约定行为，不自动采用。

当前未改内核或放宽规格，所有测试资源已经清理。

## 已批准补丁的验证结果

上述自动重建阻塞已通过严格缓存补丁解决。用户批准后完成本机和 Windows 测试、25 项补丁二进制回归及损坏缓存拒绝启动实测，详见 strict-cache-patch.md。新增任务 1.11/1.12 已完成，当前 4/40；其余 M0 门槛及应用控制器仍未完成。

## `.2` 受管理路由更新

项目为“全局模式仅全部真实候选均为私网时直连”增加 `ip_all_private` 最小补丁，并在 Windows 重跑完整严格缓存回归。另完成 15 项受管理路由与 4 项 FakeIP/TUN 全链路检查，覆盖浏览器式 HTTP、SSH 域名、IPv4/IPv6、单 IP、CIDR、范围、多地址任一命中、远程私网、混合内外网候选、直连 DNS 故障和代理端解析失败。详情见 `windows-managed-routing.md`。
