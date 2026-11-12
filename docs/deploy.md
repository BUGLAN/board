# 部署 Board 服务端

下文用 `https://board.example.com` 举例，换成自己的域名。自动部署用到的服务器地址和目录不写进仓库，只存在 GitHub 的 Secret `DEPLOY_HOST` 和 Variable `DEPLOY_DIR` 里。结构：

```
浏览器 / 桌面端 ─┐
board CLI ──────┼─ HTTPS ─▶ Cloudflare ─ Tunnel ─▶ cloudflared（服务器上的系统服务）─ HTTP ─▶ board-server（Docker，127.0.0.1:7420）
                ┘                                                                              └─ ./data/board.db
```

- HTTPS 证书在 Cloudflare 那边，服务器不用开 80/443。
- 小内存服务器上不编译：推 tag 后由 GitHub Actions 交叉编译、打镜像推到 GHCR，服务器只拉镜像运行（见「自动部署」）；首次部署时在本机交叉编译好再上传，服务器上只打运行镜像（`BOARD_BUILD=prebuilt`，见第 2 节）。

- 网页端：浏览器打开 `https://board.example.com`，用 GitHub 登录，只有 `BOARD_GITHUB_USER` 指定的账号能进。
- agent：`board login --url <服务端地址> --token <访问令牌>` 写进 `~/.board/config.toml`（也可以用环境变量 `BOARD_URL`、`BOARD_TOKEN`，优先于配置文件）。
- 桌面端：窗口打开 Board 服务端的页面。连哪个服务端依次取环境变量 `BOARD_URL`、配置文件 `~/.board/config.toml` 的 `url`（和 `board login` 同一项）、构建时注入的默认地址 `BOARD_DEFAULT_URL`，都没有时显示「连接服务端」页由人填写（见下文「自己部署的桌面端」）。代码里不写死地址。用 GitHub 登录时在系统默认浏览器里完成（服务端 `/auth/github?desktop=…` → 本机回环地址 → `/auth/desktop`）。桌面端依赖服务端的这两个地址，升级时先部署服务端，再发桌面端新版本。

## 1. 建 GitHub OAuth App

GitHub → Settings → Developer settings → OAuth Apps → New OAuth App：

| 项 | 值 |
|---|---|
| Application name | Board |
| Homepage URL | `https://board.example.com` |
| Authorization callback URL | `https://board.example.com/auth/github/callback` |

建好后记下 Client ID，再点 Generate a new client secret 生成 Client secret。

## 2. 配置

在本机交叉编译 Linux 版（需要 `brew install zig`、`cargo install cargo-zigbuild`、`rustup target add x86_64-unknown-linux-musl`），生成 `deploy/bin/board-server`：

```bash
./deploy/build-linux.sh
```

把仓库同步到服务器（不带 `target`、`node_modules`、`.git` 等）：

```bash
rsync -az --exclude .git/ --exclude /target/ --exclude node_modules/ --exclude /web/dist/ --exclude /.dev/ --exclude /src-tauri/gen/ --exclude /data/ --exclude /.env ./ <用户>@<服务器>:<部署目录>/
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
  - hostname: board.example.com
    service: http://127.0.0.1:7420
```

校验，再建 DNS 记录（CNAME 指向这个 tunnel），最后重启。重启时这个 tunnel 上的所有域名会断开几秒：

```bash
cloudflared tunnel --config /etc/cloudflared/config.yml ingress validate
```

```bash
cloudflared tunnel route dns <tunnel-id> board.example.com
```

```bash
systemctl restart cloudflared
```

实时推送（SSE）经过 Cloudflare 不需要额外设置：服务端每 15 秒发一次心跳，不会被空闲超时断开。用 nginx 等自建反向代理时，示例见 [`deploy/nginx.conf.example`](../deploy/nginx.conf.example)。

配好后检查：

```bash
curl -s https://board.example.com/api/workspace
```

应返回 `{"error":{"code":"NOT_LOGGED_IN",…}}`。浏览器打开 `https://board.example.com`，用 GitHub 登录。

## 5. agent 接入

在网页「设置 → Agent 接入 → 访问令牌」新建令牌（原文只显示一次），页面上给出一条命令，在 agent 所在的电脑上执行：

