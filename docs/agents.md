# 接入 agent

`board host` 在派到这台机器的卡片上启动本机的 agent（PRD §9.1）。这份文档写清楚：一个 agent 要满足什么才能接进来、已经接入了哪些、怎么按批次接入新的。

## board host 怎么启动 agent

派发开始时，`board host` 已经以 `<agent 标识>/<机器名>-<序号>`（例如 `dsh/mbp-3`）领到了卡片，然后：

1. 在这块白板对应的目录（`--dir <项目>/<白板>=<路径>`）里、在这个 agent 的**启动环境**里执行它的**命令模板**。模板里的 `{prompt}` 换成一行提示词，作为**一个参数**传入，不拼进 shell 命令文本：

   ```
   /board-skills 开始执行 <项目>项目之中的<白板>白板中的#<ID>任务
   ```

   启动环境是「在哪种 shell 里启动」，按机器、按 agent 在机器详情里选（PRD §9.1）：

   | 启动环境 | 系统 | 怎么启动 |
   |---|---|---|
   | 直接启动（`direct`） | 所有 | 直接执行命令，不经过 shell |
   | 登录 shell（`login`） | macOS、Linux | `$SHELL -l -i -c 'exec "$0" "$@"' <命令> <参数>…`：读 `~/.zprofile`、`~/.zshrc` 这些启动脚本，写在里面的 `PATH` 也生效 |
   | cmd | Windows | `cmd.exe /d /s /c "<命令> <参数>…"` |
   | Windows PowerShell、PowerShell 7（`powershell`、`pwsh`） | Windows | `-NoLogo -NonInteractive -ExecutionPolicy Bypass -Command "& '<命令>' '<参数>' …; exit $LASTEXITCODE"` |
   | Git Bash（`git-bash`） | Windows | `bash.exe -l -c 'exec "$0" "$@"' <命令> <参数>…` |
   | WSL（`wsl:<发行版>`） | Windows | `wsl.exe -d <发行版> --cd <目录> --exec setsid -w bash -l -i -c 'exec "$0" "$@"' <命令> <参数>…`，目录换成 `/mnt/…` |

   没选时按这个 agent「支持的启动环境」的顺序（下表），取本机有、并且在里面找得到命令的第一个。macOS、Linux 上都是登录 shell → 直接启动。

