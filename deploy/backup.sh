#!/bin/sh
# 在线备份 Board 数据库，保留最近 14 天。在 docker-compose.yml 所在目录的 deploy/ 下运行。
# 宿主机 crontab（每天 4:00）：
#   0 4 * * * /root/repo/board/deploy/backup.sh >> /var/log/board-backup.log 2>&1
# 备份文件在宿主机的 data/backups/ 下；建议再定期拷到这台服务器以外的地方。
set -eu
cd "$(dirname "$0")/.."
name="board-$(date +%Y%m%d-%H%M%S).db"
docker compose exec -T board board-server backup "/data/backups/$name"
# 文件属于容器里的用户，在容器里删
docker compose exec -T board find /data/backups -name 'board-*.db' -mtime +14 -delete
echo "$(date '+%F %T') 已备份 data/backups/$name"
