#!/bin/sh
# 一条命令把本机当前提交部署到服务器（docs/deploy.md 第 7 节）：
#   服务器上备份 → 本机交叉编译（build-linux.sh）→ rsync → docker compose up -d --build
#   → 等容器健康检查通过 → 核对线上 /api/health 的提交号等于本机 HEAD
#
# 用法：
#   ./deploy/deploy.sh             部署
#   ./deploy/deploy.sh --dry-run   只检查本机、列出要执行的步骤，不编译、不连服务器
#   ./deploy/deploy.sh --check     只核对线上的提交号是不是本机 HEAD
#
# 环境变量：
#   BOARD_DEPLOY_HOST   SSH 目标，默认 root@45.202.241.98
#   BOARD_DEPLOY_DIR    服务器上的仓库目录，默认 /root/repo/board
#   BOARD_DEPLOY_URL    核对用的对外地址，默认 https://board.buglan.com
#   ALLOW_DIRTY=1       工作区有未提交的改动时也部署（线上提交号带 -dirty）
set -eu
cd "$(dirname "$0")/.."

HOST=${BOARD_DEPLOY_HOST:-root@45.202.241.98}
DIR=${BOARD_DEPLOY_DIR:-/root/repo/board}
URL=${BOARD_DEPLOY_URL:-https://board.buglan.com}
URL=${URL%/}

mode=deploy
case "${1:-}" in
  "") ;;
  --dry-run) mode=dry ;;
  --check) mode=check ;;
  -h | --help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "不认识的参数：${1}（--dry-run、--check、--help）" >&2; exit 2 ;;
esac

step() { printf '\n==> %s\n' "$*"; }
die() { printf '✗ %s\n' "$*" >&2; exit 1; }
remote() {
  if [ "$mode" = dry ]; then
    printf '    ssh %s %s\n' "$HOST" "$1"
  else
    ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST" "$1"
  fi
}

# 线上 /api/health 返回的提交号（取不到时为空）
live_commit() {
  curl -fsS --max-time 15 "$URL/api/health?t=$(date +%s)" 2>/dev/null |
    sed -n 's/.*"commit":"\([^"]*\)".*/\1/p'
}

# 核对线上的提交号，最多试 $2 次、每次间隔 5 秒（部署刚完成时线上可能还是旧进程）
verify() {
  want=$1
  got=
  i=0
  while :; do
    got=$(live_commit || true)
    [ "$got" = "$want" ] && { echo "✓ 线上 $URL 是 $want"; return 0; }
    i=$((i + 1))
    [ $i -ge "$2" ] && break
    sleep 5
  done
  die "线上 $URL 的提交号是「${got:-取不到}」，本机是「${want}」"
}

head=$(git rev-parse --short=7 HEAD)

if [ "$mode" = check ]; then
  got=$(live_commit || true)
  # ALLOW_DIRTY=1 部署出去的是 HEAD 加上当时未提交的改动
  if [ "$got" = "${head}-dirty" ]; then
    echo "✓ 线上 $URL 是 ${got}（本机 HEAD，部署时带着未提交的改动）"
    exit 0
  fi
  verify "$head" 1
  exit 0
fi

# 线上版本 = 本机 HEAD；有未提交的改动时，部署出去的内容和提交对不上
# （tsconfig.tsbuildinfo 是 tsc 的构建缓存，编译界面时会被改写，不算改动）
commit=$head
dirty=$(git status --porcelain --untracked-files=no -- . ':(exclude)web/tsconfig.tsbuildinfo')
if [ -n "$dirty" ]; then
  [ "${ALLOW_DIRTY:-}" = 1 ] || die "工作区有未提交的改动，提交后再部署（或 ALLOW_DIRTY=1）：
$dirty"
  commit="$head-dirty"
fi

echo "部署 $commit → $HOST:${DIR}（${URL}）"
[ "$mode" = dry ] && echo "（演练：不编译、不连服务器，只列出步骤）"

step "检查服务器配置"
# 服务器只有 1 核 / 2 GB：必须用本机编译好的可执行文件，不能在镜像里编译
remote "grep -q '^BOARD_BUILD=prebuilt' $DIR/.env" || die "服务器 $DIR/.env 里没有 BOARD_BUILD=prebuilt"

step "备份数据库"
remote "cd $DIR && ./deploy/backup.sh"

step "本机交叉编译（提交号 ${commit}）"
if [ "$mode" = dry ]; then
  echo "    BOARD_COMMIT=$commit ./deploy/build-linux.sh"
else
  BOARD_COMMIT=$commit ./deploy/build-linux.sh
fi

step "同步到服务器"
set -- -az --exclude .git/ --exclude /target/ --exclude node_modules/ --exclude /web/dist/ --exclude /.dev/ \
  --exclude /src-tauri/gen/ --exclude /data/ --exclude /.env ./ "$HOST:$DIR/"
if [ "$mode" = dry ]; then
  echo "    rsync $*"
else
  rsync -e "ssh -o BatchMode=yes -o ConnectTimeout=15" "$@"
fi

step "重建并重启容器"
# 自动部署会把 .env 的 BOARD_IMAGE 改成 GHCR 镜像；手动部署用本机构建的镜像，先改回 board-server:latest，
# 让 .env 始终对应正在运行的镜像（也免得 --build 把本机构建的镜像打成 GHCR 的标签）
remote "cd $DIR && ./deploy/remote-deploy.sh set-image board-server:latest"
remote "cd $DIR && docker compose up -d --build"

step "等待健康检查"
# 容器的 HEALTHCHECK 每 30 秒一次，启动后 10 秒开始，最多等 2 分钟
remote "cd $DIR && for i in \$(seq 1 40); do s=\$(docker inspect -f '{{.State.Health.Status}}' board 2>/dev/null); [ \"\$s\" = healthy ] && echo '✓ 容器健康' && exit 0; [ \"\$s\" = unhealthy ] && break; sleep 3; done; echo \"✗ 容器状态：\$s\" >&2; docker compose logs --tail 30 board >&2; exit 1"

step "核对线上版本"
if [ "$mode" = dry ]; then
  echo "    curl $URL/api/health → commit 应为 $commit"
else
  verify "$commit" 12
fi
