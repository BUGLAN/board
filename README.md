# Board

看板式白板：人在界面上操作，agent 通过命令行（`board`）操作，两边读写同一块白板。需求见 [docs/PRD.md](docs/PRD.md)（v0.5），交互原型见 [prototype/](prototype/)，部署见 [docs/deploy.md](docs/deploy.md)。

服务端部署在线上（`https://board.buglan.com`）：人用 GitHub 登录（只允许一个指定账号），agent 用访问令牌。

## 组成

```
crates/board-core     数据与规则：SQLite 存储、规则引擎（PRD §8）、示例数据
crates/board-cli      board 命令行（agent 用，PRD §9）：连接服务端，或直接读写本机数据库
crates/board-server   服务端：登录与令牌、HTTP API + SSE 实时推送，并托管界面
src-tauri             桌面端（Tauri 2）：窗口打开线上的 Board，与网页端同源、同一套登录
web                   界面（React + Vite + TypeScript），网页端与桌面端共用
```

- **登录**（`crates/board-server/src/auth.rs`）：鉴权只在服务端入口这一层，规则引擎和原有错误码不变。
  - 人：GitHub OAuth 登录后得到会话 Cookie（`HttpOnly`、`SameSite=Lax`，线上带 `Secure`，30 天，使用中续期）；写请求还要带 `x-board-client` 请求头。
  - agent：`Authorization: Bearer bd_…` 访问令牌，只能读工作区 / 白板 / 卡片和执行卡片命令；名字取请求里的 `as`（`--as` / `BOARD_AS`）。
  - 会话和令牌在数据库里只存 SHA-256 哈希。所有 `/api/*` 都要登录或令牌，只有 `/api/health` 例外。
  - 本机模式（`--local`）：只监听回环地址、只接受 `127.0.0.1` / `localhost` 的 Host，不需要登录。
- **同一份数据**：线上只有服务端读写 SQLite；本机模式下界面和 CLI 读写同一个文件，默认 `~/.board/board.db`，用环境变量 `BOARD_DB` 指定别的位置。
- **领取互斥**：所有写操作在 `BEGIN IMMEDIATE` 事务里串行执行，多个 agent 同时领取同一张卡片只有一个成功。
- **超时释放**：每条命令执行前先释放这块白板上到期的领取；服务端另外每秒扫描一次，没有命令时界面上也能看到释放。
- **实时同步**：服务端每 250ms 检查一次版本号（rev，本机模式下 CLI 直接写库也会变），变化后通过 SSE 推送给已登录的界面；界面重新取快照，并高亮被改动的卡片、标出是谁做的。
- **桌面端**：窗口打开 `https://board.buglan.com`（环境变量 `BOARD_URL` 可覆盖），GitHub 登录在窗口里完成；顶栏即标题栏，「文件 → 新建窗口」（⌘⇧N）可以开多个窗口看不同的白板。

## 接入 agent

agent 通过 skill 接入：[`skills/board-skills/SKILL.md`](skills/board-skills/SKILL.md) 写明了怎么领取、干活、记进度、交审，以及规则和错误码。装好之后只需要发一句：

```text
/board-skills mini/main#2
```

卡片详情里「复制为提示词」（或按 `C`）复制的就是这一行；`/board-skills mini/main` 让 agent 领完整块白板的「待办」。

安装（以 `~/.agents/skills` 为例；Claude Code 用 `~/.claude/skills`）：

```bash
ln -s "$PWD/skills/board-skills" ~/.agents/skills/board-skills
```

不在仓库里时，在「设置 → Agent 接入」点「复制 SKILL.md」，保存为 `~/.agents/skills/board-skills/SKILL.md`。agent 还需要 `board` 命令行（见下文 CLI）。

## 准备

需要 Rust（1.85+）和 Node.js（20+）。

```bash
npm install
```

## 服务端

线上用 Docker Compose 部署，见 [docs/deploy.md](docs/deploy.md)。本机运行：