2. 环境变量带上 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_AS`（上面的名字）、`BOARD_HOST`（机器名）、`BOARD`（`<项目>/<白板>`），以及 `BOARD_CONFIG`：这次派发自己的配置文件 `~/.board/host/run/<机器名>/<派发 ID>.toml`（服务端地址和令牌，只有自己能读，派发结束时删除）。agent 里的 `board` 命令直接用它们，不用再 `--as`、`--board`。

   还有 `BOARD_EXE`：`board host` 自己的路径。`board-skills` 的 `scripts/board` 看到它就用它，不用 skill 里打包的 `board`，两边版本一致。

   经过 shell 启动时，shell 的启动脚本里设了 `BOARD_URL`、`BOARD_TOKEN` 也不影响：包装脚本在 `exec` 之前用 `board host` 的值（`BOARD_HOST_URL`、`BOARD_HOST_TOKEN`）再设一遍。WSL 里的变量经 `WSLENV` 传进去：`BOARD_CONFIG` 换成 WSL 路径，不传 `BOARD_EXE`（Windows 的 `board.exe` 在 WSL 里用不了，用 skill 自带的 Linux 版 `board`）。

   为什么还要 `BOARD_CONFIG`：有的 agent 不把名字像密钥的环境变量传给它执行的命令。dsh 会去掉名字里带 `KEY`、`PASSWORD`、`SECRET`、`TOKEN` 的变量，`BOARD_TOKEN` 到不了 `board`，`board` 就会拿 `~/.board/config.toml` 里别的令牌去连，得到 `INVALID_TOKEN`。`board` 读 `BOARD_CONFIG` 指向的文件代替 `~/.board/config.toml`，这个变量名不像密钥，能传过去。
3. 标准输入是空的，标准输出和标准错误写到 `~/.board/host/logs/<派发 ID>.log`，每 5 秒上传，界面「日志」里看。
4. 名字的心跳由 `board host` 发。`board-skills` 看到 `BOARD_HOST` 且卡片已由 `BOARD_AS` 持有，就不再 check、不再领取、不管心跳，只干活、提交推送、交审或交给人。
5. agent 退出后看卡片在哪：在交审状态是「已交审」；已经不归它、在其他状态是「做不下去」；还归它就记「异常退出」，卡片移到需要人处理的状态，评论里带退出码和日志最后 20 行。
6. 人点「停止」或 `board host` 退出时，结束包括启动环境外壳在内的整个进程树：先给整个进程组发 SIGTERM，10 秒后 SIGKILL（Windows 用 `taskkill /T /F`；WSL 里 agent 的进程号写在 `BOARD_PIDFILE`，在 WSL 中给它的进程组发 SIGTERM / SIGKILL）。agent 的主进程自己退出时，同一进程组里剩下的进程也一起结束（Windows 除外）；agent 自己再 `setsid` 出去的进程不在组里，管不到。`board host` 被 `kill -9`、崩溃后同名重启，会先结束上一次留下还在跑的 agent（按 `~/.board/host/run/<机器名>/<派发 ID>.proc` 里的进程号和启动时间）。

## 一个 agent 要满足什么

接入前逐条验证（「本机验证」一节有命令）：

| # | 要求 | 不满足会怎样 |
|---|---|---|
| 1 | **非交互模式**：命令行带一个任务，做完就退出，不等输入 | 一直挂着，占着并发位 |
| 2 | **不问权限**：能读写仓库、执行命令、访问网络（`git push`、`board`），没有审批弹窗；用这个 agent 自己的「跳过审批 / 关沙箱」参数 | 卡在审批上，或者命令被拒，做不下去 |
| 3 | **找得到 `board-skills`**：从某个 skill 目录加载它，提示词以 `/board-skills` 开头时照着它做 | 不认识这句话，什么也不做就退出 → 异常退出（退出码常常是 0） |
| 4 | **在这台机器上已经登录 / 配好模型**（API key、OAuth、provider） | 一启动就报错退出 |
| 5 | 它执行的命令能拿到 `BOARD_URL`、`BOARD_AS`、`BOARD`、`BOARD_CONFIG`、`BOARD_EXE` 这几个环境变量（`BOARD_TOKEN` 被去掉没关系） | `board` 连错服务端、找不到名字或白板 |
| 6 | 进度写到标准输出或标准错误 | 「日志」里看不到东西（不影响结果） |
| 7 | 收到 SIGTERM 会退出 | 停止要等 10 秒后强制结束 |
| 8 | `<命令> --version` 第一行是版本号 | 界面上只显示「找到了」（不影响使用） |
| 9 | **在它支持的启动环境里能跑**：在每个支持的环境里 `command -v <命令>`（cmd、PowerShell 里 `where.exe <命令>`）找得到，非交互命令在里面能做完一张卡。Windows 上不支持的环境（例如 pi 不支持 cmd）不写进模板 | 这个环境里显示「没找到」，或派过去异常退出 |

agent 标识：小写字母、数字、`.`、`-`，以字母开头，最长 16 个字符（名字 `<标识>/<机器名>-<序号>` 不超过 48 个字符）。

## 已经接入的（内置模板）

内置模板写在 `crates/board-cli/src/host.rs` 的 `BUILTIN` 里。**顺序就是默认 agent 的顺序**：本机找到的第一个是这台机器的默认 agent（定时领取用它，手动派发时预先选中它），机器详情里可以改。「支持的启动环境」的顺序就是默认选择的顺序；macOS、Linux 上都是登录 shell → 直接启动。「`PATH` 里没有时再看」的位置只在直接启动、登录 shell 里用。

| 顺序 | 标识 | 名称 | 命令模板 | `PATH` 里没有时再看 | 读 skill 的目录 | 支持的启动环境（Windows） | 前提 |
|---|---|---|---|---|---|---|---|
| 1 | `dsh` | DeepSeek Harness | `dsh --profile headless --patch {patch} --permission-mode danger-full-access {prompt}` | `/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh`（`~/Applications` 下同样） | `~/.agents/skills` | `pwsh` → `powershell` → `cmd` | 在 DeepSeek Harness 里配好模型 |
| 2 | `claude` | Claude Code | `claude -p {prompt} --dangerously-skip-permissions` | `~/.local/bin/claude`、`~/.claude/local/claude` | `~/.claude/skills` | `pwsh` → `powershell` → `git-bash` → `wsl:*` → `cmd` | 在终端里运行一次 `claude` 并 `/login` |
| 3 | `pi` | pi | `pi -p {prompt}` | — | `~/.agents/skills`（也读 `~/.pi/agent/skills`） | `git-bash` → `wsl:*` → `pwsh` → `powershell`（不支持 `cmd`） | `pi` 里配好 provider 和默认模型 |
| 4 | `codex` | Codex | `codex exec --dangerously-bypass-approvals-and-sandbox {prompt}` | — | `$CODEX_HOME/skills`（默认 `~/.codex/skills`） | `pwsh` → `powershell` → `git-bash` → `wsl:*` → `cmd` | 运行一次 `codex login` |

配置文件里新加的 agent 在 Windows 上支持 `pwsh` → `powershell` → `git-bash` → `wsl:*` → `cmd` → `direct`；同标识覆盖内置模板时，支持的启动环境不变。

各自的说明：

- **dsh**：`headless` 档案做完一个任务就退出，最终回答写标准输出，推理过程和诊断写标准错误（日志里都能看到）。它执行命令时去掉名字像密钥的环境变量（见上文 `BOARD_CONFIG`）。`--permission-mode danger-full-access` 关掉沙箱、不审批；默认的 `workspace-write` 只能写当前目录，`board-skills` 要在仓库旁边建 worktree、`git push`，不够用。dsh 的 headless profile 默认还挂着 goal 工具和 goal-round-driver：模型一旦 `create_goal`，任务结束后会自动续一轮，进程退出时把被终止的续轮当成最后一轮（aborted/disposed），退出码变成 1。`board host` 每次 dsh 派发会写一份禁用 `goal-round-driver`、`tool-goal`、`command-goal` 的 profile 补丁，通过 `--patch {patch}` 传入（见 `crates/board-cli/src/dsh-headless.patch.yml`）。启动时的「skipping profile bundle "dsh-ops"」是 dsh 自己的提示，不影响。DeepSeek Harness 的「设置」里可以把命令行装到 `/usr/local/bin/dsh`；没装也能用，`board host` 会用 app 里自带的那个。
- **Claude Code**：`-p` 只在结束时打印结果，跑的过程中日志是空的。登录信息在系统钥匙串里：从别的 agent 会话、沙箱里启动的 `board host` 可能读不到，会报「Not logged in」，从自己的终端或桌面端运行就好。
- **pi**：pi 本身不做权限审批。`/board-skills …` 和 `/skill:board-skills …` 两种写法都会加载 skill。Windows 上 pi 用 Git Bash 执行命令，默认的启动环境就是 Git Bash；在 PowerShell 里启动时，`board host` 会从 `PATH` 里的 `git` 找到 Git 安装目录，把它旁边的 `bin` 放到 pi 的 `PATH` 前面，所以 Git 不在 `C:\Program Files\Git` 也能用；找不到 Git Bash 时在 `~/.pi/agent/settings.json` 里设置 `shellPath`。pi 不支持 cmd。
- **Codex**：`codex exec` 做完一个任务就退出；`--dangerously-bypass-approvals-and-sandbox` 不审批、不开沙箱（`board-skills` 要 `git push`、访问服务端）。skill 读 `$CODEX_HOME/skills`，默认 `~/.codex/skills`。

## 装 `board-skills`

agent 在哪个目录找 skill，就把 `board-skills` 放到哪里（上表「读 skill 的目录」）。除了用户级目录，dsh、Claude Code、pi 也会读工作目录（仓库根目录）里的 `.agents/skills`，所以把 skill 放在仓库的 `.agents/skills/board-skills` 也可以，不用每台机器再装一遍。推荐解压发版附件 `board-skills.zip`（带各平台的 `board`）；在 Board 仓库里开发时用链接：

```sh
mkdir -p ~/.agents/skills ~/.claude/skills
ln -sfn "<Board 仓库>/.agents/skills/board-skills" ~/.agents/skills/board-skills   # dsh、pi 等
ln -sfn "<Board 仓库>/.agents/skills/board-skills" ~/.claude/skills/board-skills   # Claude Code
```

`board host` 启动时在每个 agent 用的启动环境里检查它的 skill 目录（WSL 里看 WSL 用户目录下的，例如 `/home/<用户>/.agents/skills`），看不到 `board-skills` 会提醒一行。在 WSL 里跑的 agent 要在 WSL 里另装一份（或放在仓库的 `.agents/skills` 里）。链接指向的目录不存在（例如仓库挪过位置）和没装一样，`ls ~/.agents/skills/board-skills/SKILL.md` 能看到文件才算装好。

## 按批次接入新的 agent

每一批几个 agent，每个都走完下面五步。前四步只靠配置文件和本机测试，不改代码；第五步把这一批验证过的写成内置模板，一起提交、发版。

### 1. 调研：每个 agent 填一张表

| 项 | 怎么找 |
|---|---|
| 标识、名称 | 标识用它的命令名（小写），名称用它自己的产品名 |
| 非交互命令 | `--help` 里找「print / exec / headless / run」之类：带一个任务、做完退出 |
| 跳过审批 | `--help` 里找「approval / permission / sandbox / yolo / dangerously」之类 |
| skill 目录 | 它的文档里「skills」一节；常见是 `~/.agents/skills` 或它自己的 `~/.<名字>/skills` |
| 登录 / 模型 | 它的文档；记下第一次用要做什么 |
| 安装位置 | `command -v <命令>`；装成 app 的再看 app 里有没有自带的命令行（macOS 在 `*.app/Contents/` 下找） |
| 版本 | `<命令> --version` |
| 日志特点 | 过程是写标准输出、标准错误，还是只在最后打印 |

### 2. 本机验证：不碰白板

在任意目录里，用第 1 步的非交互命令问它一句，确认它找得到 `board-skills`、不问权限、问完就退出：

```sh
<非交互命令> "/board-skills 不要执行任何命令。只回答两行：第一行是你加载的 skill 的 name，第二行是它 SKILL.md 里第一个二级标题。"
```

应该回答 `board-skills` 和 `参数`，然后退出。回答「没有这个 skill」：先按上一节装好，再问一次。

### 3. 用配置文件接上

在 `~/.board/config.toml` 里加一节（`{prompt}` 单独作为一个元素）：

```toml
[agents.<标识>]
name = "<名称>"
cmd = ["<命令>", "<非交互参数>", "<跳过审批参数>", "{prompt}"]
```

重启 `board host`，启动输出和界面「机器」里应该出现它，带版本号；机器详情「本机 agent」的启动环境下拉里能看到它在每个启动环境里找没找到。同标识的 `[agents.*]` 也用来临时改内置模板（例如换参数、换路径）。

### 4. 端到端验证：用本机服务端，不碰线上

```sh
# 一个带远端的空仓库：AGENTS.md 写明集成分支 main 和推送前检查
git init --bare -b main /tmp/e2e/origin.git && git clone /tmp/e2e/origin.git /tmp/e2e/repo
# …在 repo 里写 AGENTS.md、一个检查脚本，提交并推到 origin…

