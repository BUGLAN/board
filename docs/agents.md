# 接入 agent

`board host` 在派到这台机器的卡片上启动本机的 agent（PRD §9.1）。这份文档写清楚：一个 agent 要满足什么才能接进来、已经接入了哪些、怎么按批次接入新的。

board host 用 ACP（Agent Client Protocol）驱动所有 agent；当时为什么换、ACP 的规范和生态调研见 [acp.md](acp.md)。

## board host 怎么启动 agent

派发开始时，`board host` 已经以 `<机器名>/<模型名>-<推理强度>`（例如 `win/deepseek-v4.1-flash-high`）领到了卡片，然后：

1. 在这块白板所在项目的目录（这台设备的工作空间里的 `<工作空间>/<项目目录>`（项目设置里的项目目录，留空用项目标识），一个项目一个仓库，项目下的白板共用；`--dir <项目>=<路径>` 可以单独指定某个项目；派发开始时记下这个路径）里、在这个 agent 的**启动环境**里，用 **ACP**（Agent Client Protocol）启动它：`board host` 启动 `board acp`（board 自己的一个子命令，`crates/board-cli/src/acp.rs`），由它在启动环境里执行这个 agent 的 **ACP 入口**，完成 `initialize`、`session/new`（工作目录就是这个目录），再把一行提示词作为 `session/prompt` 发过去（提示词放在 `~/.board/host/run/<机器名>/<派发 ID>.prompt`，不放进命令行）。握手时要 **v2**（[草案](https://agentclientprotocol.com/rfds/v2/prompt.md)），按 ACP 的版本协商走：Claude Code、Codex 的适配器回 2，dsh 只会 v1、回 1（#461）：

   ```
   /board-skills 开始执行 <项目>项目之中的<白板>白板中的#<ID>任务
   ```

   `board acp` 的做法：

   - `initialize` 同时带 v2（`info`、`capabilities`）和 v1（`clientInfo`、`clientCapabilities`）的字段，只会 v1 的 agent 照样认得。Client 能力都不声明（agent 自己在 worktree 里读写、执行），`mcpServers` 传空（MCP 照旧由 Board 写进 agent 自己的配置，见下面「MCP 服务器」）。
   - 不问权限：agent 公布了不问权限的模式就切过去（Claude Code 的 `bypassPermissions`、Codex 的 `agent-full-access`）；`session/request_permission` 一律自动选「总是允许」（没有就「允许这一次」），Board 没有人工审批。dsh 用环境变量 `DSH_PERMISSION_MODE=danger-full-access` 关掉沙箱和审批。
   - agent 配置文件里有思考强度时（见下面「模型供应商」），会话配置里有 `reasoning_effort` 这一项就切过去（dsh 的 ACP 只能这样定强度）。
   - **这一轮的结束**：v1 的 `session/prompt` 回应跨整轮、带 `stopReason`。v2 的回应只表示提示词收下了，进展看 `state_update`：`running`、`requires_action` 是在干活，`idle` 带 `stopReason` 是这一轮做完了；[`idle` 不等于什么都没在跑](https://agentclientprotocol.com/rfds/v2/background-work.md)，后台还有命令、任务时 `idle` 带着 `background` 列表，后台跑完 agent 可以不等提示词自己再开一轮（又报 `running`）。
   - **这一轮做完以后**（#454、#461）：先等它稳定下来——最后一帧之后安静 2 秒、最多 10 秒（v1 收迟到的更新，v2 是规范要的宽限期）。做卡的派发（`board host` 带 `--extra-turns 3`；拆解、自检不带，做完就收尾）再查这次派发的名字（`BOARD_AS`）在这块白板（`BOARD`）上还持有没持有卡片（`GET /api/agents`）：
     - 不持有（交审了、交给人了）、查不到：收尾。
     - 还持有，v2 的 `background` 不空：不收也不续，日志里写一行「⋯ 等后台」，等 agent 跑完自己接着做；每 30 秒再查一次卡片。
     - 还持有，后台为空或 agent 不报（v1 的 dsh、不报后台的 Codex，或者 agent 停下来问人、做到一半就总结）：在同一个会话里再发一条提示词续一轮，写明还持有哪张、后台命令在这一轮里等它跑完、按 board-skills 做完或交给人，最多续 3 轮，每次在日志里写一行「↻ 第 n 次续轮」。续满了还持有，写一行「✗ 续了 3 轮…」后照常收尾，`board host` 按卡片还在它手里记成异常退出。
   - 收尾：v2 先发 `session/close`，再关掉 agent 的标准输入，最多等 5 秒，还不退就结束它。agent 进程自己退出了也收尾（它留下的子进程拿着输出也不等）。
   - 退出码：`end_turn` 0；被取消、收到 SIGTERM 130；`max_tokens`、`max_turn_requests`、`refusal` 3；握手失败（例如没登录）、协议错误、v2 这一轮出错（`stopReason: error`）2；这一轮因为订阅账号的额度用完出错 4（下一条）；这一轮没结束 agent 就退出了：agent 的退出码（它是 0 时记 1）。原因写进日志。
   - **额度**（#668）：只用跑的时候顺带拿到的，不另外去查。Claude Code 的 `usage_update` 在 `_meta["_claude/rateLimit"]` 里带 Agent SDK 的 `rate_limit_info`（`status`、`rateLimitType`、`utilization`、`resetsAt`），记下最近一次的；这一轮出错、错误的 `data` 是 `{errorKind: "rate_limit", rateLimit}` 且状态是 `rejected` 时是额度用完（一时的 429 状态是 `allowed`，照旧算出错）。Codex（codex-acp）不往 ACP 上发额度，用完时这一轮出错、`data.codexErrorInfo` 是 `usageLimitExceeded`，重置时间只在文字里（「Try again at 3:15 PM.」，本机时区）。报错文字以 Agent SDK 的 `USAGE_LIMIT_ERROR_PREFIXES`（「You've hit your」等）开头的也算。`board host` 给每次派发带 `--quota-file <run 目录>/<派发 ID>.quota`，`board acp` 一看到额度变了就写进去（先写临时文件再换上），`board host` 跑卡期间每 2 秒读一次、变了就随上报带上（`quotas`），结束时随派发结束再报一次（`quota`），服务端按 PRD §9.1「额度用完」「看额度」处理。`unifiedWindows` 里的 5 小时（`five_hour`）、本周（`seven_day`）两个窗口都记下（`Quota.windows`）。
   - **查额度**（`board acp --usage /usage`、`--usage /status`）：界面上点「刷新」时 board host 用它查一次。开会话、发这条本地命令（Claude Code、codex-acp 自己答，不调模型、不花额度；agent 公布的命令里没有它就不发），读出 5 小时、本周两个窗口，写一行 `{"usage": 额度}`（没查到时 `{"usage": null, "error": 原因}`）到标准输出，关掉会话就退出，最多 90 秒。
   - 人点「停止」、`board host` 退出时它收到 SIGTERM：发 `session/cancel`，最多等 5 秒（之后照下面第 6 步结束进程树）。

   启动环境是「在哪种 shell 里启动」，按机器、按 agent 在机器详情里选（PRD §9.1）：

   | 启动环境 | 系统 | 怎么启动 |
   |---|---|---|
   | 直接启动（`direct`） | 所有 | 直接执行命令，不经过 shell |
   | 登录 shell（`login`） | macOS、Linux | `$SHELL -l -i -c 'exec "$0" "$@"' <命令> <参数>…`：读 `~/.zprofile`、`~/.zshrc` 这些启动脚本，写在里面的 `PATH` 也生效 |
   | cmd | Windows | `cmd.exe /d /s /c "<命令> <参数>…"`：每个参数加双引号；命令名是普通名字时不加引号，含路径、空格等字符时才加（`.cmd` 靠 `%~dp0` 找自己旁边的文件，加了引号的命令 `%~dp0` 是当前目录，相对路径就找错了） |
   | Windows PowerShell、PowerShell 7（`powershell`、`pwsh`） | Windows | `-NoLogo -NonInteractive -ExecutionPolicy Bypass -Command "& '<命令>' '<参数>' …; exit $LASTEXITCODE"`：`npx`、`npm` 写成 `npx.cmd`、`npm.cmd`，不执行 Node.js 自带的 `npx.ps1`、`npm.ps1`（它们在 `-Command` 里从整行脚本截参数，截错就报错） |
   | Git Bash（`git-bash`） | Windows | `bash.exe -l -c 'exec "$0" "$@"' <命令> <参数>…` |
   | WSL（`wsl:<发行版>`） | Windows | `wsl.exe -d <发行版> --cd <目录> --exec setsid -w bash -l -i -c 'exec "$0" "$@"' <命令> <参数>…`，目录换成 `/mnt/…` |

   没选时按这个 agent「支持的启动环境」的顺序（下表），取本机有、并且在里面找得到命令的第一个。macOS、Linux 上都是登录 shell → 直接启动。

2. 环境变量带上 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_AS`（上面的名字）、`BOARD_HOST`（机器名）、`BOARD`（`<项目>/<白板>`），以及 `BOARD_CONFIG`：这次派发自己的配置文件 `~/.board/host/run/<机器名>/<派发 ID>.toml`（服务端地址和令牌，只有自己能读，派发结束时删除）。agent 里的 `board` 命令直接用它们，不用再 `--as`、`--board`。

   还有 `BOARD_EXE`：`board host` 自己的路径。`board-skills` 的 `scripts/board` 看到它就用它，不用 skill 里打包的 `board`，两边版本一致。

   经过 shell 启动时，shell 的启动脚本里设了 `BOARD_URL`、`BOARD_TOKEN` 也不影响：包装脚本在 `exec` 之前用 `board host` 的值（`BOARD_HOST_URL`、`BOARD_HOST_TOKEN`）再设一遍。WSL 里的变量经 `WSLENV` 传进去：`BOARD_CONFIG` 换成 WSL 路径，不传 `BOARD_EXE`（Windows 的 `board.exe` 在 WSL 里用不了，用 skill 自带的 Linux 版 `board`）。

   为什么还要 `BOARD_CONFIG`：有的 agent 不把名字像密钥的环境变量传给它执行的命令。dsh 会去掉名字里带 `KEY`、`PASSWORD`、`SECRET`、`TOKEN` 的变量，`BOARD_TOKEN` 到不了 `board`，`board` 就会拿 `~/.board/config.toml` 里别的令牌去连，得到 `INVALID_TOKEN`。`board` 读 `BOARD_CONFIG` 指向的文件代替 `~/.board/config.toml`，这个变量名不像密钥，能传过去。
3. `board acp` 的标准输出和标准错误写到 `~/.board/host/logs/<派发 ID>.log`：一行一帧 ACP 消息（JSON-RPC：它发出去的请求、agent 的回应和 `session/update` 通知；连续的消息片段合成一行，最多 2 秒落一次），加上 agent 和 `board acp` 写到标准错误的提示。`board host` 拿它做两件事：

   - **原始输出**：每 5 秒上传新增的行（最多最后 500 行），是 `board host` 转写后的一行一行（`crates/board-cli/src/logfmt.rs`）：agent 说的话、`▸ 工具 参数`、`↳ 结果第一行（共 N 行）`、出错的 `✗ …`、结束时的 `■ 结束`（不是正常结束时写原因）。不是 JSON 的行（标准错误里的提示）原样保留。界面「会话」切到「原始输出」看它。
   - **过程**（结构化事件，`crates/board-cli/src/events.rs`，定义在 `crates/board-core/src/runlog.rs`）：每一帧解析成统一的事件——`message`（`agent_message_chunk` 说的话，Markdown；`plan` 写成任务列表）、`tool`（`tool_call` / `tool_call_update`：工具名按 `kind` 取 shell、read、edit 这类短词，认不出时用标题；参数摘要、结果摘要、成功或失败，`rawOutput` 里有退出码时记下）、`error`（出错的回应）、`result`（prompt 的回应：是否正常结束、输入 / 输出 tokens、费用，有才记）；思考、握手、命令列表、模式和配置的变化不记；不是 JSON 的行记成 `text`，原样保留。每 2 秒上传一次新增的（`POST /api/dispatches/<ID>/events {fmt, events}`），服务端按派发存（每个派发最多 2000 条，单条超过 4 KB 截断；派发结束 30 天后只留 `result` 和最后 50 条），有新事件时推送，会话页打开着就实时追加。工具调用和结果分成两条传、带着同一个调用标识，服务端合成一条。会话换到下一张卡后，事件记到当时在做的那张卡上。界面「会话」默认显示它。

   所有 agent 都是同一种格式（上报 `fmt: "acp"`）：输入 / 输出 tokens 是 prompt 回应里带的（agent 给了才有），费用是最后一次 `usage_update` 里的累计值。

   上传前脱敏（`crates/board-cli/src/redact.rs`）：agent 执行 `env`、打印配置文件、报错时带出请求头，令牌和其他密钥不能跟着上传到服务端（任何登录的人都能看到界面上的输出，异常退出时评论里还带最后几个事件和最后 20 行）。`board host` 在向服务端发原始输出、事件，写异常退出评论之前，把这些换成 `<已隐藏>`：当前令牌的原文、这次派发注入的 `BOARD_TOKEN`、`BOARD_HOST_TOKEN`，这次从服务端取到的供应商 key、MCP 密钥值和 Git 令牌，以及常见格式的密钥（`sk-…`、`ghp_…`、`gho_/ghs_/ghu_/ghr_…`、`github_pat_…`、`glpat-…`、`bd_` 加 48 位十六进制、`xox…-…`、`AKIA…`、整段 `-----BEGIN … PRIVATE KEY-----` 块、`Authorization: Bearer …` 的凭据、URL 里的 `user:password@`）。本机的日志文件保留原文。普通的十六进制提交号不会被误伤。

   本机日志文件只有自己能读（0600）；单个日志超过 50 MB 时只留开头 1 MB 和最后 10 MB，中间写一行省略说明；结束超过 30 天的在 `board host` 启动时、之后每天删一次，天数用 `~/.board/config.toml` 的 `[host] log_days` 改。
4. 名字的心跳由 `board host` 发。`board-skills` 看到 `BOARD_HOST` 且卡片已由 `BOARD_AS` 持有，就不再 check、不再领取、不管心跳，只干活、提交推送、交审或交给人。手动启动的会话不用自己拼名字：`board check --agent <标识>` 由 Board 按这台机器的设置生成同样的名字（§3.2）。
5. agent 退出后看卡片在哪：在交审状态是「已交审」；已经不归它、在其他状态是「做不下去」；还归它就记「异常退出」，卡片移到错误节点（没有时移到需要人处理的状态），评论里带退出码、最后 10 个事件的文字（工具名加结果摘要、agent 最后说的话），原始输出的最后 20 行折叠在后面。额度用完（上面第 3 步）是例外：记「额度用完」，不算异常退出，卡片回到领取前的可领取状态最上面，这台机器上这个 agent 的本机配置角色暂停到重置时间（报了同一账号的别的机器一起暂停，下面「身份和官方订阅」）。
6. 人点「停止」或 `board host` 退出时，结束包括启动环境外壳在内的整个进程树：macOS、Linux 上先给整个进程组发 SIGTERM，10 秒后 SIGKILL（WSL 里 agent 的进程号写在 `BOARD_PIDFILE`，在 WSL 中给它的进程组发 SIGTERM / SIGKILL）；Windows 上 board host 给每个 `board acp` 建一个 Job Object（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`），`board acp` 启动 agent 时也放进一个 Job，停止和回收用 `TerminateJobObject`，不再沿父进程链找进程。agent 的主进程自己退出时，同一进程组 / Job 里剩下的进程也一起结束；Windows 的 Job 由内核记录成员，父进程先退出也不影响，`board host` 或 `board acp` 崩溃、句柄关闭时系统会结束整棵进程树。agent 自己再 `setsid` 出去的进程不在组里，管不到。`board host` 被 `kill -9`、崩溃后同名重启，会先结束上一次留下还在跑的 agent（按 `~/.board/host/run/<机器名>/<派发 ID>.proc` 里的进程号和启动时间）。

## 在一台新机器上装 `board`、接任务

只有命令行的机器（服务器、devbox）用界面「机器 → 添加机器」生成的那一行命令：填机器名和工作空间（每个项目的目录是 `<工作空间>/<项目目录>`，项目目录留空用项目标识，项目下的白板共用），界面以机器名新建一个访问令牌，给出

```sh
curl -fsSL <服务端>/install.sh | sh -s -- --token <令牌> --name <机器名> --workspace <工作空间> --service
```

（Windows 是一行 PowerShell）。它从发版附件下载本平台的 `board`、校验 SHA-256（对不上就中止）、装到 `~/.local/bin` 并写进 PATH，然后 `board login`、`board host --install`（装成系统服务，开机自动接任务）。装的 `board` 和服务端同一个版本；之后服务端升级了，在机器详情「管理」里点「升级到 <版本>」，或在那台机器上执行 `board update`。agent 本身（Claude Code、Codex……）可以在机器详情「agent」页点「安装」，也可以自己装好；`board-skills` 见下面两节。

## 让机器重启后自动接任务

机器重启（系统更新、断电）后，要在有人登录、打开程序之前就自己上线，不然派给它的卡只会排队，自动领取也白设。

- **装了桌面端的电脑**：在「机器 → 添加机器 → 这台电脑」点「开始接任务」时会一起打开「开机时自动启动」（配置文件里还没有令牌时自动以机器名建一个）。之后登录系统时桌面端在后台启动，不弹窗口，只显示托盘 / Dock 图标，照常接任务。不想要时在这台电脑的机器详情「设置」最上面单独关掉这个开关。
- **只有命令行的机器**：「添加机器」生成的那一行已经带了这一步。手动装：

  ```sh
  board host --install --name devbox --workspace ~/repo
  ```

  它按这些参数装成开机自启的服务并立即启动：macOS 是 launchd 用户代理（`~/Library/LaunchAgents/dev.board.host.plist`），Linux 是 systemd 用户服务（`~/.config/systemd/user/board-host.service`），Windows 是登录时运行的计划任务「Board host」。`board host` 异常退出（包括被 `kill -9`）10 秒后自动重启；输出在 `~/.board/host/host.log`（超过 5 MB 轮转）。

  ```sh
  launchctl print gui/$(id -u)/dev.board.host     # macOS：看状态
  systemctl --user status board-host.service       # Linux：看状态
  tail -f ~/.board/host/host.log                   # 看输出
  board host --uninstall                           # 卸载：停掉并删掉服务
  ```

  Linux 上 systemd 用户服务默认只在这个用户登录后运行；要开机后没人登录也运行，执行一次 `loginctl enable-linger $USER`（可能要 sudo），`--install` 发现没开时会提示。服务只读 `~/.board/config.toml` 里的服务端地址和令牌，不带 shell 里的 `BOARD_URL`、`BOARD_TOKEN`：先 `board login`。

## MCP 服务器：在「Agent → MCP 服务器」里维护

卡片要用到的工具（浏览器、数据库、内部文档这类 MCP 服务器）不用每台机器、每个 agent 一个个配：在「Agent → MCP 服务器」里加一次，每台机器的 board host 写进它找到的 agent 的用户级配置：

| agent | 写到哪里 | 怎么写 |
|---|---|---|
| Claude Code | `~/.claude.json` 顶层 `mcpServers`（user scope） | 直接改文件：读完、改 `mcpServers.<名称>`、写临时文件再换上；保留原文件权限（没有原文件时 0600），写完重读确认，被 Claude Code 同时改写时重试一次 |
| Codex | `$CODEX_HOME/config.toml`（默认 `~/.codex/config.toml`）的 `[mcp_servers.<名称>]` | 直接改文件：`command`、`args`、`env`，或 `url`、`http_headers` |
| dsh | — | 不支持，界面上写「不支持」 |

- 机器详情「设置 → skills 和 MCP」里 MCP 选了「用服务器的」才写（新机器默认「不改」：Board 不碰它的 MCP 配置，之前加的也不删）。
- 加过哪些记在 `~/.board/host/mcp-sync.json`。agent 里已经有同名、不是同步加的：原来的值存进 `~/.board/host/mcp-backup.json`，换成共用配置里的，之后归 Board 管；在共用配置里删掉的，同步加的那条也删掉，自己加的不动。
- 值可以标成密钥（token、API key）：只存在服务端，界面上只显示末尾 4 位，只发给 board host；会以原文写进 agent 自己的配置文件（agent 要用），所以这些文件和 `mcp-backup.json` 仍然只该自己能读（0600）。
  - 原文只发给**这台机器用的那个令牌**：`GET /api/hosts/{名字}/mcp` 比对机器最后一次上报时带的令牌 ID，只在这台机器在线、名字对得上、令牌是它上报用的那个时返回原文；别的令牌（包括界面上新建的、别的机器用的）、登录的界面、离线的机器都拿到 `NOT_LOGGED_IN`；打码的 `GET /api/mcp` 只在界面上用。所以能拿到的范围是「一台在线机器自己的令牌」，不是「任意 agent 令牌」。
- 装了桌面端的电脑可以「从本机导入」：把这台电脑上已经配好的服务器勾选后加进共用配置，名字像密钥的值自动标成密钥。

## 模型供应商和角色：谁用哪个模型

供应商（地址、API key、模型、每个模型支持的思考强度）在「Agent → 模型供应商」里加一次，各台机器共用。谁用哪个在「Agent → 分工」的**角色**里定：一个角色 = agent + 模型来源（「<供应商> · <模型>」；「本机配置」：按这台机器上 agent 自己的配置跑，可能是订阅、API key 或中转，模型可以不填；「官方订阅」：只有 Claude Code、Codex，跑法同本机配置，但只派给 agent 报的身份是订阅的机器，见下面「身份和官方订阅」）+ 强度，可选附加说明（接在提示词后面）、可选「只在这些机器上跑」。领取规则是「条件 → 角色」，手动派发也选角色；`board dispatch <ID> --host <机器> --profile <角色>`。

board host 每次派发按角色给**这一个 agent 进程**带上供应商、模型和强度，不改 agent 的全局配置文件，agent 直接连供应商，不经过 Board（Board 不做网关，也不转协议）：

| agent | 怎么换供应商 | 模型、强度 |
|---|---|---|
| Claude Code（`claude-agent-acp`） | ACP 握手后、开会话前发 `providers/set`：`providerId: "main"`、`apiType: "anthropic"`、`baseUrl`（不带 `/v1`）、`headers: {"Authorization": "Bearer <key>"}`。适配器把它写进会话的程序化 settings，优先级高于 `~/.claude/settings.json` 里的 `env` | ACP 会话配置项：类别 `model`、`thought_level`。用供应商时这个进程还带 `ANTHROPIC_CUSTOM_MODEL_OPTION=<角色的模型>`（#727）：适配器换了供应商后，模型下拉只留 SDK 自带模型里网关也列了的，网关只列 `group/4.1f` 这类自定义模型时就设不上；Claude Code 的[自定义模型选项](https://code.claude.com/docs/en/model-config#add-a-custom-model-option)会进 SDK 的模型表，不用写 `settings.json` |
| Codex（`codex-acp`） | 同样 `providers/set`：`providerId: "openai"`、`apiType: "openai"`（Responses 接口）、`baseUrl`、`headers`。适配器以 `-c model_provider=…` 覆盖启动 codex app-server，不改 `config.toml` | 同上 |
| dsh | 这次派发的 `--patch` 后面接上 `llm-pi-ai` 里一条 `board-role` 路由和 `acp` 用的提供方、模型、强度；key 放在这个进程的环境变量 `BOARD_ROLE_API_KEY` 里 | 写在补丁里；没有供应商时用会话配置项 `reasoning_effort` 切强度 |
| 配置文件里新加的 agent | 不能换供应商：角色只能选「本机配置」 | 会话配置项 |

- `providers/set` 的参数（含 key）写进 `~/.board/host/run/<派发 ID>.provider.json`（只有自己能读），交给 `board acp --provider-file`，`board acp` 第一步读完就删；派发启动失败时 board host 也会兜底删掉 provider 文件和这次的 dsh 补丁，派发日志里这一帧的请求头写成 `***`。换供应商失败（例如网关列不出模型）时不开会话，按异常退出记下原因。
- 本机配置、官方订阅的角色不换供应商，只切模型、强度；没填模型时不切，用 agent 自己的默认。

**身份和官方订阅**（#667）：本机配置不保证走订阅——机器上有别的凭据时会悄悄盖过订阅（Claude Code 的[凭据优先级](https://code.claude.com/docs/en/authentication)里 `ANTHROPIC_AUTH_TOKEN`、`ANTHROPIC_API_KEY`、`apiKeyHelper` 都排在 `/login` 的订阅前面，`~/.claude/settings.json` 的 `env` 里的也算；Codex 的 `config.toml` 里 `model_provider` 指向中转、`auth.json` 里是 API key 时不走 ChatGPT 订阅）。所以要确认走订阅的用「官方订阅」：

- **问身份**：`board acp --auth only` 只握手、不开会话（board host 用的 `--auth probe` 问完身份再列模型，见下面「本机账号能用的模型」）：agent 公布了 `auth.status`（ACP [Authentication State Query RFD](https://agentclientprotocol.com/rfds/get-auth-state)，claude-agent-acp）就发 `auth/status`，从账号的 `_meta.claudeCode.kind` 取种类；只公布了 `_meta.authStatus`（扩展，codex-acp）就等它推 `_auth/status_update`（`{authStatus: {kind, label, detail?, account?}}`），最多 30 秒。种类：`account` 订阅、`api_key`、`gateway` 网关 / 中转、`external` 云服务商、`none` 没登录；问不出来记 `unknown`。订阅账号的邮箱（#669）也取：推送的在 `authStatus.account.email`，`auth/status` 的在账号的 `email`（组织名不取）。写一行 JSON 到标准输出。
- **上报**：board host 启动时、agent 版本变了、点「自检」时在后台问 Claude Code、Codex 一次，随上报带上（`HostAgent.auth`），机器详情 agent 页写「用的身份：Claude Max · 订阅 · <邮箱>」。报的是没登录时，本机配置、官方订阅的角色都不派到这台。
- **邮箱认账号**（#669）：同一个 agent 报同一个邮箱（不区分大小写）的机器算同一个账号。一台机器上额度用完时，同一账号的别的机器一起暂停到重置时间，在其中一台点「恢复」一起恢复（PRD §9.1「额度用完」）；没报邮箱的（API key、中转、旧版 board host）只停自己。邮箱只存在机器数据里：不写进卡片动态、派发记录、操作记录，`board acp` 写派发日志时把身份推送、`auth/status` 回应里的邮箱和组织名去掉。
- **派发**：官方订阅的角色只派给报的身份是订阅的机器（没报过、不知道的也不派）；派发时 board host 给 `board acp` 带 `--auth subscription`，它在 `initialize` 之后、`session/new` 之前再问一次，不是订阅就出错结束（「按「官方订阅」派发，agent 用的不是订阅：…，没有开会话」），不开会话。
- 只判断、不改机器上的配置：发现 key 盖过了订阅时只写明，不去删 settings.json 或环境变量。实测结果见 [acp.md](acp.md) 开头的补记。

**本机账号能用的模型**（#666）：board host 问身份时用的其实是 `board acp --auth probe`：问完身份再 `session/options`（ACP [Session options RFD](https://agentclientprotocol.com/rfds/session-options)）取模型下拉和默认值，开一个不发提示词的会话逐个切模型读出每个模型的强度，读完关掉，输出一行 `{"auth": …, "models": …}`，随上报带上（`HostAgent.local_models`）。不发提示词，不耗额度（Claude Code 列 13 个模型约 12 秒，Codex 约 4 秒）。

- 角色表单：本机配置、官方订阅时，模型下拉和强度来自选中的机器报的表；fit、保存角色按表校验（见 PRD §9.1「分工」）。
- 派发：v2 的 agent 公布了 `session.options` 时，模型、强度在 `session/new` 的 `configOptions` 里当初始值带上（先模型后强度），回应里的实际值对不上就出错结束（「账号里没有模型 X」「模型 X 不支持强度 Y」），不再静默按默认跑。
- 名字和派发记录：本机配置、没填模型时用报的默认模型和默认强度。
- 每个角色只能选接口对得上的供应商：Claude Code 只认 Anthropic Messages，Codex 只认 OpenAI Responses，dsh 三种都认（路由的 `api` 跟着写成 `anthropic-messages`、`openai-completions` 或 `openai-responses`）。模型只能从这家供应商的模型里选；强度只能选这个 agent 能设、模型也标了的档位。
- 派发名字的第二段和派发记录里的模型、强度是角色的；本机配置、没填模型时用 board host 从 agent 配置文件里读出来的。
- 以前在机器详情里「指定」过供应商的 agent：Board 之前写进它配置文件的那组留着（按「本机配置」跑、角色没填模型时用的就是它）；board host 不再按机器上的设置写 agent 的配置文件（#576），要换就在 agent 自己的配置里改，或者建一个用供应商的角色。升级时规则里用到的每个 agent 建一个同名的「本机配置」角色，行为和以前一样。
- API key 只存在服务端：界面上只显示末尾 4 位，完整的值只发给这台机器用的 board host 令牌（`GET /api/hosts/{名字}/providers`，按角色派发时现取），不写进日志和卡片动态。


## Git 凭据：在「Agent → Git 凭据」里设一次

机器推不动代码（`could not read Username for 'https://github.com'`）多半是那台机器没配 git 的 https 令牌。不用每台机器配一次：在「Agent → Git 凭据」填主机（例如 `github.com`）、用户名、令牌（要有推送权限，GitHub 的 fine-grained 令牌给 Contents 读写），以及可选的提交用户名、提交邮箱（例如 `BUGLAN`、`name@example.com`；两个都填或都空），打开了同步的机器的 board host 把它写进这台机器的用户级 git 配置，按凭据的主机全局生效：

- 只加、改用户级 git 配置（`~/.gitconfig`）里的 `credential.https://<主机>.helper`：先是一个空值（清掉系统里别的 helper），再留用户自己为同一主机写的 helper，最后是 `store --file '~/.board/host/board-git-credentials'`。其它主机、其它 helper 和文件里的其它配置都不动，不整份覆盖。之后新建、克隆的仓库只要远端是同一个 https 主机就直接生效，不用再点一次同步。
- 填了提交身份时，同一套同步还会写 `user.name` / `user.email`，新建提交的作者、提交者就用这两个值，不再落成系统账号和 `用户名@主机名.local`。
- 令牌放在 `~/.board/host/board-git-credentials`，只有本人能读；board 的输出、日志里没有它。
- 机器详情「设置 → skills、MCP 和 Git 凭据」里每台机器一个开关，新连上的机器默认「不改」（Board 不碰它的用户级 git 配置，需要时在机器详情或「Agent → Git 凭据」卡片上打开）；改成「不改」或在共用配置里删掉，board host 去掉这一项、清掉旧版按仓库配过的、删掉令牌文件，并把 Board 写过的提交身份恢复成覆盖前的值。用户后来自己改过的 `user.name` / `user.email` 不会被覆盖。
- 中途出错（git 配置读不出来、记录存不上、清理没做完）时机器详情里这一行显示同步失败和原因，Board 不会把你原来的 `user.name` / `user.email` 当成空的，也不会删掉还被 helper 指着的令牌文件，下次同步再试。恢复信息记在 `~/.board/host/git-sync.json`，这个文件坏了时 Board 什么都不改，修好或删掉它再同步。
- 只管 https 远端。远端是 SSH（`git@…`）的仓库照旧用本机的 key；别的 https 主机不受影响。
## 一个 agent 要满足什么

接入前逐条验证（「本机验证」一节有命令）：

| # | 要求 | 不满足会怎样 |
|---|---|---|
| 1 | **讲 ACP**（Agent Client Protocol v1，标准输入、输出上的 JSON-RPC）：自己支持（dsh 的 `--profile acp`），或者有维护中的 ACP 适配器（Claude Code 的 `claude-agent-acp`、Codex 的 `codex-acp`，见 [ACP 官方 agent 列表](https://agentclientprotocol.com/get-started/agents)） | 接不进来：board host 只用 ACP 启动 agent |
| 2 | **不问权限**：能读写仓库、执行命令、访问网络（`git push`、`board`），不开沙箱。agent 公布了不问权限的模式（写进 `acp_mode`）、或用环境变量关掉审批和沙箱（写进 `acp_env`）；ACP 的权限请求 board acp 会自动允许 | 命令被沙箱拒绝，做不下去 |
| 3 | **找得到 `board-skills`**：从某个 skill 目录加载它，提示词以 `/board-skills` 开头时照着它做 | 不认识这句话，什么也不做就退出 → 异常退出（退出码常常是 0） |
| 4 | **在这台机器上已经登录 / 配好模型**（API key、OAuth、provider） | 一启动就报错退出 |
| 5 | 它执行的命令能拿到 `BOARD_URL`、`BOARD_AS`、`BOARD`、`BOARD_CONFIG`、`BOARD_EXE` 这几个环境变量（`BOARD_TOKEN` 被去掉没关系） | `board` 连错服务端、找不到名字或白板 |
| 6 | **运行中**用 `session/update` 报进度（说的话、工具调用和结果）；v1 这一轮做完用 `stopReason` 回应 prompt，v2 报带 `stopReason` 的 `idle`。用后台命令时：v2 在 `idle` 上报 `background`、跑完自己接着做；只会 v1 的，后台命令在这一轮里等它跑完再结束这一轮 | 「过程」里看不出它在干活还是卡住了；这一轮没结束就退出算异常退出；卡片还在手里、后台也没东西在跑就结束这一轮会被续一轮（最多 3 轮），续满了算异常退出 |
| 7 | 收到 `session/cancel` 会结束这一轮；标准输入关闭（EOF）或收到 SIGTERM 会退出 | 停止要等 10 秒后强制结束 |
| 8 | `<命令> --version` 第一行是版本号（用适配器的，是 agent 自己的命令） | 界面上只显示「找到了」（不影响使用） |
| 9 | **在它支持的启动环境里能跑**：在每个支持的环境里 `command -v <命令>`（cmd、PowerShell 里 `where.exe <命令>`）找得到，ACP 入口在里面能做完一张卡（用适配器的，`npx` 也要找得到）。Windows 上不支持的环境不写进模板 | 这个环境里显示「没找到」或「缺 Node.js（npx）」，或派过去异常退出 |

agent 标识：小写字母、数字、`.`、`-`，以字母开头，最长 16 个字符（名字 `<机器名>/<模型名>-<强度>` 不超过 48 个字符）。它也是 skill 生成名字时 `board check --agent <标识>` 的参数（手动启动的会话按它取机器设置里的模型）；内置模板的标识见下表。

## 已经接入的（内置模板）

内置模板写在 `crates/board-core/src/agentdef.rs` 的 `AGENT_DEFS` 里（board host、服务端、桌面端、界面都从这一处读）。**顺序就是默认 agent 的顺序**：本机找到的第一个是这台机器的默认 agent（自动领取用它，手动派发时预先选中它），机器详情里可以改。「支持的启动环境」的顺序就是默认选择的顺序；macOS、Linux 上都是登录 shell → 直接启动。「`PATH` 里没有时再看」的位置只在直接启动、登录 shell 里用。「默认模型」是派发的 agent 名字（`<机器名>/<模型名>-<强度>`）的第二段；`—` 表示这个 agent 没有默认的模型，名字的第二段用 agent 标识（例如 `mbp/codex`），配置文件里的 `model` 可以覆盖。

| 顺序 | 标识 | 名称 | agent 命令 | ACP 入口 | `PATH` 里没有时再看 | 读 skill 的目录 | 支持的启动环境（Windows） | 前提 |
|---|---|---|---|---|---|---|---|---|
| 1 | `dsh` | DeepSeek Harness | `dsh` | `{bin} --profile acp --patch {patch}`（自带，0.2.2 起） | `/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh`（`~/Applications` 下同样） | `~/.agents/skills` | `pwsh` → `powershell` → `cmd` | 在 DeepSeek Harness 的 acp profile 里配好模型（或在 Board 的角色里选供应商） |
| 2 | `claude` | Claude Code | `claude` | `npx -y @agentclientprotocol/claude-agent-acp@0.107.3` | `~/.local/bin/claude`、`~/.claude/local/claude` | `~/.claude/skills` | `pwsh` → `powershell` → `git-bash` → `wsl:*` → `cmd` | 在终端里运行一次 `claude` 并 `/login`；装了 Node.js（`npx`） |
| 3 | `codex` | Codex | `codex` | `npx -y @agentclientprotocol/codex-acp@2.12.0` | — | `$CODEX_HOME/skills`（默认 `~/.codex/skills`） | `pwsh` → `powershell` → `git-bash` → `wsl:*` → `cmd` | 运行一次 `codex login`；装了 Node.js（`npx`） |

「agent 命令」是在每个启动环境里找的命令：取版本号、判断安装方式、安装和更新都按它。「ACP 入口」是 `board acp` 执行的命令：`{bin}` 换成找到的 agent 命令，`{patch}` 换成这次派发的 profile 补丁。agent 自己不讲 ACP 的用 ACP 组织维护的适配器，`npx -y` 固定版本运行：不用另外安装，升级跟着 Board 发版；npm 缓存放在 `~/.board/host/npm-cache`（WSL 里用它自己的），启动环境里还要找得到 `npx`：board host 找 agent 命令时顺带找它，找不到就把这个环境里的 agent 当成不可用、显示「缺 Node.js（npx）」；第一次派发要下载适配器；npm 的更新、fund、audit 提示关掉（`npm_config_update_notifier` 等设成 `false`），不混进派发日志。适配器用机器上找到的 agent（Claude Code 经 `CLAUDE_CODE_EXECUTABLE`、Codex 经 `CODEX_PATH` 告诉它），所以机器详情里的版本就是实际在跑的版本；Windows 上找到的是 npm 给全局命令生成的垫片（无扩展名的 sh 脚本、`.cmd`、`.ps1`，适配器执行不了）时，改传 npm 包里真正的程序：Claude Code 是 `<npm 前缀>\node_modules\@anthropic-ai\claude-code\bin\claude.exe`，Codex 是平台包 `@openai/codex-win32-<架构>` 里的 `vendor\<目标>\bin\codex.exe`；找不到时不传，适配器用它自带的那份（版本可能和机器详情里写的不一样）。

配置文件里新加的 agent 在 Windows 上支持 `pwsh` → `powershell` → `git-bash` → `wsl:*` → `cmd` → `direct`；同标识覆盖内置模板时，支持的启动环境不变。

**模型和思考强度**：按角色派发时模型、强度按 ACP 会话配置项切（上面「模型供应商」），不按次传命令行参数；本机配置、没填模型的角色用 agent 自己配置文件里的。派发名字的第二段是角色的模型，或 board host 读出来的、它配置文件里现在的模型（例如 `mbp/sonnet-high`；强度也读出来接在后面），读不到时用配置文件 `[agents.<标识>] model`，再没有就是 agent 标识。

各自的说明：

- **dsh**：用它自带的 acp profile（`@deepseek-ai/dsh-acp-app`，0.2.2 起；本机 `dsh --version` 更旧时不启动，提示升级）。`--patch {patch}` 是 board host 每次派发写的补丁（`crates/board-cli/src/dsh-acp.patch.yml`），关掉 `goal-round-driver`、`tool-goal`、`command-goal`：acp profile 也挂着 goal，模型一旦 `create_goal`，这一轮结束后会自动续轮，prompt 迟迟不结束。环境变量 `DSH_PERMISSION_MODE=danger-full-access` 关掉沙箱和审批（默认的 `workspace-write` 只能写当前目录，`board-skills` 要在仓库旁边建 worktree、`git push`，不够用）。会话用的提供方和模型是 acp profile 里 `acp` 这一项配置的（`~/.dsh/profiles/acp/cordis.patch.yml`；默认 `deepseek-official` 的 `deepseek-v4-flash`），强度在会话里用 `reasoning_effort` 切；以前给 headless profile 配的模型不会自动带过来。dsh 不报 tokens 和费用。它执行命令时去掉名字像密钥的环境变量（见上文 `BOARD_CONFIG`）。DeepSeek Harness 的「设置」里可以把命令行装到 `/usr/local/bin/dsh`；没装也能用，`board host` 会用 app 里自带的那个。dsh 只会 ACP v1：它也有后台任务（前台命令跑满 2 分钟自动转到后台，跑完唤醒 agent），但 v1 报不出来，`board acp` 看不到，靠续轮的提示词让它在这一轮里等后台跑完（#461）。
- **Claude Code**：适配器 `claude-agent-acp`（Claude Agent SDK），走 ACP v2。会话开始后切到 `bypassPermissions` 模式；root 用户下没有这个模式，权限请求照样自动允许。后台任务照常用：Claude Code 把长命令（`npm test` 这类）放到后台、先结束这一轮时，`idle` 带着 `background`，`board acp` 等它跑完、Claude Code 自己接着做（#461）。命令的输出在终端里（`terminal_output_chunk`、跑完时带完整输出的 `terminal_update`），日志边跑边长。登录信息在系统钥匙串里：从别的 agent 会话、沙箱里启动的 `board host` 可能读不到，会报要先登录，从自己的终端或桌面端运行就好。
- **Codex**：适配器 `codex-acp`（Codex App Server），走 ACP v2（报 `running` / `idle`，不报 `background`；Codex 的后台终端跑完不会自己再开一轮，它在这一轮里轮询，结束这一轮时卡片还在手里就续轮）。环境变量 `INITIAL_AGENT_MODE=agent-full-access`，会话开始后也切到这个模式：不审批、不开沙箱。skill 读 `$CODEX_HOME/skills`，默认 `~/.codex/skills`。
## 安装和更新 agent

机器详情「agent」页会显示每个内置 agent 的版本、安装方式（npm 全局、Homebrew、官方安装脚本、桌面 app 自带或配置文件）和有没有新版本。board host 每天在后台查一次 npm 上的最新版本，不把网络请求放进 10 秒心跳；打开机器详情看到的是最近一次检查的结果。

- 没找到的内置 agent 给出安装命令，可以复制到那台机器执行，也可以在界面点「安装」由 board host 执行。macOS、Linux 上 npm 包装进 `~/.local`（`npm install -g --prefix ~/.local …`），不需要管理员权限；board host 也会在 `~/.local/bin` 找命令。首次装进用户目录后把 `~/.local/bin` 写进登录 shell 的配置（zsh 的 `~/.zshrc`、bash 的 profile、fish 的 `config.fish`，只追加一次），新开终端也能直接敲命令。三个内置 agent 的 npm 包分别是 `@deepseek-ai/dsh`（用 `next` 标签）、`@anthropic-ai/claude-code`、`@openai/codex`。
- 找到的按命令实际路径识别安装方式：npm 全局沿实际安装目录用 `npm install -g --prefix <prefix> <包>@<版本>`，Homebrew 用 `brew upgrade <formula>`，官方安装脚本用命令自己的 `update`，桌面 app 自带的随 app 更新。npm 全局目录当前用户写不了时（例如以前用管理员权限装进了 `/usr/local`），直接把目标版本装到 `~/.local`，并把 `~/.local/bin` 放到登录 shell 的 PATH 前面；系统目录里的旧版本不动，board host 和用户新开终端都用较新的用户目录版本，不需要 sudo。写了 prefix 的 npm 安装、更新先装进 prefix 里的临时目录 `.board-stage-<agent>`，装好后才把包和命令换进去：npm 被中途结束时会把旧包、旧命令挪成隐藏文件不挪回来，先装到别处，失败、超时只删临时目录，旧版本一直能用。一次最多跑 30 分钟，用 `~/.board/config.toml` 的 `[host] agent_job_minutes` 改。装完 board host 自动重新检查并上报新版本；失败也重新检查一次，派发前发现找到过的命令不在了（软链断了、被删了）同样先重新检查，不拿失效的路径去启动。失败原因留在卡片上，可以重试。更新在后台子进程里跑，不占会话位，不影响正在跑的派发。
- 机器设置「agent 更新 → 自动更新 agent」默认关。开着时，board host 发现有新版本、这台机器没有在跑的派发，就按上面同一套方式更新，机器操作记录里写「自动更新 <agent> 到 <版本>」；失败后把卡片标成失败，等人重试，不会每 10 秒反复重试。
- 配置文件里新加的 agent、改了内置 agent 启动命令的，以及 WSL 里的 agent 只显示版本和原因，不代装、不代升。

## 装 `board-skills`

**跑着 `board host` 的机器不用手动装**：board host 按「Agent → skills」的 skills 共用库自动同步，`board-skills` 是库里固定的一项，内容随服务端版本，服务端升级后各台机器自动换成新的；`bin/<平台>/board` 用 board host 自己的 board。它写进每个找到的 agent 读用户级 skill 的目录（`~/.agents/skills`、`~/.claude/skills`、`$CODEX_HOME/skills`，WSL 里的 agent 写到 WSL 用户目录下），目录里放标记文件 `.board-sync`。`board-skills` 不管机器上 skills 选的是「用服务器的」还是「不改」都装。已经有同名、不是 Board 装的：挪进 `~/.board/host/skills-backup/<名称>-<时间>/` 再换上库里的；软链接（开发时链接到仓库的）不碰，界面上写「本机是软链接，没有覆盖」。自己的其他 skill 也可以放进库里，机器详情「设置 → skills 和 MCP」里 skills 选了「用服务器的」的机器都会装上：

```sh
board skills push ~/.claude/skills/my-skill     # 传进库里（目录名就是名称，合计不超过 5 MB），同名的换成新版本
board skills ls                                 # 库里有哪些、本机各目录的状态
board skills rm my-skill                        # 从库里删掉，各台机器上同步装的那份随后删掉
```

下面是不跑 board host 时手动装的办法。

agent 在哪个目录找 skill，就把 `board-skills` 放到哪里（上表「读 skill 的目录」）。除了用户级目录，agent 也会读工作目录（仓库根目录）里的 skill：dsh、Codex 读 `.agents/skills`，Claude Code 只读 `.claude/skills`、不读 `.agents/skills`。所以把 skill 放在仓库的 `.agents/skills/board-skills` 也可以，不用每台机器再装一遍；给 Claude Code 用时再在仓库里建一个 `.claude/skills/board-skills` 链接指向它。推荐解压发版附件 `board-skills.zip`（带各平台的 `board`）；在 Board 仓库里开发时用链接：

```sh
mkdir -p ~/.agents/skills ~/.claude/skills
ln -sfn "<Board 仓库>/.agents/skills/board-skills" ~/.agents/skills/board-skills   # dsh 等
ln -sfn "<Board 仓库>/.agents/skills/board-skills" ~/.claude/skills/board-skills   # Claude Code
```

`board host` 启动时在每个 agent 用的启动环境里检查它的 skill 目录（WSL 里看 WSL 用户目录下的，例如 `/home/<用户>/.agents/skills`），看不到 `board-skills` 会提醒一行。在 WSL 里跑的 agent 要在 WSL 里另装一份（或放在仓库里它会读的目录）。链接指向的目录不存在（例如仓库挪过位置）和没装一样。skill 由入口 `SKILL.md` 和按场景的分文件 `work.md`（处理卡片）、`plan.md`（拆解建卡、整理未整理）、`host.md`（board host 启动的会话）、`reference.md`（工作流、命令、错误码）组成，agent 读到哪一步再读对应的文件；这五个文件都在才算装好：

```sh
ls ~/.agents/skills/board-skills/{SKILL,work,plan,host,reference}.md
```

只复制 `SKILL.md` 不够（分文件缺了，agent 读不到那部分规则）：解压 `board-skills.zip` 或链接整个目录。

用 Board 桌面端让这台电脑接任务时，可以不手动装：board host 启动时就会装上。看不到时，「机器」里点开这台机器，agent 页里那个 agent 的卡片上写「缺 board-skills」，点「装上」让 board host 再同步一次（它把 skill 写进这些 agent 读 skill 的用户级目录：dsh 是 `~/.agents/skills`，Claude Code 是 `~/.claude/skills`，Codex 是 `$CODEX_HOME/skills`；已经是有效链接的不动）。WSL 里的 agent 仍按上面手动装。

## 按批次接入新的 agent

每一批几个 agent，每个都走完下面五步。前四步只靠配置文件和本机测试，不改代码；第五步把这一批验证过的写成内置模板，一起提交、发版。

### 1. 调研：每个 agent 填一张表

| 项 | 怎么找 |
|---|---|
| 标识、名称 | 标识用它的命令名（小写），名称用它自己的产品名 |
| ACP 入口 | 它的文档里「ACP」「Agent Client Protocol」「Zed」一节，或 [ACP 官方 agent 列表](https://agentclientprotocol.com/get-started/agents)、[Registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)：自带的启动参数（例如 `--acp`、`--profile acp`），或者维护中的适配器（npm 包名、固定一个版本）；适配器用哪个环境变量指定 agent 本体 |
| 不问权限 | ACP 会话公布的模式里有没有不审批的（`session/new` 回应的 `modes`、`configOptions`）；没有时看它用什么环境变量关掉沙箱和审批 |
| skill 目录 | 它的文档里「skills」一节；常见是 `~/.agents/skills` 或它自己的 `~/.<名字>/skills` |
| 登录 / 模型 | 它的文档；记下第一次用要做什么 |
| 安装位置 | `command -v <命令>`；装成 app 的再看 app 里有没有自带的命令行（macOS 在 `*.app/Contents/` 下找） |
| 版本 | `<命令> --version` |
| 用量 | prompt 的回应里带不带 `usage`、`usage_update` 里带不带 `cost`（「过程」里的 tokens、费用） |

### 2. 本机验证：不碰白板

在任意目录里，用 `board acp` 和第 1 步的 ACP 入口问它一句，确认它讲得通 ACP、找得到 `board-skills`、不问权限、问完就结束：

```sh
echo "/board-skills 不要执行任何命令。只回答两行：第一行是你加载的 skill 的 name，第二行是它 SKILL.md 里第一个二级标题。" > /tmp/q.txt
board acp --env direct --cwd /tmp --prompt-file /tmp/q.txt [--mode <不问权限的模式>] -- <ACP 入口命令> [参数…]
```

标准输出是一行一帧的 ACP 消息，最后一帧是 `"stopReason":"end_turn"`，退出码 0；它说的话在 `agent_message_chunk` 里。

应该回答 `board-skills` 和 `按场景读哪个文件`（SKILL.md 现在的第一个二级标题），然后退出。回答「没有这个 skill」：先按上一节装好，再问一次。

这一步可以直接在界面上做：第 3 步接上之后，在「机器」里点开那台机器，agent 页里这个 agent 的卡片上点「自检这一个」（或者「全部自检」，顺带检查每个项目的目录能不能推送）。board host 用它设置好的启动环境、它配置文件里的模型和思考强度，在一个临时目录里问同样的这一句，界面上写结果：通过；没登录；超时；回答里没有 `board-skills`。判的是它说的话（ACP 的消息片段接起来，思考和工具调用不算）：答出了 SKILL.md 第一个二级标题，或者答出了名字（不看大小写、连字符、空白这些写法）又没在说找不到它，就算通过。人不用登录到那台机器上。

### 3. 在界面上接上

写那台机器的 `~/.board/config.toml`，`cmd` 是 ACP 入口命令（提示词走 `session/prompt`，命令里不写 `{prompt}`）：

```toml
[agents.<标识>]
name = "<名称>"
cmd = ["<ACP 入口命令>", "<参数>"]
model = "<它实际用的模型名>"
```

`cmd` 的第一个是在每个启动环境里找的命令（取版本号也用它）。以前一次性命令的写法（带 `{prompt}`）已经不能用：新加的 agent 这样写时派发报错说明；同标识覆盖内置模板时不用它，照内置的 ACP 入口跑。覆盖内置模板时第一个是 `npx`（换适配器的版本）或用了 `{bin}` 时，照旧找内置的 agent 命令并交给适配器；否则第一个就是要找的命令，Board 不再代装、代升它。

`model` 写进派发的 agent 名字（`<机器名>/<模型名>-<强度>`，PRD §3.2）：内置 agent 先用它自己配置文件里现在的模型，读不到时才用这里的；都没有时用 agent 标识。转成小写、空白换成 `-`。模型、强度不按次传参数：以前的 `model_args`、`effort_args`、`efforts` 不再用，写了也不起作用；要固定模型，直接写进 `cmd`。

改了配置文件后，在界面「机器」里点开这台机器，点「维护」里的「重新检查」（或重启 `board host`），启动输出和界面「机器」里应该出现它，带版本号；机器详情 agent 页里它的启动环境下拉能看到它在每个启动环境里找没找到。同标识的 `[agents.*]` 也用来临时改内置模板（例如换参数、换路径、换模型）。

### 4. 端到端验证：用本机服务端，不碰线上

```sh
# 一个带远端的空仓库：AGENTS.md 写明集成分支 main 和推送前检查
git init --bare -b main /tmp/e2e/origin.git && git clone /tmp/e2e/origin.git /tmp/e2e/repo
# …在 repo 里写 AGENTS.md、一个检查脚本，提交并推到 origin…

board-server --local --port 7478 --db /tmp/e2e/board.db
# 新建令牌：curl -X POST http://127.0.0.1:7478/api/tokens -H 'content-type: application/json' -H 'x-board-client: e2e' -d '{"name":"e2e"}'
export BOARD_URL=http://127.0.0.1:7478 BOARD_TOKEN=<令牌>
board host --name e2e --workspace /tmp/e2e
# 另一个终端：建一张小卡（例如「新建 notes/<标识>.txt，内容是一行 hello」），派给它
BOARD=default/main board dispatch 1 --host e2e --agent <标识>
```

通过的标准：派发结果「已交审」；`origin/main` 上多了一个提交，最后一行是 `卡片：default/main#1`；卡片动态里有 agent 的交审评论（`提交：`、`集成分支：`、`检查：`、`碰到的别的卡：` 四行）；`~/.board/host/logs/<派发 ID>.log` 里能看到它的过程。再试一次「停止」：10 秒内进程结束，卡片移到「需人工校验」。

集成分支：先在项目设置里把它设成 `dev` 再派一张，通过的标志是 `origin/dev` 上多了提交（`main` 不动），交审评论里写着 `集成分支：dev`；把它清空后再派一张，回到读仓库 AGENTS.md 的规则。

**每个支持的启动环境各跑一次**：在机器详情 agent 页里把它的启动环境依次换成每个支持的环境（Windows 上的 PowerShell、Git Bash、WSL …；macOS、Linux 上的登录 shell 和直接启动），各派一张卡，结果都要「已交审」，派发记录写的是那个环境；在 WSL 里再试一次「停止」，WSL 里的 agent 进程也要在 10 秒内结束（`wsl.exe -d <发行版> -- ps -ef` 看）。

### 5. 写成内置模板

这一批都通过第 4 步后：

- `crates/board-core/src/agentdef.rs` 的 `AGENT_DEFS`：加一条，内置 agent 的描述只在这里写一份——`id`、`name`、npm 包和 dist-tag、`install`（安装命令）、`bin`（agent 命令）、`acp`（ACP 入口，自带 ACP 的写 `{bin} …`，用适配器的写 `npx -y <包>@<版本>`）、`acp_bin_env`（告诉适配器 agent 在哪的环境变量）、`acp_env`、`acp_mode`（不问权限的环境变量、模式）、`acp_since`（自带 ACP 从哪个版本起）、`also`（常见安装位置）、`skills`（读用户级 skill 的目录，第一个是装 board-skills、同步 skills 的地方）、`project`（仓库里读 skill 的目录）、`git_bash`、`win`（Windows 上支持的启动环境，按默认选择的顺序；第 4 步没跑通的环境不写）、`protocols`（认哪些供应商接口）、`efforts`（角色里能选的强度档位，从低到高）、`mcp`（支持 MCP 同步时它的 MCP 配置文件；不支持写 `None`）、`color`（界面上的主色）。放在哪一位就是默认顺序里的哪一位。board host 的内置模板、供应商的接口和强度（界面从 `GET /api/providers` 的 `meta` 拿）、MCP 同步名单、`board skills` 和桌面端「一键安装」的 skill 目录、桌面端「从本机导入 MCP」都从这里读，不用再改别处的名单。界面里只有 `web/src/lib/agentMeta.ts` 的兜底（连旧服务端、取到之前用）要跟着加一条。
- 要支持按角色换供应商、读回本机配置里的模型：在 `crates/board-cli/src/providersync.rs` 的 `route_for` 里加上这个 agent 的换法（ACP `providers/set` 或这次派发的补丁），在 `files_of` / `current` 里加上读回模型的那一段。
- 标了 `mcp`：在 `crates/board-cli/src/mcpsync.rs` 的 `implemented`、`put`、`remove` 里加上它的写法（`every_mcp_agent_in_the_description_has_a_writer` 会检查）。
- 测试：`agentdef.rs` 的 `every_builtin_agent_is_well_formed`（标识合法、接口和强度认得）、`crates/board-cli/src/host/tests.rs` 的 `builtin_order_and_overrides`（改期望的顺序）。
- 不用按 agent 写日志解析：ACP 的转写（`logfmt.rs`）、事件（`events.rs`）所有 agent 共用。它的帧里有现在没认出的写法（工具的 `kind`、`rawOutput` 的字段）时，在那里补上，按它的真实输出补一份样例进 `crates/board-cli/tests/fixtures/`。
- 界面上的图标：用它自己的图标（应用图标或官网 logo），放进 `web/src/assets/agents/<标识>.png`（裁掉透明边，64×64）或 `.svg`，在 `web/src/lib/hosts.ts` 的 `MARKS` 里引用（主色在 `agentdef.rs` 的 `color`，并发位顶边用）；白底的图标标 `light: true`。没有图标时显示标识的前两个字母。
- `docs/PRD.md` §9.1 的内置模板表，和本文「已经接入的」表。
- 跑完 `npm test`，提交 `feat(cli): board host 内置 <agent> 模板`；需要在别的机器上试时请人发版。

## 出了问题看哪里

先在「机器」里点开那台机器点「自检」：它直接告诉你 agent 有没有登录、认不认得 `board-skills`、每个供应商在这台机器上连不连得上、白板目录能不能推送、board 版本是不是落后，不用拿真卡片去试。没登录、连不上供应商的结果会算进「能不能跑」：用到它们的角色在这台机器上标成跑不了、不会自动派过来，登录或修好后重新自检就恢复。

| 现象 | 多半是 |
|---|---|
| 派发结果「异常退出（退出码 0）」，日志里 agent 说不认识 `/board-skills` | 没装 `board-skills`，或链接指向的目录不存在 |
| 日志里是「agent 要求先登录」「Authentication required」「Not logged in」 | 这台机器上的 agent 还没登录（Claude Code 的登录在系统钥匙串里：从别的 agent 会话、沙箱里启动的 board host 读不到） |
| 「机器」里这个 agent 显示「没找到」 | 在它用的启动环境里找不到命令：看机器详情的启动环境下拉，换一个找得到的环境；或者在那个环境的启动脚本（`~/.zshrc`、`~/.bashrc`、PowerShell 的 `$PROFILE`）里把它加进 `PATH`；在配置文件里写绝对路径也行 |
| 「机器」里这个 agent 显示「缺 Node.js（npx）」 | agent 装好了，但这个启动环境里找不到 `npx`：Claude Code、Codex 的 ACP 适配器要用它跑。装上 Node.js（或把它加进这个环境的 `PATH`），再在机器详情「设置」里点「重新检查」；也可以换一个找得到 `npx` 的启动环境 |
| WSL 里显示「没找到」，可 WSL 的终端里能运行 | 装在 `/mnt/` 下的是 Windows 程序，不算；在 WSL 里另装一份 Linux 版 |
| 在 WSL 里派发，日志里 `board` 报找不到命令或连不上 | WSL 里没装 `board-skills`（它自带 Linux 版 `board`），或者 WSL 里的 skill 太旧 |
| 日志开头有 `bash: cannot set terminal process group`、`no job control` | 交互式 shell 没有终端时的提示，不影响 |
| 一直在跑、日志不动 | agent 卡在一个很长的工具调用里，或者在等审批：ACP 的权限请求 board acp 会自动允许，看它是不是在等别的输入（例如 MCP 服务器的登录）；到「没输出多久算卡住」的时间 board host 会停止它 |
| 日志第一行之后就停在 `initialize`，最后报「agent … 秒内没有完成握手」 | ACP 入口没起来：用适配器的看日志里 `npx` 的报错（下载不了、没装 Node.js）；dsh 看是不是太旧（没有 acp profile） |
| 日志里是 `npm error code EEXIST`、`EACCES` | 用户的 npm 缓存权限坏了（以前用 sudo 跑过 npm）：board host 给 npx 用自己的缓存 `~/.board/host/npm-cache`，升级 board；自己执行 npx 时按 npm 的提示修 `~/.npm` 的属主 |
| 派发报「配置文件里 [agents.<标识>] cmd 是旧写法」 | 配置文件里新加的 agent 还是以前一次性命令的写法（带 `{prompt}`）：改成 ACP 入口命令，见「在界面上接上」 |
| 日志里 `board` 报 `INVALID_TOKEN`，或者连到了别的服务端 | agent 没把 `BOARD_CONFIG` 传给它执行的命令（或者它的 shell 启动脚本里又设了 `BOARD_URL`、`BOARD_TOKEN`）：在它的设置里放行这几个变量。也可能是 skill 太旧：旧版 `scripts/board` 不认 `BOARD_EXE`，用的是 skill 里打包的旧 `board`，更新 skill |
| 桌面端「这台电脑」开始接任务后马上停了，输出里是「未知命令「host」」 | 桌面端旁边的 `board` 是旧版：重新编译它（`node scripts/build-sidecar.mjs --debug`，`npm run desktop:dev` 会先做这一步） |
| 从设置了 `BOARD_TOKEN` 的终端或 agent 会话里启动桌面端，打开「让这台电脑接任务」后 board host 以 `INVALID_TOKEN` 停止 | 旧版桌面端把启动环境里的 `BOARD_URL`、`BOARD_TOKEN`（还有 `BOARD_CONFIG`、`BOARD_AS`、`BOARD`）继承给了后台的 board host，环境变量优先于配置文件，令牌是别的服务端的。新版启动 board host 和改机器名时都去掉这些变量，只用配置文件；升级桌面端 |
| Windows 上 `npm run desktop:dev` 报 `failed to remove file ... target\debug\board.exe`（os error 5：拒绝访问） | 上次开发启动的 `board-desktop.exe` / `board host` 还在从 `target\debug\board.exe` 运行，锁住了文件。新版 `scripts/build-sidecar.mjs` 会在 dev 构建前自动结束本仓库这两个进程；如果还失败，先从托盘退出 Board，再运行 `npm run desktop:dev` |
| 卡片动态里 agent 又 check 了一个新名字、领取失败 | 用的 `board-skills` 是旧版（没有「由 board host 启动的会话」一节），更新 skill |
