# Board

看板式白板：人在界面上操作，agent 通过命令行（`board`）操作，两边读写同一块白板。需求见 [docs/PRD.md](docs/PRD.md)（v0.3），交互原型见 [prototype/](prototype/)。

## 组成

```
crates/board-core     数据与规则：SQLite 存储、规则引擎（PRD §8）、示例数据
crates/board-cli      board 命令行（agent 用，PRD §9）
crates/board-server   网页端：HTTP API + SSE 实时推送，并托管界面
src-tauri             桌面端（Tauri 2）：内嵌同一个服务，加载同一套界面
web                   界面（React + Vite + TypeScript），网页端与桌面端共用
```

- **同一份数据**：界面、CLI、多个窗口读写同一个 SQLite 文件，默认 `~/.board/board.db`，用环境变量 `BOARD_DB` 指定别的位置。
- **领取互斥**：所有写操作在 `BEGIN IMMEDIATE` 事务里串行执行，多个 agent 同时领取同一张卡片只有一个成功。
- **超时释放**：每条命令执行前先释放这块白板上到期的领取；服务端另外每秒扫描一次，没有命令时界面上也能看到释放。
- **实时同步**：CLI 直接写数据库，服务端每 250ms 检查一次版本号（rev），变化后通过 SSE 推送给已打开的界面；界面重新取快照，并高亮被改动的卡片、标出是谁做的。
- **桌面端**：启动时在 `127.0.0.1` 的随机端口上运行内嵌服务，只允许桌面窗口跨域访问；顶栏即标题栏，「文件 → 新建窗口」（⌘⇧N）可以开多个窗口看不同的白板。

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

## 网页端

```bash
npm run build:web
```

```bash
cargo run --release -p board-server
```

打开 <http://127.0.0.1:7420>。常用参数：

| 参数 | 作用 |
|---|---|
| `--port 7420` | 端口 |
| `--host 0.0.0.0` | 监听所有网卡，手机浏览器用电脑的局域网地址访问（没有账号与登录，只在可信网络里这样做） |
| `--db <路径>` | 数据库文件，默认 `$BOARD_DB` 或 `~/.board/board.db` |
| `--demo` | 数据库为空时写入示例项目（3 个项目、5 块白板） |

界面在编译 `board-server` 时打包进可执行文件；改了界面要先 `npm run build:web` 再编译。

## 桌面端

```bash
npm run desktop:dev
```

```bash
npm run desktop:build
```

安装包在 `target/release/bundle/`。桌面端与 CLI 共用 `~/.board/board.db`。

## CLI

```bash
cargo install --path crates/board-cli
```

```bash
export BOARD_AS=alpha BOARD=default
board claim --json
```

```bash
board help
```

首次运行时会建立默认项目 `default`（白板 `main`）。项目、白板、标签在界面上新建。

## 开发

界面热更新：先启动服务（示例数据放在仓库内的 `.dev/`），再启动 Vite（1420 端口，`/api` 代理到 7420）。

```bash
cargo run -p board-server -- --demo --db .dev/board.db
```

```bash
npm run dev:web
```

测试（规则与 CLI 的验收标准，以及界面的类型检查）：

```bash
npm test
```

## 验收标准对应

| PRD §13 | 在哪里验证 |
|---|---|
| 1–9、10（rev）、13–15、17、19–22、25 | `crates/board-core/src/tests.rs` |
| 2（并发领取） | `ac2_concurrent_claim_only_one_wins`：4 个进程同时 `claim 1` |
| 11、12、18、23、24 | `crates/board-cli/tests/cli.rs`（真实执行 `board`，检查退出码与 JSON） |
| 8、10、16、20、21、26 | 界面：拖拽被拒绝弹回、CLI 新建 2 秒内出现、多窗口各看各的白板、复制为提示词 |