board-server --local --port 7478 --db /tmp/e2e/board.db
# 新建令牌：curl -X POST http://127.0.0.1:7478/api/tokens -H 'content-type: application/json' -H 'x-board-client: e2e' -d '{"name":"e2e"}'
export BOARD_URL=http://127.0.0.1:7478 BOARD_TOKEN=<令牌>
board host --name e2e --dir default/main=/tmp/e2e/repo
# 另一个终端：建一张小卡（例如「新建 notes/<标识>.txt，内容是一行 hello」），派给它
BOARD=default/main board dispatch 1 --host e2e --agent <标识>
```

通过的标准：派发结果「已交审」；`origin/main` 上多了一个提交，最后一行是 `卡片：default/main#1`；卡片动态里有 agent 的交审评论（`提交：`、`检查：` 两行）；`~/.board/host/logs/<派发 ID>.log` 里能看到它的过程。再试一次「停止」：10 秒内进程结束，卡片移到「需人工校验」。

**每个支持的启动环境各跑一次**：在机器详情「本机 agent」里把它的启动环境依次换成每个支持的环境（Windows 上的 PowerShell、Git Bash、WSL …；macOS、Linux 上的登录 shell 和直接启动），各派一张卡，结果都要「已交审」，派发记录写的是那个环境；在 WSL 里再试一次「停止」，WSL 里的 agent 进程也要在 10 秒内结束（`wsl.exe -d <发行版> -- ps -ef` 看）。