```bash
board login --url https://board.example.com --token <访问令牌>
```

它先校验令牌，再写进 `~/.board/config.toml`（权限 0600）。之后：

```bash
board doctor
board check --as claude/opus-5.5
board ls --as claude/opus-5.5 --board default
```

agent 的名字格式是 `<运行环境>/<模型>`（需要时加后缀），会话开始时 `board check` 占用；之后 `board heartbeat start` 在后台每 10 秒发一次 `board beat` 维持在线，会话结束时 `board heartbeat stop`。

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

### 异地同步（可选）

默认只备份到这台服务器的 `data/backups/`，磁盘坏了还是会全部丢失。在 `.env` 里配置 `BOARD_BACKUP_REMOTE` 后，`backup.sh` 每次备份完会把新备份和 `backups/files/` 同步过去：

| 值 | 同步方式 | 需要 |
|---|---|---|
| `user@host:/path/board-backup` | rsync over SSH | 宿主机装 rsync，配好 SSH 免密 |
| `/mnt/backup/board` | `cp -R` 到本地目录 | 目录已挂载（移动硬盘、NFS 等） |
| `rclone:gdrive:board-backup` | `rclone copy` | 宿主机装 rclone 并配好 remote |

不配置时和以前一样，只写 `data/backups/`。脚本任何一步失败都会返回非零，并在最后一行打印「失败」；crontab 把输出重定向到 `/var/log/board-backup.log` 后，可以据此做告警。

`data/backups/` 还在同一台服务器上，数据库很小，但服务器磁盘小的话，附件（单个不超过 10 MB）会占用空间，留意 `df -h`。

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

### 恢复演练

建议每次大版本发版前在本机走一遍，确认备份真的能恢复：

1. 取最新一份备份和附件：`ls -lt data/backups | head`，把 `board-YYYYMMDD-HHMMSS.db` 和 `data/backups/files/` 拷到本机一个临时目录。
2. 在本机用这份备份启动一个临时服务（附件目录要和数据库文件放在同一层，服务端按数据库旁边的 `files/` 找附件）：

   ```bash
   mkdir -p /tmp/board-restore
   cp data/backups/board-YYYYMMDD-HHMMSS.db /tmp/board-restore/board.db
   cp -R data/backups/files /tmp/board-restore/files
   cargo run --release -p board-server -- --local --db /tmp/board-restore/board.db --port 7421
   ```

3. 浏览器打开 `http://127.0.0.1:7421`，核对：卡片、项目数量与线上一致，随机打开一张带附件的卡片能看到附件。
4. 核对完 `Ctrl-C` 停掉临时服务，删除临时目录。

## 7. 升级

发版由人决定：推 `vX.Y.Z` tag 才构建、部署线上服务端（见下文「自动部署」）；推 `main` 只跑测试，不部署。版本号规则见 [AGENTS.md](../AGENTS.md)「发版」：x 只由人决定；y 是重构或大升级；z 是修复和比较小的新功能。第一个 tag 是 `v0.1.0`。

在 `main` 分支或分离 HEAD 上、工作区干净时执行（不用先手动 `git pull`）：

```bash
npm run release -- 0.1.1 --dry-run   # 先看这一版有哪些提交（按 origin/main 列），不改任何文件
npm run release -- 0.1.1 --push      # 发版并推送
```

`scripts/release.mjs` 依次：

1. 拉到最新：`git fetch`；工作区要干净；本地只是落后于 `origin/main`（别的电脑先推了）时自动快进过去；本地有没推到 `main` 的提交、或在别的分支上时中止
2. 核对版本号格式、tag 还不存在、比上一个 tag 大
3. 从上一个 tag 之后的每个提交生成这一版的 changelog，按提交类型分组（新功能、修复、性能、重构、文档、其他），写到 `CHANGELOG.md` 最上面
4. 改版本号：`Cargo.toml` 的 `workspace.package.version` 和 `Cargo.lock`、`package.json` 和 `web/package.json`（连同 `package-lock.json`）、`src-tauri/tauri.conf.json`
5. 提交「chore: 发版 vX.Y.Z」，打附注 tag `vX.Y.Z`，tag 说明里是这一版的 changelog
6. 带 `--push` 时推 `main`，再推 tag；不带时只打印这两条推送命令。推 `main` 被拒（发版途中别人推了）时撤掉这次的发版提交和 tag，拉到最新后自动再发一次；第二次还被拒、或推送因为别的原因失败时，撤掉后停下，说明原因

