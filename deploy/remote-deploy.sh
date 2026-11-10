#!/bin/sh
# 自动部署在服务器上执行的部分（.github/workflows/deploy.yml 拷到服务器目录的 deploy/ 下，再通过 SSH 调用）。
# 在 docker-compose.yml 所在目录运行；不打印 .env 的内容和任何凭据。
#
#   deploy <提交号> <GitHub 用户名>   stdin 传入 GHCR 令牌（只用来拉这一次镜像，拉完 docker logout）
#       检查 .env → 备份 → 拉镜像 → 记下当前 BOARD_IMAGE 并改成新镜像 → docker compose up -d --no-build
#       → 等容器健康检查（最多 2 分钟）；重启或健康检查失败时自动切回原来的镜像
#   rollback                          切回 deploy 时记下的镜像（核对线上提交号失败时由工作流调用）
#   cleanup                           只留当前和上一个 GHCR 镜像，再 docker image prune -f
#   set-image <镜像>                   只改 .env 的 BOARD_IMAGE（deploy/deploy.sh 手动部署前改回 board-server:latest）
set -eu
cd "$(dirname "$0")/.."

IMAGE=ghcr.io/buglan/board-server
# deploy 时记下的「部署前的镜像」，rollback / cleanup 用
PREV_FILE=.deploy-prev-image

die() { printf '✗ %s\n' "$*" >&2; exit 1; }

# 读 / 写 .env 里的 BOARD_IMAGE（没有这一行时视为 board-server:latest）
get_image() {
  v=$(sed -n 's/^BOARD_IMAGE=//p' .env | tail -n 1)
  echo "${v:-board-server:latest}"
}
set_image() {
  if grep -q '^BOARD_IMAGE=' .env; then
    # 不用 sed -i：写到临时文件再覆盖回去，.env 的权限和属主不变
    sed "s|^BOARD_IMAGE=.*|BOARD_IMAGE=$1|" .env > .env.tmp && cat .env.tmp > .env && rm -f .env.tmp
  else
    # 文件末尾没有换行时先补一个
    [ -z "$(tail -c 1 .env)" ] || echo >> .env
    echo "BOARD_IMAGE=$1" >> .env
  fi
}

# 等容器健康检查：healthy 通过；unhealthy、退出、崩溃循环或 2 分钟没结果算失败
wait_healthy() {
  i=0
  while [ $i -lt 40 ]; do
    s=$(docker inspect -f '{{.State.Health.Status}}' board 2>/dev/null || echo missing)
    case "$s" in
      healthy) echo "✓ 容器健康"; return 0 ;;
      unhealthy) break ;;
    esac
    state=$(docker inspect -f '{{.State.Status}}' board 2>/dev/null || echo missing)
    case "$state" in
      exited|dead|restarting) s=$state; break ;;
    esac
    # 崩溃循环时状态可能短暂是 running，用重启次数兜底，别等满 2 分钟
    restarts=$(docker inspect -f '{{.RestartCount}}' board 2>/dev/null || echo 0)
    if [ "${restarts:-0}" -gt 0 ]; then
      s="restarting（重启 $restarts 次）"
      break
    fi
    i=$((i + 1))
    sleep 3
  done
  echo "✗ 容器状态：${s}" >&2
  return 1
}

restore() {
  prev=$(cat "$PREV_FILE" 2>/dev/null || true)
  [ -n "$prev" ] || die "没有记录部署前的镜像，无法自动切回"
  echo "==> 切回部署前的镜像 ${prev}"
  set_image "$prev"
  docker compose up -d --no-build || true
  wait_healthy || true
  echo "==> 最近 50 行日志"
  docker compose logs --tail 50 board || true
}

case "${1:-}" in
deploy)
  commit=${2:?缺少提交号}
  user=${3:?缺少 GitHub 用户名}
  echo "$commit" | grep -Eq '^[0-9a-f]{7}$' || die "提交号格式不对：${commit}"
  target="$IMAGE:$commit"

  # 令牌从 stdin 读，只在这一次 login 里用，拉完立刻 logout，服务器上不留 GHCR 凭据。
  # 必须最先读：后面的 docker compose exec 会把 stdin 读光
  token=$(cat)
  [ -n "$token" ] || die "没有收到 GHCR 令牌"

  echo "==> 检查配置"
  [ -f .env ] || die "$(pwd)/.env 不存在"

  echo "==> 备份数据库"
  ./deploy/backup.sh </dev/null

  echo "==> 拉取 ${target}"
  printf '%s' "$token" | docker login ghcr.io -u "$user" --password-stdin >/dev/null
  unset token
  pulled=0
  docker pull -q "$target" && pulled=1
  docker logout ghcr.io >/dev/null 2>&1 || true
  [ $pulled = 1 ] || die "GHCR 上没有镜像 ${target}（提交号 ${commit}），线上不变"

  prev=$(get_image)
  echo "$prev" > "$PREV_FILE"
  echo "==> 部署前的镜像：${prev}"
  set_image "$target"

  echo "==> 重启容器"
  if ! docker compose up -d --no-build; then
    restore
    die "docker compose up 失败，已切回 ${prev}"
  fi

  echo "==> 等待健康检查"
  if ! wait_healthy; then
    restore
    die "健康检查没通过，已切回 ${prev}"
  fi
  ;;

rollback)
  restore
  ;;

set-image)
  [ -f .env ] || die "$(pwd)/.env 不存在"
  set_image "${2:?缺少镜像}"
  echo "BOARD_IMAGE=${2}"
  ;;

cleanup)
  cur=$(get_image)
  prev=$(cat "$PREV_FILE" 2>/dev/null || true)
  echo "==> 清理旧镜像（保留 ${cur}${prev:+、${prev}}）"
  docker images "$IMAGE" --format '{{.Repository}}:{{.Tag}}' | while read -r img; do
    [ "$img" = "$cur" ] || [ "$img" = "$prev" ] || [ "$img" = "$IMAGE:<none>" ] && continue
    docker rmi "$img" >/dev/null 2>&1 && echo "    删除 ${img}" || true
  done
  docker image prune -f >/dev/null
  ;;

*)
  sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
  ;;
esac