### 5. 写成内置模板

这一批都通过第 4 步后：

- `crates/board-cli/src/host.rs` 的 `BUILTIN`：加上 `id`、`name`、`cmd`、`also`（常见安装位置）、`skills`（读用户级 skill 的目录）、`win`（Windows 上支持的启动环境，按默认选择的顺序；第 4 步没跑通的环境不写）。放在哪一位就是默认顺序里的哪一位。
- 同一个文件里的测试 `builtin_order_and_overrides`：改期望的顺序。
- 界面上的图标：用它自己的图标（应用图标或官网 logo），放进 `web/src/assets/agents/<标识>.png`（裁掉透明边，64×64）或 `.svg`，在 `web/src/lib/hosts.ts` 的 `MARKS` 里引用，并配一个主色（并发位顶边用）；白底的图标标 `light: true`。没有图标时显示标识的前两个字母。
- `docs/PRD.md` §9.1 的内置模板表，和本文「已经接入的」表、下面的批次记录。
- 跑完 `npm test`，提交 `feat(cli): board host 内置 <agent> 模板`；需要在别的机器上试时请人发版。

## 批次记录

| 批次 | agent | 结果 |
|---|---|---|
| 1 | Claude Code（`claude`） | v0.1.2 内置 |
| 2 | DeepSeek Harness（`dsh`，设为默认）、pi（`pi`） | 内置。2026-11-09 本机端到端：dsh 0.2.2-rc.1、pi 1.5.0 各做一张卡，都是「已交审」，提交和交审评论齐全；dsh 的「停止」正常。接入时发现 dsh 不传带 TOKEN 的环境变量，加了 `BOARD_CONFIG`、`BOARD_EXE` |
| 3 | Codex（`codex`）；所有 agent 加上启动环境 | 内置。2026-11-09 macOS 本机：dsh 在登录 shell 里端到端「已交审」、「停止」正常；只写在 `~/.zshrc` 里的 `PATH` 找得到，启动脚本改了 `BOARD_URL` 也会被覆盖回来。Codex 只验证了登录 shell 里找得到命令，端到端还没跑（在 agent 的沙箱 shell 里 codex 起不来）。Windows 的 cmd、PowerShell、Git Bash、WSL 只做了交叉编译和单元测试，要在 Windows 机器上按第 4 步各跑一次 |

