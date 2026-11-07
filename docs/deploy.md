# 部署 Board 服务端

线上地址 `https://board.buglan.com`，服务器 `45.202.241.98`，目录 `/root/repo/board`。结构：

```
浏览器 / 桌面端 ─┐
board CLI ──────┼─ HTTPS ─▶ Cloudflare ─ Tunnel ─▶ cloudflared（服务器上的系统服务）─ HTTP ─▶ board-server（Docker，127.0.0.1:7420）
                ┘                                                                              └─ ./data/board.db
```

- HTTPS 证书在 Cloudflare 那边；服务器不开 80/443（443 被 sing-box 占用）。
- 服务器只有 1 核、2 GB 内存、约 2 GB 空闲磁盘，所以 board-server 在本机交叉编译好再上传，服务器上只打运行镜像（`BOARD_BUILD=prebuilt`，见第 2 节）。

- 网页端：浏览器打开 `https://board.buglan.com`，用 GitHub 登录，只有 `BOARD_GITHUB_USER` 指定的账号能进。
- agent：`board` 命令行设置 `BOARD_URL` 和 `BOARD_TOKEN`（访问令牌）。
- 桌面端：窗口直接打开 `https://board.buglan.com`，用 GitHub 登录时在系统默认浏览器里完成（服务端 `/auth/github?desktop=…` → 本机回环地址 → `/auth/desktop`）。桌面端依赖服务端的这两个地址，升级时先部署服务端，再发桌面端新版本。

## 1. 建 GitHub OAuth App

GitHub → Settings → Developer settings → OAuth Apps → New OAuth App：

| 项 | 值 |
|---|---|
| Application name | Board |
| Homepage URL | `https://board.buglan.com` |
| Authorization callback URL | `https://board.buglan.com/auth/github/callback` |

建好后记下 Client ID，再点 Generate a new client secret 生成 Client secret。

## 2. 配置

在本机交叉编译 Linux 版（需要 `brew install zig`、`cargo install cargo-zigbuild`、`rustup target add x86_64-unknown-linux-musl`），生成 `deploy/bin/board-server`：

```bash
./deploy/build-linux.sh
```

把仓库同步到服务器（不带 `target`、`node_modules`、`.git` 等）：

```bash
rsync -az --exclude .git/ --exclude /target/ --exclude node_modules/ --exclude /web/dist/ --exclude /.dev/ --exclude /src-tauri/gen/ --exclude /data/ --exclude /.env ./ root@45.202.241.98:repo/board/
```

之后在服务器上：

```bash
cd /root/repo/board
```

```bash
cp .env.example .env
```

编辑 `.env`，填写 `BOARD_GITHUB_CLIENT_ID`、`BOARD_GITHUB_CLIENT_SECRET`、`BOARD_GITHUB_USER`（允许登录的 GitHub 账号名），并设 `BOARD_BUILD=prebuilt`。`.env` 里有密钥，权限设为 600，不要提交。

| 环境变量 | 说明 |
|---|---|
| `BOARD_PUBLIC_URL` | 对外地址，GitHub 回调地址由它生成；`https://` 开头时 Cookie 带 `Secure` |
| `BOARD_GITHUB_CLIENT_ID` / `BOARD_GITHUB_CLIENT_SECRET` | GitHub OAuth App |
| `BOARD_GITHUB_USER` | 允许登录的 GitHub 账号（不区分大小写）；换成别的账号后，旧账号的会话立即失效 |
| `TZ` | 动态里时间的时区，默认 `Asia/Shanghai` |
| `RUST_LOG` | 日志级别，默认 `info` |
| `BOARD_BUILD` | `prebuilt`：镜像用 `deploy/bin/board-server`；`source`（默认）：在镜像里编译，需要 2 GB 以上内存和磁盘 |

`BOARD_DB`、`BOARD_HOST`、`BOARD_PORT` 已在 `docker-compose.yml` 里设好。