tag 去掉 `v` 后和 `workspace.package.version` 不一致，或者 `CHANGELOG.md` 里没有这一版的一节时，不部署。tag 推出去后不改、不删、不挪，发错了就再发一个版本。同一个 tag 还构建桌面端安装包、打包 skill，部署成功后一起附到 GitHub Release（见「自动部署」「桌面端安装包」）。

桌面端依赖服务端的新接口时，先部署服务端，再发桌面端。数据库结构在启动时自动补齐。

**agent 名字查重、agent 级心跳、CLI 配置文件**这一版的服务端、CLI、skill 要一起发布：

- 新服务端要求带令牌的卡片命令、上传附件里的名字先 `board check` 并保持心跳（严格模式）。旧名字（例如 `claude`）不符合新格式，新服务端上线后会报 `NAME_OFFLINE`。
- 每台跑 agent 的电脑重装 CLI：`cargo install --path crates/board-cli`（或装新版桌面端），再 `board login --url <服务端地址> --token <令牌>`，删掉 shell 配置文件里旧的 `export BOARD_URL`、`export BOARD_TOKEN`（`board doctor` 会提示）。
- 更新 skill（`.agents/skills/board-skills/`，含 `scripts/`；发版附件 `board-skills.zip` 还带 `bin/` 里各平台的 `board`）。

## 自动部署

`.github/workflows/deploy.yml`：

| 触发 | 做什么 |
|---|---|
| 推 `main` | test：`npm ci` → `cargo test` → 界面类型检查（`npm --prefix web run typecheck`）→ 界面单元测试（`npm --prefix web test`）；同时在 Windows 上跑 `board heartbeat` 的测试和多行正文经过 skill 入口 `board.cmd` 的测试（`cargo test -p board-cli --test heartbeat --test multiline`）。不构建镜像、不部署。改动全部在 `docs/`、`README.md` 里时不跑 |
| 推 `v*` tag | build → deploy；skill、desktop（和它们同时跑）→ release，见下图 |
| 在 `main` 上手动运行，填 7 位提交号 | 回滚，见下文「回滚」 |

```
build（GitHub 托管的 ubuntu-latest）：核对 tag、Cargo.toml 的版本号和 CHANGELOG.md → npm ci → 构建界面 → cargo test
                                     → deploy/build-linux.sh 交叉编译 → prebuilt 镜像
                                     → 推到 GHCR：ghcr.io/buglan/board-server:<7 位提交号> 和 :latest
deploy（SSH 到服务器，deploy/remote-deploy.sh）：备份 → 拉镜像 → .env 的 BOARD_IMAGE 改成新镜像
                                     → docker compose up -d --no-build → 等健康检查
                                     → 核对线上 /api/health：version 是 tag 的版本号，commit 是 tag 指向的提交
skill（GitHub 托管的 macos-latest）：node scripts/build-skill-bin.mjs --zip，编 macOS（arm64、x64）、Linux（x64、arm64，musl）、
                                     Windows（x64）的 board，放进 skill 的 bin/，打包成 board-skills.zip
desktop（GitHub 托管的 macos-latest、windows-latest，两个平台同时跑）：核对 tag 和 tauri.conf.json 的版本号
                                     → npm ci → 编安装包自带的 board → tauri build（带更新签名）→ 核对安装包里的 board --version
                                     macOS：通用（Apple Silicon + Intel）.dmg；Windows：NSIS 安装包 .exe、.msi 和免安装便携 .zip
                                     → 收齐更新包和签名（updater/<平台键>/）→ 生成这个平台的更新清单
release：用 CHANGELOG.md 里这一版的一节建 GitHub Release，附上 board-skills.zip、桌面端安装包、更新包和 latest.json
         （部署成功、skill 打包成功、两个平台的安装包和更新清单都构建成功后）
```