## 出了问题看哪里

| 现象 | 多半是 |
|---|---|
| 派发结果「异常退出（退出码 0）」，日志里 agent 说不认识 `/board-skills` | 没装 `board-skills`，或链接指向的目录不存在 |
| 日志第一行就是登录错误（例如「Not logged in」） | 这台机器上的 agent 还没登录 |
| pi 的日志说 bash 工具完全不可用，错误里有 `HCS_E_SERVICE_NOT_AVAILABLE` | Windows 上 pi 落到了 WSL 的 `bash.exe`：装 Git for Windows，或在 `~/.pi/agent/settings.json` 里设置 `shellPath`；`board host` 会把 PATH 里 git 旁边的 Git Bash 放到 pi 前面 |
| 「机器」里这个 agent 显示「没找到」 | 在它用的启动环境里找不到命令：看机器详情的启动环境下拉，换一个找得到的环境；或者在那个环境的启动脚本（`~/.zshrc`、`~/.bashrc`、PowerShell 的 `$PROFILE`）里把它加进 `PATH`；在配置文件里写绝对路径也行 |
| WSL 里显示「没找到」，可 WSL 的终端里能运行 | 装在 `/mnt/` 下的是 Windows 程序，不算；在 WSL 里另装一份 Linux 版 |
| 在 WSL 里派发，日志里 `board` 报找不到命令或连不上 | WSL 里没装 `board-skills`（它自带 Linux 版 `board`），或者 WSL 里的 skill 太旧 |
| 日志开头有 `bash: cannot set terminal process group`、`no job control` | 交互式 shell 没有终端时的提示，不影响 |
| 一直在跑、日志不动 | agent 在等审批或输入：模板里少了跳过审批的参数，或者它的非交互模式不是这个 |
| 日志里 `board` 报 `INVALID_TOKEN`，或者连到了别的服务端 | agent 没把 `BOARD_CONFIG` 传给它执行的命令（或者它的 shell 启动脚本里又设了 `BOARD_URL`、`BOARD_TOKEN`）：在它的设置里放行这几个变量。也可能是 skill 太旧：旧版 `scripts/board` 不认 `BOARD_EXE`，用的是 skill 里打包的旧 `board`，更新 skill |
| 桌面端「本机」打开后马上停了，输出里是「未知命令「host」」 | 桌面端旁边的 `board` 是旧版：重新编译它（`node scripts/build-sidecar.mjs --debug`，`npm run desktop:dev` 会先做这一步） |
| 卡片动态里 agent 又 check 了一个新名字、领取失败 | 用的 `board-skills` 是旧版（没有「由 board host 启动的会话」一节），更新 skill |
