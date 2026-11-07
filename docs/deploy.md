# 部署 Board 服务端

线上地址 `https://board.buglan.com`。服务器和目录不写进仓库：本机用环境变量 `BOARD_DEPLOY_HOST`（用户@服务器）、`BOARD_DEPLOY_DIR`（服务器上的仓库目录），GitHub 上是 Variables `DEPLOY_HOST`、`DEPLOY_DIR`。结构：

```
浏览器 / 桌面端 ─┐
board CLI ──────┼─ HTTPS ─▶ Cloudflare ─ Tunnel ─▶ cloudflared（服务器上的系统服务）─ HTTP ─▶ board-server（Docker，127.0.0.1:7420）
                ┘                                                                              └─ ./data/board.db
```

- HTTPS 证书在 Cloudflare 那边；服务器不开 80/443（443 被 sing-box 占用）。
- 服务器只有 1 核、2 GB 内存、约 2 GB 空闲磁盘，所以服务器上不编译：默认由 GitHub Actions 交叉编译、打镜像推到 GHCR，服务器只拉镜像运行（见「自动部署」）；手动部署时在本机交叉编译好再上传，服务器上只打运行镜像（`BOARD_BUILD=prebuilt`，见第 2 节）。

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
rsync -az --exclude .git/ --exclude /target/ --exclude node_modules/ --exclude /web/dist/ --exclude /.dev/ --exclude /src-tauri/gen/ --exclude /data/ --exclude /.env ./ "$BOARD_DEPLOY_HOST:$BOARD_DEPLOY_DIR/"
```

之后在服务器上：

```bash
cd <部署目录>
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

[`deploy/backup.sh`](../deploy/backup.sh) 用 `board-server backup` 做在线备份（SQLite `VACUUM INTO`，服务运行时也能得到一致的快照），存到 `data/backups/`，保留 14 天。卡片附件在 `data/files/`（按内容命名、写入后不变），备份时把还没有的文件补到 `data/backups/files/`，所有备份共用这一份。宿主机 crontab：

```cron
0 4 * * * <部署目录>/deploy/backup.sh >> /var/log/board-backup.log 2>&1
```

`data/backups/` 还在同一台服务器上，建议再定期拷到别处。数据库很小，但服务器磁盘只剩约 2 GB，附件（单个不超过 10 MB）会占用空间，留意 `df -h`。

恢复：

```bash
docker compose stop board
```

```bash
sudo cp data/backups/board-20261107-040000.db data/board.db && sudo rm -f data/board.db-wal data/board.db-shm && sudo chown 10001:10001 data/board.db
```

附件：把备份里的文件补回 `data/files/`（已有的不覆盖）：

```bash
sudo mkdir -p data/files && sudo cp -n data/backups/files/* data/files/ && sudo chown -R 10001:10001 data/files
```

```bash
docker compose start board
```

## 7. 升级

默认走自动部署：推送到 `main` 即自动测试、构建、部署（见下文「自动部署」）。

`./deploy/deploy.sh` 保留为手动方式（例如 GitHub Actions 不可用时）。在本机仓库里，提交好要上线的改动后：

```bash
./deploy/deploy.sh
```

它依次执行：检查服务器 `.env` 是 `BOARD_BUILD=prebuilt` → 服务器上跑 `deploy/backup.sh` → 本机 `deploy/build-linux.sh`（把提交号编进可执行文件）→ 按第 2 节的排除规则 rsync → 服务器上 `docker compose up -d --build` → 等容器健康检查通过 → 核对线上 `/api/health` 返回的提交号等于本机 `HEAD`。任何一步失败都停下并报错。

- `./deploy/deploy.sh --dry-run`：只检查本机、列出要执行的步骤，不编译、不连服务器
- `./deploy/deploy.sh --check`：只核对线上是不是本机 `HEAD`
- 工作区有未提交的改动时拒绝部署；确实要部署时加 `ALLOW_DIRTY=1`，线上提交号带 `-dirty`
- 服务器和目录必须用环境变量给出：`BOARD_DEPLOY_HOST`（用户@服务器）、`BOARD_DEPLOY_DIR`（服务器上的仓库目录），没有设置时脚本直接退出；对外地址 `BOARD_DEPLOY_URL` 默认 `https://board.buglan.com`
- 重建容器前把服务器 `.env` 的 `BOARD_IMAGE` 改成 `board-server:latest`（本机构建的镜像），`.env` 始终对应正在运行的镜像；之后再推送 `main`，自动部署照常进行