- tag 去掉 `v` 后和 `Cargo.toml` 的 `workspace.package.version` 不一致，或者 `CHANGELOG.md` 里没有这一版的一节：build 第一步失败，不推镜像、不部署
- 桌面端安装包任一平台构建失败：不建 Release（不留缺附件的 Release），服务端照常构建、部署；修好后再发一个版本
- PR、其他分支不触发
- 同一时间只有一次部署，后来的排队
- `cargo test` 失败时不推镜像、不部署
- 重启、健康检查（最多 2 分钟）或核对版本号、提交号失败：`.env` 的 `BOARD_IMAGE` 改回部署前的镜像并重启，打印最近 50 行日志，任务失败
- 服务器不编译：只拉镜像、运行；服务器上 `docker-compose.yml` 和 `deploy/backup.sh`、`deploy/remote-deploy.sh` 由工作流拷过去
- 拉镜像用本次任务的 `GITHUB_TOKEN`（经 stdin 传给 `docker login`），拉完立刻 `docker logout`，服务器上不留 GHCR 凭据
- 清理：GHCR 上只保留最新 5 个版本；服务器上只留当前和上一个 `ghcr.io/buglan/board-server` 镜像

### 桌面端安装包

GitHub Release 里的附件，都从 tag 指向的提交构建（`.github/workflows/deploy.yml` 的 desktop 任务）：

| 文件 | 平台 |
|---|---|
| `Board_<版本>_universal.dmg` | macOS（Apple Silicon 和 Intel 通用） |
| `Board_<版本>_x64-setup.exe` | Windows x64，NSIS 安装包（推荐） |
| `Board_<版本>_x64_en-US.msi` | Windows x64，MSI 安装包 |
| `Board_<版本>_x64_portable.zip` | Windows x64，便携版（免安装，解压即用） |
| `board-skills.zip` | skill，`bin/` 里带各平台的 `board` |
| `Board_<版本>_universal.app.tar.gz`、`latest.json` | 桌面端「检查更新」用的更新包和清单（见下文「桌面端检查更新」），不用手动下载 |

- 安装包里有界面和自带的 `board` 命令行（Windows 安装时写进 PATH；macOS 在 `Board.app/Contents/MacOS/board`），装好后 `board --version` 打印版本号。
- 便携版 `Board_<版本>_x64_portable.zip` 是免安装的绿色版：解压到任意目录，直接运行里面的 `board-desktop.exe`；内容就是安装版装进安装目录的那几样（桌面端、`board` 命令行等）。不带 WebView2 引导（安装版在安装时检测、自动下载 WebView2），系统没有 WebView2 运行时（老版本 Windows、精简系统）时先装 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)，或改用安装版；要在终端里用自带的 `board` 命令行，把解压出来的目录加进 `PATH` 环境变量。
- 没有签名和公证：macOS 只做了 ad-hoc 签名，第一次打开会被拦下，到「系统设置 → 隐私与安全性」里点「仍要打开」（或 `xattr -dr com.apple.quarantine /Applications/Board.app`）；Windows 上 SmartScreen 提示时点「更多信息 → 仍要运行」。
- 桌面端的「设置 → 桌面端更新」读 `latest.json`，装新版本（见下文「桌面端检查更新」）。
- 本机自己打包：`npm run desktop:build`（本机架构）；macOS 通用包：`npx tauri build --target universal-apple-darwin --bundles app,dmg`（要先 `rustup target add aarch64-apple-darwin x86_64-apple-darwin`）；Windows 便携版照 deploy.yml 的「打包 Windows 便携版」一步：把 `target/<triple>/release/` 里的 `board-desktop.exe`、`board.exe`（和有才有的 `WebView2Loader.dll`、`resources/`）压成 zip。

### 自己部署的桌面端

桌面端（`src-tauri/`）里没有写死的服务端地址。装官方安装包的电脑第一次打开时，按下面的顺序决定连哪个服务端：

| 顺序 | 在哪 | 说明 |
|---|---|---|
| 1 | 环境变量 `BOARD_URL` | 开发时用（`http://localhost:1420` 指向界面开发服务器，或本机服务端）；从图标启动时一般看不到 shell 里的变量 |
| 2 | `~/.board/config.toml` 的 `url` | 和 `board login` 写的是同一项，桌面端「连接服务端」页和设置 → 账号的「切换服务端」也写这里 |
| 3 | 构建时注入的 `BOARD_DEFAULT_URL` | 官方安装包在发版工作流里用 GitHub 仓库变量 `DEPLOY_URL` 注入；没有注入时为空 |
| — | 都没有 | 显示「连接服务端」页：填地址 → 请求 `<地址>/api/health` 确认是 Board 服务端 → 写进配置文件 → 按它打开 |