数据目录要属于容器里的用户（uid 10001）：

```bash
mkdir -p data && sudo chown -R 10001:10001 data
```

## 3. 启动

```bash
docker compose up -d --build
```

```bash
curl -s http://127.0.0.1:7420/api/health
```

返回 `{"ok":true}` 即正常。容器自带健康检查（`board-server healthcheck`），`docker compose ps` 里能看到状态。

## 4. Cloudflare Tunnel

`/etc/cloudflared/config.yml` 的 `ingress` 里，在最后的 `http_status:404` 之前加一条：

```yaml
  - hostname: board.buglan.com
    service: http://127.0.0.1:7420
```

校验，再建 DNS 记录（CNAME 指向这个 tunnel），最后重启。重启时同一个 tunnel 上的其他域名会断开几秒：

```bash
cloudflared tunnel --config /etc/cloudflared/config.yml ingress validate
```

```bash
cloudflared tunnel route dns 61368f29-52ea-4bd3-9e07-76ca148df803 board.buglan.com
```

```bash
systemctl restart cloudflared
```

实时推送（SSE）经过 Cloudflare 不需要额外设置：服务端每 15 秒发一次心跳，不会被空闲超时断开。用 nginx 等自建反向代理时，示例见 [`deploy/nginx.conf.example`](../deploy/nginx.conf.example)。

配好后检查：

```bash
curl -s https://board.buglan.com/api/workspace
```

应返回 `{"error":{"code":"NOT_LOGGED_IN",…}}`。浏览器打开 `https://board.buglan.com`，用 GitHub 登录。

## 5. agent 接入

在网页「设置 → Agent 接入 → 访问令牌」新建令牌（原文只显示一次），然后在 agent 的环境里：

```bash
export BOARD_URL=https://board.buglan.com BOARD_TOKEN=<访问令牌> BOARD_AS=claude BOARD=default
```

```bash
board ls
```

也可以在服务器上直接管理令牌：

```bash
docker compose exec board board-server token new ci
```

```bash
docker compose exec board board-server token ls
```

```bash
docker compose exec board board-server token revoke 3
```

## 6. 备份与恢复

[`deploy/backup.sh`](../deploy/backup.sh) 用 `board-server backup` 做在线备份（SQLite `VACUUM INTO`，服务运行时也能得到一致的快照），存到 `data/backups/`，保留 14 天。宿主机 crontab：

```cron
0 4 * * * /root/repo/board/deploy/backup.sh >> /var/log/board-backup.log 2>&1
```

`data/backups/` 还在同一台服务器上，建议再定期拷到别处。数据库很小，但服务器磁盘只剩约 2 GB，留意 `df -h`。

恢复：

```bash
docker compose stop board
```

```bash
sudo cp data/backups/board-20261107-040000.db data/board.db && sudo rm -f data/board.db-wal data/board.db-shm && sudo chown 10001:10001 data/board.db
```

```bash
docker compose start board
```

## 7. 升级

先在服务器上跑一次 `deploy/backup.sh`。然后在本机重新编译并同步（第 2 节的 `./deploy/build-linux.sh` 和 `rsync`），再在服务器上：

```bash
docker compose up -d --build
```

数据库结构在启动时自动补齐。

## 8. 日志

```bash
docker compose logs -f board
```

每个请求一行：方法、路径（不含查询参数）、状态码、耗时、是谁（GitHub 账号，或 `alpha（令牌#3）`）。令牌和 Cookie 不写进日志。日志文件按 10 MB 轮转，保留 5 个（`docker-compose.yml` 的 `logging`）。

## 本机模式

不部署、只在自己电脑上用时：

```bash
cargo run --release -p board-server -- --local
```

只监听 `127.0.0.1`，不需要登录，数据在 `~/.board/board.db`。CLI 不设置 `BOARD_URL` 时直接读写同一个数据库。
