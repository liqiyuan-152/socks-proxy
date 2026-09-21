#!/bin/sh
# 在仓库运行；环境配置了失效镜像时可由调用方设置 CARGO_HOME 并从 /tmp 使用 --manifest-path。
set -eu
cargo check --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