自己部署时，两种做法都行：

1. **不改安装包**：装官方安装包，第一次打开在「连接服务端」页填自己的地址（例如 `https://board.example.com`）；或者先用 `board login` 写一次配置文件（社区好之后，设置 → 账号里的「切换服务端」也能换）。地址只写在本机的 `~/.board/config.toml` 里。
2. **打包里带上自己的地址**（自己发一版桌面端时）：构建前设 `BOARD_DEFAULT_URL`，源码构建时就是环境变量，GitHub Actions 发版时在仓库 Settings → Secrets and variables → Actions → Variables 里设 `DEPLOY_URL`：

   ```bash
   BOARD_DEFAULT_URL=https://board.example.com npm run desktop:build
   ```

   构建出来的安装包第一次打开就是这个地址（配置文件里已经写了 `url`、或设了 `BOARD_URL` 时以它们为准），不会出现「连接服务端」页。

- 「连接服务端」页和设置 → 账号的「切换服务端」都先校验 `/api/health`（`{"ok":true,…}`），不是 Board 服务端的地址当场报错，配置文件不写。地址写进 `url` 只是服务端地址，令牌还是 `board login` 那一项，两者互不覆盖。
- 本机命令（在默认浏览器里登录、一键写入配置、让这台电脑接任务、检查更新等）只对当前连着的这个地址的页面开放：换了地址，桌面端关掉窗口按新地址重新打开，旧地址的页面调不到本机命令。本机在跑 `board host` 时，切换会先确认，之后按新地址重新启动它。
- 桌面端打开的页面必须是这个服务端的页面：反向代理到别的域名时，用最终访问的地址（浏览器地址栏里的那个）填，才能拿到本机命令。
- 用局域网地址（例如 `http://192.168.1.5:7420`）跑桌面端时，登录回调 `http://127.0.0.1:<端口>/callback` 是桌面端在本机监听的，不受影响。

### 桌面端检查更新

设置 → 桌面端更新：显示当前版本、「检查更新」；启动后隔几秒自动检查一次（默认开，可以关）。发现新版本时用应用内的确认框问，确认后下载、校验签名、安装，装完提示重启（Windows 上安装器接管：关闭 Board、装完自己拉起新版）。代码在 `src-tauri/src/update.rs`（外壳命令）、`web/src/lib/update.ts`（版本比较、状态）、`web/src/components/update-card.tsx`（设置页）。

