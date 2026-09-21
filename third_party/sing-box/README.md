# 严格缓存模式补丁

基于 source-lock.json 固定的官方源代码，维护 strict-cache.patch。
配置 `experimental.cache_file.strict_mode: true` 后，已有缓存先只读验证；空/损坏文件报错且不删除。运行期已检测数据库损坏输出 STRICT_CACHE_ERROR，以 78 退出，不重建数据库。默认 false 保持上游行为。

## 构建

安装 source-lock.json 指定 Go 版本、Python 3、git，然后在项目目录执行：

```sh
python3 third_party/sing-box/build.py
```

脚本验证源压缩包 SHA-256、应用补丁、运行本地缓存测试，再构建 Windows amd64 可执行程序和测试程序。dist/sing-box 包含补丁后的对应源码包、构建材料和校验清单。使用 with_gvisor,with_clash_api 标签，未复制官方发行包所有可选特性，需单独验证本项目所需能力。构建还会按相同 Windows 目标和标签运行 license_inventory.py，收集实际链接模块及 Go 标准库的根许可证；缺少许可证文件时构建失败。

## 控制器责任

- 应用生成配置必须启用严格模式；启动时识别固定错误标记，运行期识别退出 78。
- 记录缓存曾经初始化的事实，区分首次创建与旧映射文件被删除。
- 运行期退出不保证优雅清理路由，由控制器执行网络恢复，不能承诺零直连。
- 显式重建前提示应用缓存影响并隔离保留旧文件；当前补丁不自动实现 Rust UI 和恢复控制器。
- 测试覆盖检测到的结构损坏和故障注入，不保证发现任意逻辑篡改、所有掉电窗口或未被库检测的损坏。

## 发行

保留 LICENSE.upstream、官方来源、版本后缀、补丁、对应完整源码和 licenses/manifest.json。sing-box 是 GPL-3.0-or-later；发行组合二进制时应用须按 GPL 兼容方式提供完整对应源码和许可证通知。自动分类仅用于发现遗漏，不替代许可证原文。不要把当前验证包当作完整桌面应用。
