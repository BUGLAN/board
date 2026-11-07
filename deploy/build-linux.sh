#!/bin/sh
# 在本机（macOS / Linux）交叉编译 Linux x86_64 的 board-server，输出到 deploy/bin/board-server。
# 服务器内存或磁盘不够在镜像里编译时用这种方式，.env 里设 BOARD_BUILD=prebuilt。
# 需要：zig、cargo-zigbuild、rustup target add x86_64-unknown-linux-musl
set -eu
cd "$(dirname "$0")/.."
npm run build:web
cargo zigbuild --release --locked --target x86_64-unknown-linux-musl -p board-server
mkdir -p deploy/bin
cp target/x86_64-unknown-linux-musl/release/board-server deploy/bin/board-server
echo "已生成 deploy/bin/board-server"
