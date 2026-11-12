#!/bin/sh
# 在本机（macOS / Linux）交叉编译 Linux x86_64 的 board-server，输出到 deploy/bin/board-server。
# 服务器内存或磁盘不够在镜像里编译时用这种方式，.env 里设 BOARD_BUILD=prebuilt。
# 需要：zig、cargo-zigbuild、rustup target add x86_64-unknown-linux-musl
# 界面（web/dist）不在了，或者界面代码刚改过时先在这里构建；已经构建过的地方（例如 CI 的
# 「构建界面」步骤）加 --skip-web，不要重复构建一次
set -eu
cd "$(dirname "$0")/.."
skip_web=false
for arg in "$@"; do
  case "$arg" in
    --skip-web) skip_web=true ;;
    *) echo "未知参数：$arg（可用：--skip-web）" >&2; exit 2 ;;
  esac
done
if [ "$skip_web" = false ]; then
  npm run build:web
fi
cargo zigbuild --release --locked --target x86_64-unknown-linux-musl -p board-server
mkdir -p deploy/bin
cp target/x86_64-unknown-linux-musl/release/board-server deploy/bin/board-server
echo "已生成 deploy/bin/board-server"
