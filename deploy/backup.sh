#!/bin/sh
# 在线备份 Board 数据库，保留最近 14 天。在 docker-compose.yml 所在目录的 deploy/ 下运行。
# 宿主机 crontab（每天 4:00）：
#   0 4 * * * <部署目录>/deploy/backup.sh >> /var/log/board-backup.log 2>&1
# 备份文件在宿主机的 data/backups/ 下。
# 可选异地同步：在 .env 里配置 BOARD_BACKUP_REMOTE 后，每次备份完把新备份和附件同步过去：
#   - user@host:/path/to/backup  用 rsync（需要 SSH 免密和 rsync）
#   - /mnt/backup/board          本地目录（挂载的移动硬盘、NFS 等），用 cp
#   - rclone:remote:path         用 rclone（例如 rclone:gdrive:board）
set -eu

fail() {
    echo "$(date '+%F %T') 失败：$*" >&2
    exit 1
}

cd "$(dirname "$0")/.." || fail "无法进入部署目录"

# 从 .env 读取可选的异地备份目标；shell 环境变量优先。
if [ -z "${BOARD_BACKUP_REMOTE:-}" ] && [ -f .env ]; then
    BOARD_BACKUP_REMOTE=$(sed -n 's/^BOARD_BACKUP_REMOTE=//p' .env | tail -n 1 | tr -d '\r')
fi

name="board-$(date +%Y%m%d-%H%M%S).db"

# 容器在跑时照旧用 exec；容器不在跑（崩溃循环、已经停了）时用一次性容器做同样的备份，
# 否则线上已经坏掉的时候，部署会卡在备份这一步，修复版本推不上去。
if [ "$(docker inspect -f '{{.State.Running}}' board 2>/dev/null || echo false)" = "true" ]; then
    docker compose exec -T board board-server backup "/data/backups/$name" || fail "备份数据库失败"
    # 文件属于容器里的用户，在容器里删
    docker compose exec -T board find /data/backups -name 'board-*.db' -mtime +14 -delete || fail "清理过期备份失败"
else
    docker compose run --rm --no-deps -T board backup "/data/backups/$name" || fail "备份数据库失败"
    # 一次性容器没有常驻的 find，用 --entrypoint 绕过镜像入口清理旧备份
    docker compose run --rm --no-deps -T --entrypoint find board /data/backups -name 'board-*.db' -mtime +14 -delete || fail "清理过期备份失败"
fi
echo "$(date '+%F %T') 已备份 data/backups/$name"

sync_remote() {
    remote=$1
    case "$remote" in
        rclone:*)
            target=${remote#rclone:}
            command -v rclone >/dev/null 2>&1 || fail "找不到 rclone，无法同步到 $remote"
            rclone copy "data/backups/$name" "$target/" || fail "rclone 同步数据库失败（$remote）"
            if [ -d data/backups/files ]; then
                rclone copy "data/backups/files" "$target/files" || fail "rclone 同步附件失败（$remote）"
            fi
            ;;
        *:*)
            # 带冒号的目标按 rsync 远端处理（本地路径不要带冒号）
            command -v rsync >/dev/null 2>&1 || fail "找不到 rsync，无法同步到 $remote"
            rsync -a "data/backups/$name" "$remote/" || fail "rsync 同步数据库失败（$remote）"
            if [ -d data/backups/files ]; then
                rsync -a "data/backups/files/" "$remote/files/" || fail "rsync 同步附件失败（$remote）"
            fi
            ;;
        *)
            mkdir -p "$remote/files" || fail "无法创建异地目录 $remote"
            cp -R "data/backups/$name" "$remote/" || fail "复制数据库到 $remote 失败"
            if [ -d data/backups/files ]; then
                cp -R "data/backups/files/." "$remote/files/" || fail "复制附件到 $remote 失败"
            fi
            ;;
    esac
}

if [ -n "${BOARD_BACKUP_REMOTE:-}" ]; then
    sync_remote "$BOARD_BACKUP_REMOTE"
    echo "$(date '+%F %T') 已同步到 $BOARD_BACKUP_REMOTE"
fi
