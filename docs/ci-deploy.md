# 需求：GitHub Actions 自动部署

> 触发方式已改：推 `main` 只跑测试，推 `v*` tag 才构建、部署，tag 和 `Cargo.toml` 的 `workspace.package.version` 不一致时不部署。以 [deploy.md](deploy.md)「升级」「自动部署」为准；本文的「push main」按「推 tag」读。

推送到 `main` 后，GitHub Actions 自动测试、构建镜像、推到 GHCR，再通过 SSH 让服务器拉取新镜像并重启，最后核对线上提交号。这是唯一的部署方式，原来的手动部署脚本 `deploy/deploy.sh` 已删除。

服务器、Cloudflare Tunnel、数据目录、备份脚本都不变（见 [deploy.md](deploy.md)）。服务器只有 1 核 / 2 GB，**不在服务器上编译**，服务器只拉镜像、运行。

## 1. 流程

```
push main ─▶ build 任务（GitHub 托管的 ubuntu-latest）
               构建界面 → cargo test → 交叉编译 board-server → 打 prebuilt 镜像 → 推到 GHCR
           ─▶ deploy 任务
               SSH 到服务器：备份 → 拉镜像 → 改 .env 的 BOARD_IMAGE → docker compose up -d
               → 等健康检查 → 核对线上 /api/health 的提交号
               失败时切回上一个镜像
```

## 2. 触发

| 触发 | 行为 |
|---|---|
| push 到 `main` | 执行 build 和 deploy |
| push 到 `main`，但改动的文件全部在 `docs/**`、`prototype/**`、`lianzi/**`、`README.md` 里 | 不触发 |
| 手动运行（`workflow_dispatch`），输入 `commit` 为空 | 对 `main` 最新提交执行 build 和 deploy |
| 手动运行，输入 `commit` 为 7 位提交号 | 跳过 build，直接部署 GHCR 上已有的 `board-server:<commit>` 镜像（回滚用）；镜像不存在时 deploy 失败并报出提交号 |

- 其他分支、PR 不触发。
- 同一时间只有一次部署在跑：`concurrency: deploy`，`cancel-in-progress: false`（后来的排队，不取消正在进行的）。

## 3. build 任务

- 运行环境 `ubuntu-latest`，权限 `contents: read`、`packages: write`。
- 依次执行：
  1. `npm ci`（根目录，postinstall 会装 `web/` 的依赖）
  2. `npm run build:web`
  3. `cargo test --locked`（只测 default-members，不编译桌面端）
  4. 交叉编译：`BOARD_COMMIT=<7 位提交号> ./deploy/build-linux.sh`，产出 `deploy/bin/board-server`（zig + cargo-zigbuild，`x86_64-unknown-linux-musl`）
  5. `docker build --target prebuilt`，推送两个标签：`ghcr.io/buglan/board-server:<7 位提交号>`、`ghcr.io/buglan/board-server:latest`
- Rust 依赖用 `Swatinem/rust-cache` 缓存，npm 用 `actions/setup-node` 的缓存。
- 推送 GHCR 用 `GITHUB_TOKEN`，不用个人令牌。
- 推送后清理 GHCR：`board-server` 只保留最新 5 个版本（`latest` 指向的那个算在内），更早的删除。

## 4. deploy 任务