| 东西 | 在哪 |
|---|---|
| 客户端读取的清单 | `https://github.com/buglan/board/releases/latest/download/latest.json`（`src-tauri/tauri.conf.json` 的 `plugins.updater.endpoints`），总是指向最新一个 Release |
| 清单内容 | 版本号、那一版 CHANGELOG、每个平台的更新包地址和签名（`scripts/updater-manifest.mjs` 生成，release 时作为附件上传） |
| 更新包 | macOS：`Board_<版本>_universal.app.tar.gz`（通用包，清单里两个架构指向同一个附件）；Windows：NSIS 安装包 `.exe`（就是安装包本身）。`tauri build` 打了 `createUpdaterArtifacts`，签名放在同名的 `.sig`，内容写进 `latest.json` |
| 公钥 | `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，可以进仓库；客户端只信任它 |
| 私钥 | 只在 GitHub secret 里，**仓库里没有**；丢了只能换一对新密钥并发一版（旧版本客户端只认旧公钥，收不到更新） |

一次性配置（发布带更新功能的版本之前）：

1. 生成密钥对（私钥不要提交、不要贴到任何地方）：

   ```sh
   npx @tauri-apps/cli signer generate -w board-updater.key      # 会问密码，可以不设
   ```

2. GitHub 仓库 Settings → Secrets and variables → Actions → Secrets：

   | 名称 | 值 |
   |---|---|
   | `TAURI_SIGNING_PRIVATE_KEY` | `board-updater.key` 文件的内容 |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 生成时设的密码；没设就留空 |

3. `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey` 换成 `board-updater.key.pub` 的内容。

没有配私钥时，desktop 任务在编译之前的「核对更新签名私钥」一步就失败、不建 Release（服务端照常部署）：不会发出一份装不上的更新。私钥配错（例如只贴了一部分、密码不对）时 `tauri build` 报「failed to decode secret key」，同样不建 Release。

发版流程不用另外操作：`npm run release -- x.y.z --push` 之后，desktop 任务带着签名打包、收齐更新包、按 tag 生成清单；release 任务把两个平台的清单合并成 `latest.json`（要求 macOS 两个架构和 Windows 都在），和安装包一起附在 Release 上。版本号在 tag、`tauri.conf.json`、`CHANGELOG.md` 和 `latest.json` 里一致（前两处不一致时构建就失败；清单的版本号和说明直接取 tag 和 CHANGELOG）。

校验：

- `node scripts/updater-manifest.mjs --dry-run`：不发版时验证清单结构（版本号、签名、https 地址、平台键、缺平台要被拦下）。推 `main` 的测试任务也会跑。
- 发版后打开 GitHub Release，确认 `latest.json` 在附件里；`curl -sL https://github.com/buglan/board/releases/latest/download/latest.json` 能看到三个平台。
- 装上一版桌面端，等自动检查，或在设置页点「检查更新」：应提示有新版本，下载安装后重启，设置页的「当前版本」变成新版本。检查失败不影响当前版本，可以重试。

本机验证（不用真发版）：用一个本地更新服务代替 GitHub Release，两个版本都用一次性的 dev 密钥打包。

1. 生成 dev 密钥（一次性，不设密码），换一个和现在不同的版本号（例如 `tauri.conf.json` 临时改成 `0.1.13`），带签名打包：

   ```bash
   npx @tauri-apps/cli signer generate -w /tmp/dev-updater.key -p ''        # 没设密码
   # src-tauri/tauri.conf.json 的 plugins.updater.pubkey 临时换成 /tmp/dev-updater.key.pub 的内容
   TAURI_SIGNING_PRIVATE_KEY="$(cat /tmp/dev-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npm run desktop:build
   ```

   签名产物在安装包旁边：Windows 是 `target/release/bundle/nsis/` 下的 NSIS 安装包 `.exe` 和 `.sig`，macOS 是 `target/release/bundle/macos/Board.app.tar.gz` 和 `.sig`（要打 `app`：只打 `dmg` 时不出更新包）。

2. 把它们按平台键收进一个目录（Windows：`windows-x86_64-nsis`；macOS：`darwin-aarch64-app` 或 `darwin-x86_64-app`），再用脚本生成清单（`--partial`：只有本机这一个平台）：

   ```bash
   mkdir -p /tmp/updater/windows-x86_64-nsis
   cp target/release/bundle/nsis/*-setup.exe* /tmp/updater/windows-x86_64-nsis/
   BOARD_VERSION=0.1.13 node scripts/updater-manifest.mjs --dir /tmp/updater --version 0.1.13 \
     --base http://127.0.0.1:8000/windows-x86_64-nsis --out /tmp/updater/latest.json --partial
   cd /tmp/updater && python -m http.server 8000
   ```

3. 把 `src-tauri/tauri.conf.json` 的 `plugins.updater.endpoints` **临时**改成 `["http://127.0.0.1:8000/latest.json"]`（调试构建允许 http），回到上一版（例如 `0.1.12`）用同一个 dev 密钥再打一次包运行，在设置 → 桌面端更新点「检查更新」：应提示 0.1.13，下载、校验签名、安装后提示重启，重启后「当前版本」是 0.1.13。

4. 验证完把版本号、`pubkey`、`endpoints` 都改回去：`npm test` 会核对 `endpoints` 是线上地址，`npm run release` 会核对版本号和 tag。

检查失败、签名不对、清单写坏（`node scripts/updater-manifest.mjs --dry-run` 会拦下）时都不会动当前版本，可以重试。

### 一次性配置

