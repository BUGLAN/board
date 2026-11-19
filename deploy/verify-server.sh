#!/bin/sh
# 部署后的验收：同时核对 GitHub 登录模式、版本、提交号和首页 HTML。
# 用法（在服务器目录）：./deploy/verify-server.sh <版本> <7 位提交号>
# BOARD_HEALTH_WAIT 是等待服务重启的秒数，默认 45。
set -eu
cd "$(dirname "$0")/.."

version=${1:-}
commit=${2:-}
if [ -z "$version" ] || [ -z "$commit" ]; then
  echo "用法：$0 <版本> <7 位提交号>" >&2
  exit 2
fi
if [ ! -f .env ]; then
  echo "✗ $(pwd)/.env 不存在，无法确认线上模式" >&2
  exit 1
fi

set -a
. ./.env
set +a
port=${BOARD_PORT:-7420}

exec ./bin/board-server healthcheck \
  --url "http://127.0.0.1:${port}" \
  --expect-mode github \
  --expect-version "$version" \
  --expect-commit "$commit" \
  --wait "${BOARD_HEALTH_WAIT:-45}"