- 依赖 build（手动回滚时不依赖）。权限 `packages: read`。
- 通过 SSH 在服务器上执行，任何一步失败都让任务失败：
  1. 检查服务器目录里有 `.env`
  2. 把仓库里的 `docker-compose.yml`、`deploy/backup.sh` 拷到服务器目录（服务器上不再 rsync 整个仓库）
  3. 备份：`./deploy/backup.sh`
  4. 登录 GHCR：用本次任务的 `GITHUB_TOKEN`（通过 stdin 传给 `docker login ghcr.io --password-stdin`），拉取 `ghcr.io/buglan/board-server:<提交号>`，拉完 `docker logout ghcr.io`。服务器上不长期保存 GHCR 凭据
  5. 记下 `.env` 里当前的 `BOARD_IMAGE`（没有这一行时记为 `board-server:latest`），然后把它改成 `BOARD_IMAGE=ghcr.io/buglan/board-server:<提交号>`
  6. `docker compose up -d --no-build`
  7. 等容器健康检查：最多 2 分钟，`healthy` 通过；`unhealthy` 或超时算失败
  8. 核对线上：`<DEPLOY_URL>/api/health` 返回的 `commit` 等于提交号，最多试 12 次、每次间隔 5 秒
  9. 清理：删除服务器上 `ghcr.io/buglan/board-server` 除当前和上一个以外的镜像，再 `docker image prune -f`
- 第 6–8 步任何一步失败：把 `.env` 的 `BOARD_IMAGE` 改回第 5 步记下的值，`docker compose up -d --no-build`，打印 `docker compose logs --tail 50 board`，任务标记为失败。
- 日志里不出现私钥、`GITHUB_TOKEN`、`.env` 的内容。

## 5. 仓库里要改的文件

| 文件 | 改动 |
|---|---|
| `.github/workflows/deploy.yml` | 新增，实现第 2–4 节 |
| `docker-compose.yml` | `image: ${BOARD_IMAGE:-board-server:latest}` |
| `.env.example` | 加 `BOARD_IMAGE`，注明由自动部署维护，不用手改 |
| `docs/deploy.md` | 新增「自动部署」一节：一次性配置（第 6 节）、怎么看部署结果、怎么回滚；第 7 节「升级」改为说明走自动部署 |

## 6. 一次性配置

| 项 | 值 |
|---|---|
| 部署专用 SSH 密钥 | 新生成一对 ed25519 密钥，只用于 Actions；公钥加到服务器上部署用户的 `authorized_keys` |
| Secret `DEPLOY_SSH_KEY` | 上面的私钥 |
| Secret `DEPLOY_KNOWN_HOSTS` | 服务器的 `ssh-keyscan` 输出；SSH 严格校验主机密钥，不用 `StrictHostKeyChecking=no` |
| Secret `DEPLOY_HOST` | `用户@服务器`（不写进仓库；存成 Secret，日志里显示为 `***`） |
| Variable `DEPLOY_DIR` | 服务器上的仓库目录 |
| Variable `DEPLOY_URL` | `https://board.buglan.com` |

Secret 和 Variable 在仓库 Settings → Secrets and variables → Actions 里设置。

## 7. 验收标准

- [ ] 推送一个改了 `crates/` 的提交到 `main`：Actions 全部通过，线上 `/api/health` 的 `commit` 等于该提交的 7 位提交号，服务器 `.env` 里 `BOARD_IMAGE` 是 `ghcr.io/buglan/board-server:<该提交号>`，`data/backups/` 里多了一份本次部署前的备份
- [ ] 只改 `docs/` 的提交推送到 `main`：不触发工作流
- [ ] `cargo test` 失败时：不推镜像、不部署，线上版本不变
- [ ] 手动运行并输入上一个提交号：不跑 build，线上 `commit` 变为该提交号
- [ ] 手动运行并输入 GHCR 上不存在的提交号：deploy 失败，线上版本不变
- [ ] 健康检查失败时（例如部署一个启动即退出的镜像）：自动切回上一个镜像，线上 `commit` 恢复为部署前的值，任务显示失败
- [ ] 连续推送两次：第二次排队等第一次结束，不并行部署
- [ ] GHCR 上 `board-server` 不超过 5 个版本；服务器上 `ghcr.io/buglan/board-server` 的镜像不超过 2 个
- [ ] 服务器 `~/.docker/config.json` 里没有 `ghcr.io` 的凭据
- [ ] Actions 日志里搜不到私钥、令牌、`.env` 内容
- [ ] `docs/deploy.md` 按第 5 节更新