1. 生成部署专用的 SSH 密钥（只给 Actions 用），公钥加到服务器上部署用户的 `authorized_keys`：

   ```bash
   ssh-keygen -t ed25519 -N '' -C board-deploy -f board-deploy
   ssh-copy-id -i board-deploy.pub <用户>@<服务器>
   ssh-keyscan -t ed25519 <服务器>      # 输出填到 DEPLOY_KNOWN_HOSTS
   ```

2. GitHub 仓库 Settings → Secrets and variables → Actions：

   | 类型 | 名称 | 值 |
   |---|---|---|
   | Secret | `DEPLOY_SSH_KEY` | 上面的私钥 `board-deploy` 的内容 |
   | Secret | `DEPLOY_KNOWN_HOSTS` | `ssh-keyscan` 的输出（SSH 严格校验主机密钥） |
   | Secret | `DEPLOY_HOST` | `用户@服务器`（存成 Secret，Actions 日志里显示为 `***`） |
   | Variable | `DEPLOY_DIR` | 服务器上的仓库目录 |
   | Variable | `DEPLOY_URL` | `https://board.example.com` |

3. 服务器 `.env` 里不用加 `BOARD_IMAGE`：第一次自动部署时会加上。

### 看部署结果

- GitHub 仓库 Actions → deploy：每一步的输出；失败时 deploy 任务最后打印了容器最近 50 行日志
- 线上版本：`curl -s https://board.example.com/api/health` 的 `version`、`commit`
- 服务器上正在运行的镜像：`.env` 的 `BOARD_IMAGE`

### 回滚

Actions → deploy → Run workflow（分支选 `main`），`commit` 填要回到的 7 位提交号（某个 tag 的提交号：`git rev-parse --short=7 v0.3.0`）：跳过构建，直接部署 GHCR 上已有的 `board-server:<提交号>` 镜像，部署后只核对提交号。GHCR 只保留最新 5 个版本，更早的提交号会报「GHCR 上没有镜像」，线上不变。

## 8. 日志

```bash
docker compose logs -f board
```

每个请求一行：方法、路径（不含查询参数）、状态码、耗时、是谁（GitHub 账号，或 `claude/opus-5.5（令牌#3）`）。令牌和 Cookie 不写进日志。成功的心跳（`/api/agents/beat`、`/api/hosts/<机器>/beat`）和日志上传（`POST /api/dispatches/<ID>/log`）默认只记 DEBUG，不会刷掉有用的行；排查时用 `RUST_LOG=debug` 打开。日志文件按 10 MB 轮转，保留 5 个（`docker-compose.yml` 的 `logging`）。

### 线上出问题怎么查

1. 先看最近的日志：`docker compose logs --tail 200 board`（加 `-f` 持续跟踪；按时间查用 `--since 1h`）。5xx 响应会额外有一行 `ERROR`，带具体原因（数据库错误、文件错误等），和请求的 span 一起显示方法、路径、调用者。
2. 需要更细的日志（例如成功的心跳、日志上传）：在 `.env` 里设 `RUST_LOG=debug`，再 `docker compose up -d board`。排查完改回 `RUST_LOG=info` 重启，避免日志被心跳刷掉。
3. 确认线上版本：`curl -s https://board.example.com/api/health`，核对返回的 `version` 和 `commit` 与部署记录、tag 的提交号一致。
4. 从客户端检查：`board doctor` 看服务端地址、令牌和名字；带白板再跑一次 `board doctor --as <名字> --board <项目>/<白板>`。
5. 日志按 10 MB 轮转、保留 5 个；旧记录被挤掉时用 `--since` 缩小时间范围，或先看宿主机上的备份和监控。

## 本机模式

不部署、只在自己电脑上用时：

```bash
cargo run --release -p board-server -- --local
```

只监听 `127.0.0.1`，不需要登录，数据在 `~/.board/board.db`（`--db <路径>` 或 `BOARD_DB` 可指定）。

CLI 同样连这个服务端：浏览器打开 `http://127.0.0.1:7420`，在「设置 → Agent 接入」新建访问令牌，然后

```bash
board login --url http://127.0.0.1:7420 --token <访问令牌>
```

环境变量和 `~/.board/config.toml` 里都没有服务端地址或令牌时，CLI 直接报错（`NOT_LOGGED_IN`，提示运行 `board login`），不会去读写数据库文件。
