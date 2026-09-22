# 内核与 TUN 分发核对

日期：2026-09-20。核对对象是项目实际构建的 Windows amd64 内核，不是 sing-box 官方全功能压缩包。

## 固定构建

- sing-box：`v1.14.1` / commit `1ac1a339cb1223e9c70eae14c44411c75033c02d`，项目版本 `1.14.1-socks-proxy.2`。
- 上游源码：`https://codeload.github.com/SagerNet/sing-box/tar.gz/1ac1a339cb1223e9c70eae14c44411c75033c02d`，SHA-256 `8420c7723828a8d9d062c3fafee28c7b7e0d20a4c904fa7ff283f6e884edd537`。
- Go：`go1.26.7`；目标 `windows/amd64`；`CGO_ENABLED=0`；标签 `with_gvisor,with_clash_api`。
- 补丁、源锁、完整对应 sing-box 源码包和构建参数由 `third_party/sing-box/build.py` 生成并写入 `dist/sing-box`。

## 实际链接依赖

`third_party/sing-box/license_inventory.py` 使用和构建相同的目标及标签运行 `go list -deps`。本次得到 66 个 Go 模块/标准库条目，加 1 个嵌入的 Wintun 预编译 DLL，共 67 项；所有项均找到根许可证，原文、版本和 SHA-256 位于 `dist/sing-box/licenses/manifest.json` 及相邻目录。

清单检测到的许可证族包括 GPL-3.0-or-later、MPL-2.0、Apache-2.0、BSD、MIT、ISC、CC0-1.0、Unlicense 和 Wintun Prebuilt License。`UNCLASSIFIED` 项仅为与 BSD 许可证同时收集的 Go `PATENTS` 文件，不是缺失的许可证。

关键 TUN 依赖：

- `github.com/sagernet/sing-tun v0.9.3`：GPL-3.0-or-later。
- `github.com/sagernet/gvisor v0.0.0-20260727.0-sing-box-mod.1`：Apache-2.0；其根目录另含 BSD/MIT 通知文件，均已收集。
- Wintun `0.14.1`：sing-tun 嵌入 `amd64/wintun.dll`，SHA-256 `e5da8447dc2c320edc0fc52fa01885c103de8c118481f683643cacc3220dafce`。该值与官方 `wintun-0.14.1.zip` 中 DLL 完全一致；官方包 SHA-256 为 `07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51`。

桌面壳固定 `tray-icon 0.25.1`，源码位于 `third_party/tray-icon-0.25.1`，并通过 Cargo `[patch.crates-io]` 使用。项目补丁仅让 Windows 固定 GUID 同样参与运行期 tooltip 的 `NIM_MODIFY`，解决 Explorer 拒绝更新的问题；上游 `LICENSE-MIT`、`LICENSE-APACHE` 和 SPDX 清单均随源码保留。

界面导航图标固定为 `lucide-static 0.468.0` 的 `chart-no-axes-combined`、`server`、`network`、`file-text` 和 `settings` SVG，资源与 ISC 原文位于 `assets/lucide/`。构建时将固定 SVG 转换为同目录 64 px PNG，并由 egui 编译时内嵌，不在运行时联网加载；图标栏保留页面名称工具提示，常规宽度保留图标加文字标签。

## 发行条件

- sing-box 及多个 SagerNet 依赖为 GPL-3.0-or-later。发行组合内核时必须附许可证和完整对应源码，包括本项目补丁、构建脚本、版本锁及用于该二进制的依赖源码；桌面应用的发行许可必须与该组合方式兼容。最终安装包不能只附一个 sing-box LICENSE 文件。
- MPL-2.0 依赖 `github.com/hashicorp/yamux v0.1.2` 要求其覆盖文件的源代码可得并保留通知。对应模块版本和许可证已锁定；发行源码材料必须包含该版本。
- Wintun 预编译许可允许 DLL 随仅通过 `wintun.h` 公开 API 使用它的软件一起分发。不得修改 DLL、移除声明、单独转售/再分发或使用 WireGuard/Wintun 名称背书。发行包必须保留官方 `LICENSE.txt`；构建脚本校验嵌入 DLL 与固定官方发布包相同。
- MIT/BSD/ISC/Apache 等依赖的版权、许可证及 NOTICE/PATENTS 原文随包保留。自动分类只用于发现遗漏，具体权利义务以收集的原文为准。

`license_inventory.py` 对缺少根许可证、Wintun 官方包校验失败或嵌入 DLL 不一致均返回失败。完整桌面安装包的许可证页面、源码材料随包方式及卸载行为仍属于任务 7.3/7.4，本记录不把安装交付标记为完成。