```bash
npm run build:web
```

```bash
cargo run --release -p board-server -- --local
```

打开 <http://127.0.0.1:7420>。常用参数（`board-server help` 查看全部）：

| 参数 | 作用 |
|---|---|
| `--local` | 本机模式：只监听本机地址，不需要登录 |
| `--port 7420` | 端口（`BOARD_PORT`） |
| `--host 127.0.0.1` | 监听地址（`BOARD_HOST`）；本机模式只能是回环地址 |
| `--db <路径>` | 数据库文件，默认 `$BOARD_DB` 或 `~/.board/board.db` |
| `--demo` | 数据库为空时写入示例项目（3 个项目、5 块白板） |

不加 `--local` 时需要 GitHub 登录的环境变量：`BOARD_PUBLIC_URL`、`BOARD_GITHUB_CLIENT_ID`、`BOARD_GITHUB_CLIENT_SECRET`、`BOARD_GITHUB_USER`（见 [.env.example](.env.example)）。

子命令：`board-server token new|ls|revoke`（管理访问令牌）、`board-server backup <文件>`（在线备份）、`board-server healthcheck`。

界面在编译 `board-server` 时打包进可执行文件；改了界面要先 `npm run build:web` 再编译。

## 桌面端

```bash
npm run desktop:build
```

安装包在 `target/release/bundle/`，打开的是 `https://board.buglan.com`。开发时连本机：

```bash
BOARD_URL=http://localhost:1420 npm run desktop:dev
```

## CLI

```bash
cargo install --path crates/board-cli
```

连接线上服务端（访问令牌在网页「设置 → Agent 接入」新建）：

```bash
export BOARD_URL=https://board.buglan.com BOARD_TOKEN=<访问令牌> BOARD_AS=alpha BOARD=default
```

```bash
board claim --json
```

不设置 `BOARD_URL` 时直接读写本机的 `~/.board/board.db`，首次运行时建立默认项目 `default`（白板 `main`）。项目、白板、标签在界面上新建。

```bash
board help
```

## 开发

界面热更新：先启动服务（本机模式，示例数据放在仓库内的 `.dev/`），再启动 Vite（1420 端口，`/api`、`/auth` 代理到 7420）。

```bash
cargo run -p board-server -- --local --demo --db .dev/board.db
```

```bash
npm run dev:web
```

在本机走一遍 GitHub 登录（不需要真的 GitHub OAuth App）：`scripts/github-mock.py` 模拟 GitHub 的授权页和接口，`.claude/launch.json` 里的 `github-mock` 与 `board-server-auth` 配置把两者连起来（打开 <http://127.0.0.1:7433>）。`BOARD_GITHUB_OAUTH_URL`、`BOARD_GITHUB_API_URL` 只在这种测试时设置。

测试（规则、CLI 两种连接方式、鉴权的验收标准，以及界面的类型检查）：

```bash
npm test
```

## 验收标准对应

| PRD §13 | 在哪里验证 |
|---|---|
| 1–9、10（rev）、13–15、17、19–22、25 | `crates/board-core/src/tests.rs` |
| 2（并发领取） | `ac2_concurrent_claim_only_one_wins`：4 个进程同时 `claim 1` |
| 11、12、18、23、24 | `crates/board-cli/tests/cli.rs`（真实执行 `board`，检查退出码与 JSON） |
| 1–4、7、8、11–13、35–38（服务端方式） | `crates/board-cli/tests/remote.rs`：起一个真实的服务端，CLI 用令牌连接；含 4 个进程同时领取、与本机方式输出一致 |
| 32–34、37 | `crates/board-server/tests/auth.rs`：每个接口未登录都被拒绝、模拟的 GitHub 登录与拒绝、退出、令牌权限与吊销、本机模式 |
| 8、10、16、20、21、26 | 界面：拖拽被拒绝弹回、CLI 新建 2 秒内出现、多窗口各看各的白板、复制为提示词 |
