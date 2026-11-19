#!/bin/sh
# 服务器运行入口（放在服务器目录，和 bin/、data/、.env 同级）。
# 线上只接受 GitHub 登录模式；.env 缺失或配置不完整时直接退出，绝不退回 --local --demo。
set -eu
cd "$(dirname "$0")"

if [ ! -f .env ]; then
  echo "✗ $(pwd)/.env 不存在：线上服务必须使用 GitHub 登录配置，不能退回本机模式" >&2
  exit 1
fi

# 线上配置以这个 .env 为准。board host 给 agent 的环境里也有 BOARD_HOST（机器名），
# 不能让它被 board-server 当成监听地址；否则从 board host 会话里启动会变成 --host mac。
unset BOARD_PUBLIC_URL BOARD_GITHUB_CLIENT_ID BOARD_GITHUB_CLIENT_SECRET BOARD_GITHUB_USER BOARD_HOST BOARD_PORT
set -a
. ./.env
set +a

missing=
[ -n "${BOARD_PUBLIC_URL:-}" ] || missing="${missing} BOARD_PUBLIC_URL"
[ -n "${BOARD_GITHUB_CLIENT_ID:-}" ] || missing="${missing} BOARD_GITHUB_CLIENT_ID"
[ -n "${BOARD_GITHUB_CLIENT_SECRET:-}" ] || missing="${missing} BOARD_GITHUB_CLIENT_SECRET"
[ -n "${BOARD_GITHUB_USER:-}" ] || missing="${missing} BOARD_GITHUB_USER"
if [ -n "$missing" ]; then
  echo "✗ .env 缺少线上登录配置：${missing}" >&2
  exit 1
fi

exec ./bin/board-server \
  --host "${BOARD_HOST:-0.0.0.0}" \
  --port "${BOARD_PORT:-7420}" \
  --db "$PWD/data/board.db"