桌面端依赖服务端的新接口时，先部署服务端，再发桌面端。数据库结构在启动时自动补齐。

## 自动部署

`.github/workflows/deploy.yml`。推送到 `main` 后：

```
build（GitHub 托管的 ubuntu-latest）：npm ci → 构建界面 → cargo test → deploy/build-linux.sh 交叉编译 → prebuilt 镜像
                                     → 推到 GHCR：ghcr.io/buglan/board-server:<7 位提交号> 和 :latest
deploy（SSH 到服务器，deploy/remote-deploy.sh）：备份 → 拉镜像 → .env 的 BOARD_IMAGE 改成新镜像
                                     → docker compose up -d --no-build → 等健康检查 → 核对线上 /api/health 的提交号
```

- 改动全部在 `docs/`、`prototype/`、`lianzi/`、`README.md` 里时不触发；其他分支、PR 不触发
- 同一时间只有一次部署，后来的排队
- `cargo test` 失败时不推镜像、不部署
- 重启、健康检查（最多 2 分钟）或核对提交号失败：`.env` 的 `BOARD_IMAGE` 改回部署前的镜像并重启，打印最近 50 行日志，任务失败
- 服务器不编译：只拉镜像、运行；服务器上 `docker-compose.yml` 和 `deploy/backup.sh`、`deploy/remote-deploy.sh` 由工作流拷过去
- 拉镜像用本次任务的 `GITHUB_TOKEN`（经 stdin 传给 `docker login`），拉完立刻 `docker logout`，服务器上不留 GHCR 凭据
- 清理：GHCR 上只保留最新 5 个版本；服务器上只留当前和上一个 `ghcr.io/buglan/board-server` 镜像

### 一次性配置

1. 生成部署专用的 SSH 密钥（只给 Actions 用），公钥加到服务器上部署用户的 `authorized_keys`：

   ```bash
   ssh-keygen -t ed25519 -N '' -C board-deploy -f board-deploy
   ssh-copy-id -i board-deploy.pub "$BOARD_DEPLOY_HOST"
   ssh-keyscan -t ed25519 "${BOARD_DEPLOY_HOST#*@}"      # 输出填到 DEPLOY_KNOWN_HOSTS
   ```

2. GitHub 仓库 Settings → Secrets and variables → Actions：

   | 类型 | 名称 | 值 |
   |---|---|---|
   | Secret | `DEPLOY_SSH_KEY` | 上面的私钥 `board-deploy` 的内容 |
   | Secret | `DEPLOY_KNOWN_HOSTS` | `ssh-keyscan` 的输出（SSH 严格校验主机密钥） |
   | Variable | `DEPLOY_HOST` | `用户@服务器`（同本机的 `BOARD_DEPLOY_HOST`） |
   | Variable | `DEPLOY_DIR` | 服务器上的仓库目录（同 `BOARD_DEPLOY_DIR`） |
   | Variable | `DEPLOY_URL` | `https://board.buglan.com` |

3. 服务器 `.env` 里不用加 `BOARD_IMAGE`：第一次自动部署时会加上。

### 看部署结果

- GitHub 仓库 Actions → deploy：每一步的输出；失败时 deploy 任务最后打印了容器最近 50 行日志
- 线上版本：`curl -s https://board.buglan.com/api/health` 的 `commit`，或本机 `./deploy/deploy.sh --check`
- 服务器上正在运行的镜像：`.env` 的 `BOARD_IMAGE`

### 回滚

Actions → deploy → Run workflow，`commit` 填要回到的 7 位提交号：跳过构建，直接部署 GHCR 上已有的 `board-server:<提交号>` 镜像。GHCR 只保留最新 5 个版本，更早的提交号会报「GHCR 上没有镜像」，线上不变。`commit` 留空则构建并部署 `main` 最新提交。

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
