# Agent 接入与 ACP 调研

> 核实日期：2026-11-16（Asia/Shanghai）。
> 范围：调研 ACP 的规范、生态和 Board 现有接入方式，给出结论与后续任务；本文不改代码。
>
> 2026-11-17 补记：pi 已经不再是内置 agent（内置只剩 dsh、claude、codex），下文里 pi 的部分只作为当时的调研记录保留。
>
> 2026-11-17 已切换：人决定不走下文「部分换」的路线，board host 只用 ACP 启动 agent，旧的「命令模板 + 各家 JSONL」删掉了。
> dsh 用它自带的 `--profile acp`（下文写它没有 ACP 是当时没查到），Claude Code、Codex 用 ACP 组织维护的适配器（`npx -y` 固定版本）。
> 现在怎么做见 [agents.md](agents.md)「board host 怎么启动 agent」「已经接入的」；下文是当时的调研，结论和路线不再适用。
>
> 2026-11-25 补记（#667）：身份用 ACP 的 `auth/status`（Authentication State Query RFD）和适配器的 `authStatus` 扩展（`_auth/status_update`）问。claude-agent-acp 0.107.3 两种都有（`capabilities.auth.status`、`capabilities._meta.authStatus`），回的账号在 `_meta.claudeCode` 里带 `kind`（`account` / `api_key` / `gateway` / `external`）和 `plan`；codex-acp 2.7.0 只推 `_auth/status_update`（`capabilities.auth._meta.authStatus`），握手后读一次账号就推。本机（macOS）用 `board acp --auth only` 实测，只握手、不开会话：
>
> | agent | 情况 | 报的身份 |
> |---|---|---|
> | Claude Code | `/login` 登了 Max | `{"kind":"account","label":"Claude Max","plan":"max"}` |
> | Claude Code | 同上，环境里另有 `ANTHROPIC_API_KEY` | `{"kind":"api_key","label":"Anthropic API key","detail":"ANTHROPIC_API_KEY"}` |
> | Codex | `config.toml` 的 `model_provider` 指向自己配的中转 | `{"kind":"gateway","label":"Custom model gateway","detail":"<中转的名字>"}` |
> | Codex | `auth.json` 里是 API key | `{"kind":"api_key","label":"OpenAI API key"}` |
> | Codex | 没登录（空的 `CODEX_HOME`） | `{"kind":"none","label":"Not logged in"}` |
>
> ChatGPT 登录这台机器上没有（Codex 用的是中转），没实测；按 codex-acp 源码是 `{"kind":"account","label":"ChatGPT <套餐>","account":{"plan":…}}`。另外：只带 `HOME`、`PATH` 启动时（没有 `USER`、`LOGNAME`）macOS 上的 Claude Code 读不到钥匙串里的登录，报没登录；board host 带着用户的完整环境，不受影响。
>
> 2026-11-25 补记（#666）：codex-acp 升到 2.12.0（`session/options` 从 2.12 起有）。2.12.0 也公布了 `auth.status`，但回的账号里没有种类（只有 label、description），种类还在 `_auth/status_update` 里：`board acp` 回应看不出种类时接着等推送（上表在 2.12.0 上重跑，结果一样）。模型表用 `session/options` + 不发提示词的会话逐个切模型取，本机实测：
>
> | agent | 默认 | 模型（强度） |
> |---|---|---|
> | Claude Code（Max） | `opus` · `high` | 13 个：`opus`、`fable`、`sonnet`、`haiku` 这几个别名和完整 id；多数是 low / medium / high / xhigh / max，`claude-opus-4-6`、`claude-sonnet-4-6` 没有 xhigh，`claude-haiku-4-5-20251001` 没有强度这一项 |
> | Codex（中转） | `gpt-5.4` · 无 | 12 个，多数 low…max 再加 ultra；`gpt-5.4` 没有强度 |
>
> 两家的模型下拉里都可能有 `default`（「agent 自己的默认」），强度里 Claude 也有 `default`：都不算。`session/new` 的 `configOptions` 初始值实测：设得上的回应里就是要的值；账号里没有的模型、模型不支持的强度被忽略（回应里是默认值，或者没有强度这一项），`board acp` 据此出错结束。
>
> 2026-11-25 补记（#668）：订阅账号的额度用 ACP 上顺带来的信息认——claude-agent-acp 0.107.3 的 `usage_update` 在 `_meta["_claude/rateLimit"]` 带 Agent SDK 的 `rate_limit_info`，额度用完时这一轮的错误 `data` 是 `{errorKind: "rate_limit", rateLimit}`；codex-acp 2.7.0 不发额度，用完时错误 `data.codexErrorInfo` 是 `usageLimitExceeded`、重置时间只在文字里。`board acp` 认出时以退出码 4 结束、把额度写进 `--quota-file`，服务端记「额度用完」、不算异常退出，见 [agents.md](agents.md) 和 PRD §9.1「额度用完」。

## 结论

**部分换：把 ACP 作为 `board host` 新增的可选传输层，先接 Codex，再视结果接 Claude；Pi 继续验证，DeepSeek Harness 保持现有方式。不要现在改成 ACP-only，也不要用 v2 草案。**

理由：

- ACP 的稳定版已经足够成熟：稳定协议版本是 `1`，官方 Rust SDK 已到 1.0，当前 schema 和 Rust crate 在 2026-11-14 还在发版；Codex、Claude Agent、pi 都有可用的 ACP 适配器（[ACP 官方概览][acp-overview]、[协议仓库][acp-repo]）。
- ACP 正好覆盖 Board 现在最零碎的一段：每个 agent 自己的 JSONL 事件格式、工具调用、流式输出和取消。改成 ACP 后，“过程”和工具调用只解析一套协议，不再按四个 agent 各写一套解析器。
- 但 ACP 不会让“接一个新 agent”变成零成本。每个 agent 仍然需要 ACP 适配器、安装/更新、登录、模型配置和版本兼容；当前四个内置 agent 中只有 Codex、Claude、pi 有可用适配器，DeepSeek Harness 不在 ACP 官方 agents/registry 清单里。
- Board 是无人值守的任务分发器，不是编辑器。ACP 的交互式权限批准、运行中插话和多客户端 session，在 Board 现在没有对应 UI，短期用不上；ACP v1 也没有标准化的运行中插话，这部分要等 v2 草案稳定。
- Board 已经能用一个 agent 进程连续做多张卡：自动领取会一次排好一个会话的几张卡，agent 用 `board next` 接着做，不是严格“一卡一进程”。ACP 的 session 复用不能直接当成新能力，主要价值在协议统一和进程生命周期更清楚。
- ACP 的标准事件不覆盖 Board 现在记录的全部信息：例如标准 `usage_update` 是上下文用量和累计费用，不是当前的输入/输出 token 两个数；工具退出码、最终回答、原始日志也要另外映射。全量替换会先丢掉一部分现有可观测性。

因此建议的边界是：

- **新增 ACP transport，不删除现有 `exec` 路径**；默认仍是现在的命令模板 + JSONL。
- **只接稳定 v1**，用 `initialize` 的 `protocolVersion: 1` 和能力协商；v2 只做兼容性跟踪，不进入生产。
- **先 Codex，后 Claude**。Codex 的适配器由 ACP 组织维护、更新活跃，能力覆盖也最接近 Board 现在需要的“无人值守 coding agent”；Claude 适配器也成熟，但登录和模型配置的影响面更大。
- **Pi 先做评估卡，不做默认路径**。`pi-acp` 仍是 MVP，且它的限制需要逐项确认。
- **DeepSeek Harness 不迁移**，直到有官方或社区稳定、可维护的 ACP 入口。

什么时候值得从“部分换”走到“更大范围替换”：

- DeepSeek Harness 有官方或长期维护的 ACP 适配器，并且进入 ACP Registry；
- ACP v2 正式稳定，Codex、Claude、Pi 的适配器都明确支持同一版 v2；
- Board 需要运行中插话、人工权限批准、多个客户端同时观察一个 session；
- 接入的 agent 数量继续增加，现有四种 JSONL 解析器的维护成本明显超过 ACP 适配器的维护成本；
- ACP 适配器能通过扩展稳定提供 Board 需要的 tokens、退出码和工具结果，不再出现可观测性倒退。

## ACP 是什么，现在到什么程度

### 规范与版本

ACP（Agent Client Protocol）标准化的是 **Client 和 coding agent 之间的交互**，类似 LSP 标准化编辑器和语言服务器。Client 不必是编辑器：官方文档明确说另一个 agent 也可以做 ACP Client；MCP 负责给 agent 提供工具和数据，ACP 负责驱动 agent，两者不是替代关系（[ACP 与其他协议][acp-related]）。

ACP 最早由 Zed 在 2025-08-27 发布，最初接入的是 Gemini CLI（[Zed 发布文章][zed-launch]）。现在规范、schema 和 SDK 在 `agentclientprotocol` GitHub 组织维护，也有公开的 RFD 流程和多个编辑器/agent 参与。

截至核实日：

- **稳定协议版本是 `1`**。`initialize` 里的 `protocolVersion` 是大版本整数，只有破坏性变更才升大版本；同一大版本内靠 capabilities 和 `_meta` 扩展（[ACP 初始化][acp-init]）。
- 最新稳定 schema 是 `schema-v1.32.0`，最新 Rust crate 是 `v1.18.0`，二者均发布于 2026-11-14；当前还有 `schema-v2.0.0-alpha.16` 草案（[协议仓库][acp-repo]、[v1.18.0 release][acp-v1-release]）。
- 官方 Rust 和 TypeScript SDK 在 2026-06-25 达到 1.0.0（[SDK 1.0 公告][acp-sdk]）。
- **v2 仍是 Draft**，2026-07-20 才发布第一版草案。它把 prompt 的请求/响应和整个工作生命周期解耦，更适合后台工作、队列和运行中插话，但官方明确要求同时保留 v1，且不要在生产默认开启（[v2 草案公告][acp-v2]）。

### 协议管哪些事

ACP 使用 JSON-RPC 2.0，消息是 request/response 和 notification 两类。默认且目前唯一稳定定义的传输是 **stdio**：Client 启动 agent 子进程，换行分隔的 JSON-RPC 消息走 stdin/stdout，agent 的诊断日志走 stderr；stdout 不能混入非 ACP 内容。HTTP/WebSocket 远程传输仍在 RFD 阶段（[ACP 传输][acp-transports]）。

核心流程：

1. `initialize`：协商协议版本、Client/Agent capabilities、认证方式和实现信息。
2. `session/new`：用工作目录和 MCP server 列表创建一个 session；一个 agent 进程可以同时承载多个 session。另有可选的 `session/load`、`session/resume`、`session/list`、`session/close`（[Session Setup][acp-session]）。
3. `session/prompt`：发送用户消息；agent 用 `session/update` 通知流式报告消息、思考、计划、工具调用、工具结果、notice、压缩和用量，最后用带 `stopReason` 的响应结束这一轮（[Prompt Turn][acp-prompt]）。
4. `session/request_permission`：agent 在执行工具前向 Client 请求用户批准，选项包括 allow once/always、reject once/always；Client 返回选中的选项或 cancelled（[Tool Calls][acp-tools]）。
5. `session/cancel`：Client 取消当前轮；agent 应尽快停止并返回 `cancelled`，Client 还要把待处理的权限请求回复成 cancelled（[Cancellation][acp-cancel]）。
6. 可选能力：文件系统 `fs/read_text_file`、`fs/write_text_file`，终端 `terminal/*`，结构化提问 `elicitation/*`，session 配置选项、模型配置、额外的 workspace root 等。Client 必须在 `initialize` 里声明自己支持哪些能力，agent 不能假设未声明的能力可用。

几个和 Board 直接相关的边界：

- **ACP v1 可以多轮对话，但没有标准化的“运行中插话”。** 一轮 `session/prompt` 进行中，Client 的标准动作是取消；Codex 适配器有 `_session/steering` 扩展，但这是适配器私有能力，不是所有 ACP agent 都有。v2 才把后台工作、状态更新和 prompt 生命周期解耦作为核心设计。
- **ACP 的权限请求不等于 Board 有审批 UI。** `board host` 如果无人值守，只能自动批准、自动拒绝或把请求变成失败；自动批准必须和现有的 full-access 策略绑定，不能只看 agent 给的 title/name。
- **ACP 的 usage 不是当前的 token 统计。** 标准 `usage_update` 给的是当前上下文 `used`/`size` 和可选累计费用；现有 Board 的结束事件记录 `tokensIn`、`tokensOut`、费用、退出码。迁移时要决定哪些字段保留、哪些用适配器扩展补齐、哪些暂时显示为空。
- **原始输出要重新定义。** 现在 Board 直接保存 stdout/stderr 的原文；ACP 的 stdout 是协议帧，给人看的“原始输出”只能从 `session/update` 重新渲染，或者只看适配器的 stderr。

### 生态支持

ACP 官方 agents 页和 clients 页已经列出大量实现；Registry 是官方维护的可安装列表，当前 registry 文件在 2026-11-16 仍可正常获取（[官方 Agents][acp-agents]、[官方 Clients][acp-clients]、[ACP Registry][acp-registry]）。

明确列出的客户端包括 Zed、JetBrains 系列、Neovim 的多个插件、Emacs 的 `agent-shell`、VS Code 兼容扩展、Sublime Text 插件等；明确列出的 agent 包括 Gemini CLI、Cursor、GitHub Copilot CLI（public preview）、Goose、OpenCode、Qwen Code、Kimi、Junie、Claude、Codex、Pi 等。**这是“官方清单列出了实现”，不等于每个都是原生 ACP，也不等于每个能力都齐全**；清单里既有原生 agent，也有 adapter/connector。

与 Board 当前四个内置 agent 的对应关系：

| 当前内置 | ACP 现状 | 一手依据 | 对 Board 的判断 |
|---|---|---|---|
| `codex` | 有官方 ACP 适配器 `codex-acp`，最新稳定版 `v2.6.0`（2026-11-08）。适配器启动 Codex App Server，再把 ACP 请求翻译成 Codex 操作；支持认证、模型/强度、权限/沙箱、工具调用、计划、终端输出、会话和运行中 steering 扩展。 | [codex-acp][codex-acp]、[release][codex-release] | **首选试点。** 适配器维护活跃，能力最贴近 Board；仍要验证它和 Board 自己安装的 Codex 版本兼容，必要时用 `CODEX_PATH` 固定二进制。 |
| `claude` | 有 `claude-agent-acp`，最新稳定版 `v0.107.2`（2026-11-16），支持工具权限、diff、计划、终端、MCP、session，且实验性支持 v2。 | [claude-agent-acp][claude-acp]、[release][claude-release] | **第二个接入。** 适配器成熟，但登录、已有 Claude CLI、API key 和模型配置的兼容面比 Codex 大，先做登录/自检再灰度。 |
| `pi` | 有社区 `pi-acp`，最新版 `v0.0.34`（2026-09-24），已进 Registry；README 自述是 MVP，明确没有 ACP 文件系统/终端委托，MCP server 目前只接收不接线。 | [pi-acp][pi-acp]、[release][pi-release] | **先评估，不默认开启。** Board 不依赖 ACP 的 fs/terminal，但 MVP 稳定性和 MCP 缺口要实测；不能因为它“有 ACP”就直接替换现有 `pi --mode json`。 |
| `dsh` | 截至核实日，ACP 官方 agents 页和 Registry 都没有 DeepSeek Harness；GitHub 上能看到多个社区 `dsh-acp` 实现，但不在官方清单，未逐个验证维护状态和安全模型。 | [官方 Agents][acp-agents]、[Registry 文件][acp-registry] | **保持现有 `dsh --profile headless --json`。** 等有官方或长期维护的适配器，再单独评估。 |

注意：`claude-agent-acp` 和 `codex-acp` 都在 `agentclientprotocol` 组织下，但仍然是一层独立适配器；适配器打包或依赖的 agent 版本、认证方式和扩展能力，不能等同于 Board 现在自己管理的 `claude`/`codex` 安装。

## Board 现在怎么接 agent

### 启动和输入

`board host` 的内置 agent 定义在 `crates/board-core/src/agentdef.rs` 的 `AGENT_DEFS`。四个模板都是一次性命令，`{prompt}` 在 `crates/board-cli/src/host/runs.rs` 的 `agent_args`/`spawn` 里作为一个参数传入；标准输入是 `Stdio::null()`，标准输出和标准错误写到 `~/.board/host/logs/<派发 ID>.log`。命令模板和启动环境见 `docs/agents.md`「board host 怎么启动 agent」「已经接入的（内置模板）」。

当前模板：

| agent | 当前命令模板 |
|---|---|
| dsh | `dsh --profile headless --patch {patch} --json --permission-mode danger-full-access {prompt}` |
| claude | `claude -p {prompt} --dangerously-skip-permissions --output-format stream-json --verbose` |
| pi | `pi --mode json {prompt}` |
| codex | `codex exec --json --dangerously-bypass-approvals-and-sandbox {prompt}` |

配置文件里的 `[agents.<标识>] cmd` 可以覆盖命令；命令没有打开结构化输出时，Board 只能保留原始日志，不能生成结构化“过程”。当前仓库 `crates/` 里搜不到 ACP/Agent Client Protocol 相关实现。

### 输出和过程

每种 agent 的 JSONL 由 `crates/board-cli/src/logfmt.rs` 和 `crates/board-cli/src/events.rs` 分别解析，统一成 `crates/board-core/src/runlog.rs` 的 `RunEvent`：`message`、`tool`、`error`、`result`、`text`。工具调用和结果用 agent 自己的 call id 合并；工具参数和结果只留摘要；每个派发最多保留 2000 条事件，单条 4 KB 截断。`logfmt::stream_for` 只识别四个内置标识和它们各自的事件流参数，配置文件里新增的普通命令没有结构化事件。

这套做法的直接结果：

- 每个 agent 的格式变化都要改 `logfmt.rs`、`events.rs` 和 fixture；
- 工具调用、流式片段、用量要不要显示，由 Board 对每个 JSONL 的理解决定；
- 原始输出和结构化“过程”来自同一份 stdout，排查时比较直接。

### 进程、会话和停止

- 手动派发一个会话只做一张卡；自动领取一次可能给一个 agent 进程排最多 `per_session` 张卡，agent 用 `board next` 在同一进程里接着做。服务端在 `crates/board-core/src/hosts/run.rs` 的 `plan_session`/`dispatch_next` 里安排，`board host` 用 `successor` 把后续派发映射到同一个 `Run`。
- 停止时 `Run::stop` 结束进程树：macOS/Linux 先发 SIGTERM，10 秒后升级为 SIGKILL；Windows 用进程树结束；WSL 通过 `BOARD_PIDFILE` 找到 WSL 内的进程组。`enforce_timeouts` 还会处理“运行超过 N 分钟”和“多久没有任何输出”。
- agent 退出后 `reap`/`finish_run` 读取退出码、最后日志和事件，并在异常退出、超时、人工停止时评论卡片。`shutdown` 在 `board host` 退出时结束所有在跑的 agent。
- 环境变量里带 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_AS`、`BOARD_HOST`、`BOARD` 和 `BOARD_CONFIG`；`BOARD_CONFIG` 是本次派发专用的配置文件，解决某些 agent 不传 `*_TOKEN` 环境变量的问题。登录和模型供应商仍由各 agent 自己的配置文件/登录状态负责，Board 只是写入或同步配置。

### 现状判断

Board 现在的接入方式**不是“每个 agent 完全独立”**：启动环境、配置、模型、MCP、skills、停止和派发生命周期已经统一；真正按 agent 分叉的是**命令模板和事件流解析**。这也决定了 ACP 的收益和边界：它能消掉事件流解析这一大块，但不会自动消掉安装、认证、配置和进程生命周期。

## 和 ACP 逐项对比

### 接入成本

| 项目 | Board 现在 | ACP | 判断 |
|---|---|---|---|
| 新 agent 的上手 | 在 `AGENT_DEFS` 加命令、安装方式、skill 目录、Windows 启动环境；再决定是否写事件解析。自定义 agent 只能看原始日志。 | 需要一个 ACP agent/adapter（可能是原生，也可能是包装器）；Board 实现一次 v1 Client 后，所有适配器共享协议层。 | **协议层成本下降，适配器成本仍在。** 对第 5、6 个 agent 的边际成本更低，对当前四个稳定 agent 的短期节省有限。 |
| 事件解析 | `logfmt.rs`/`events.rs` 分别解析 Claude、Codex、pi、dsh；格式变了就要改代码和 fixture。 | `session/update` 统一消息、工具、计划、权限、用量；Board 只写一套映射。 | **明显改善。** 这是 ACP 最确定的收益。 |
| 安装/更新 | Board 已有 npm/brew/官方脚本/桌面 app 的识别和更新逻辑，见 `docs/agents.md`「安装和更新 agent」。 | adapter 自己是另一个要安装、升级、诊断的二进制/包；它还可能捆绑自己的 agent 版本。 | **额外一层维护。** 不能把 ACP 当成“装一次协议就完事”。 |
| 版本兼容 | 命令模板固定，Board 自己控制 agent 版本。 | 协议版本、capabilities、adapter 版本和底层 agent 版本四者要兼容。 | **更复杂，但可用能力协商和固定版本控制。** |

### 能力

| 能力 | 现在 | ACP 能给什么 | 结论 |
|---|---|---|---|
| 流式消息 | 各 agent 的 JSONL 行，Board 转成 `message`。 | `session/update` 的 message/thought chunk，标准 Markdown/内容块。 | **改善**；但 Board 的 `RunEvent` 要决定聚合粒度，不能每块都写成一条 UI 事件。 |
| 结构化工具调用 | 从各 agent 的 JSONL 里推断工具名、参数、结果，退出码不一定有。 | `tool_call`/`tool_call_update`，含 toolCallId、title、name、kind、status、content、locations、rawInput/rawOutput。 | **明显改善**；退出码仍需 adapter 或扩展补齐。 |
| 计划、diff、文件位置 | 现在分散在各解析器，能显示多少取决于格式。 | plan、tool locations、diff content 等都是协议内容。 | **改善**，但要改“过程”UI 的展示模型。 |
| 权限请求 | full-access 直接跳过审批，没有人机审批通道。 | 标准 `session/request_permission`，可以把工具调用和选项交给 Client。 | **Board 现在用不上，反而增加策略判断。** 无人值守只能显式自动批准/拒绝。 |
| 多轮对话 | 一个一次性 prompt；连续卡片靠 `board next` 和同一进程。 | 同一 session 可以连续 `session/prompt`，进程可承载多个 session。 | **Board 已有顺序复用，短期收益主要是协议化，不是产品能力。** |
| 运行中插话 | 没有；stdin 为空。 | v1 没有标准化的运行中插话；v2 草案把后台工作和状态更新解耦；Codex 等适配器有私有 steering 扩展。 | **不能作为当前迁移理由。** 等 v2 稳定或按适配器扩展单独做。 |
| 取消 | 结束进程树，10 秒后强制杀；WSL/Windows 有专门处理。 | `session/cancel` 优雅取消，agent 应返回 `cancelled`；Client 还要取消待处理的权限请求。 | **改善优雅停止**；仍必须保留进程树强杀，adapter 可能卡住。 |
| 退出码/异常退出 | 直接读子进程退出码，结合最后的日志和事件评论卡片。 | prompt 返回 `stopReason`，但不等于进程退出码；adapter 崩溃、协议错误和底层 agent 失败要分别归类。 | **需要新的状态映射，不能直接照搬。** |
| 同一进程连续做卡 | 自动领取已经用 `board next` 在同一进程连续做，见 `plan_session`/`successor`。 | 一个 agent 进程可开多个 session，可 load/resume。 | **不是 ACP 独有的优势**；如果 Host 接管排卡，可以减少对 skill 循环的依赖，但这是另一项架构改动。 |
| 多个 session | 最多 N 个 agent 进程，一个进程内顺序做卡。 | 一个进程内多个独立 session。 | **可能减少进程启动开销**；但每个 session 的 cwd、日志、停止和卡片映射都要重新设计。 |
| MCP | Board 写各 agent 的用户级 MCP 配置；见 `docs/agents.md`「MCP 服务器」。 | `session/new` 直接传 `mcpServers`；stdio 必选，HTTP 可选。 | **长期可简化配置同步**，但只在 ACP 路径成立，非 ACP 路径仍要保留。 |
| 模型/强度 | Board 写各 agent 的供应商配置；命令不按次传模型参数。 | session config options、模型配置类别等可选能力。 | **可能更标准**，但适配器支持不一，短期不宜替换现有配置同步。 |
| token/费用 | Claude 有汇总；pi、Codex、dsh 按事件累加；字段能落到 `RunEvent.result`。 | `usage_update` 是上下文用量和累计费用，标准字段不等于现有 tokensIn/tokensOut。 | **可观测性有倒退风险。** 需要适配器扩展或 UI 口径调整。 |
| 文件/终端委托 | agent 自己做工具，Board 不提供文件/终端 API。 | 可选 fs/terminal 能力；Client 可按需声明不支持。 | **暂时不需要**；不声明这些 capability 可以缩小实现面。 |

### 代价

1. **`board host` 要常驻一个 ACP Client。** 需要处理 JSON-RPC 请求/响应、双向请求、进程重启、session 映射、超时和并发；不是“把命令参数换成另一个命令”这么小。
2. **协议输出和人类日志要分开。** ACP 的 stdout 只能有协议帧，原始日志只能走 stderr 或由事件重新渲染；现有 `~/.board/host/logs/<派发 ID>.log`、`post_events`、界面「原始输出」都要重新定义。
3. **停止和异常退出的状态机更大。** 当前 Board 已经有进程树、WSL PID、10 秒强杀和超时；ACP 还要加 `session/cancel`、等待 `cancelled`、权限请求取消、adapter 崩溃和协议错误。
4. **适配器是新增依赖。** Codex/Claude 适配器可以自带或指向已有 CLI，Pi 依赖外部 pi；Board 现在自己识别 agent 版本和更新路径，接入 ACP 后还要识别 adapter 版本和两者兼容矩阵。
5. **连续卡片的调度要重新设计。** 现在 skill 通过 `board next` 让同一个 agent 继续；如果 Host 接管，需要把服务端的会话排队和 ACP 的 session/prompt 结合起来，不能只改客户端。
6. **测试成本。** 需要至少一个假 ACP agent 做协议测试，再在 macOS、Windows、WSL、Git Bash 上做真实 adapter 端到端；现有四套 JSONL fixture 还要保留一段时间做回退验证。

### 风险

| 风险 | 说明 | 缓解 |
|---|---|---|
| adapter 质量不一 | Codex/Claude 适配器维护活跃，但 pi 仍是 MVP；适配器可能只实现部分 ACP 能力。 | 先 Codex；按 capability 做能力矩阵；不回退现有路径直到端到端通过。 |
| DeepSeek Harness 没有官方 ACP | 社区适配器可能有，但不在官方清单，维护和安全无法作为产品依赖。 | 保持 `dsh` 现有 JSONL 路径，单独跟踪。 |
| v1/v2 分叉 | v2 仍是 Draft，适配器可能同时支持两版；v2 的 prompt 生命周期和 v1 不同。 | 只发 `protocolVersion: 1`，版本协商失败就回退；v2 只做开发验证。 |
| 无人值守权限 | ACP 会要求 Client 对工具权限作出决定，Board 没有实时人工审批。 | 权限策略显式绑定“该 agent 当前是否 full-access”；无法判定就拒绝并失败派发，不能盲批。 |
| token/退出码缺失 | ACP 标准字段不覆盖 Board 当前全部统计。 | 保留适配器 `_meta`/原始日志；先在 UI 标注口径，再决定是否补协议扩展。 |
| 原始日志体验变化 | stdout 是协议帧，不能直接当原始输出展示。 | 从 `session/update` 渲染统一原始视图，同时保留 adapter stderr。 |
| Windows/WSL | 多一层 wrapper、进程树和 PID 处理，stdio/管道失败模式更多。 | 先 macOS 验证；Windows/WSL 作为独立卡验收，不把“能启动”当完成。 |
| 安全边界 | ACP 适配器可启动底层 agent，仍会继承 Board 的令牌和环境。 | 继续用 `BOARD_CONFIG` 的最小权限临时文件；adapter 进程清理、临时文件清理和命令注入测试不能省。 |

## 落地路线

### 阶段 1：Codex 的 ACP 可选 transport

目标：证明 ACP 能把 Codex 的事件、工具调用、取消和异常退出接进现有派发模型，不改变默认路径。

- 在 `board-cli` 引入官方 Rust SDK，先只实现 v1 Client；不引入 v2。
- 新增按 agent 的 `transport = "acp" | "exec"` 配置，默认 `exec`；先只允许 `codex` 使用 `acp`。
- 启动 `codex-acp`，完成 `initialize`、`session/new`、`session/prompt`；stdout 只走协议，stderr 写日志。
- 实现 `session/request_permission` 的显式策略；若无人工审批，就只在当前 agent 的 full-access 模式匹配时自动允许，否则失败并写清原因。
- 把 `session/update` 映射到现有 `RunEvent`；“过程”先保证消息、工具调用、工具结果、错误、结束五类等价，token/退出码缺失处明确标记。
- 保留现有 `codex exec --json` 作为同一条配置的回退，任何协议/启动错误都能人工切回。

验收：同一张 Codex 卡片在两条路径都能完成；停止按钮在 10 秒内结束且无残留进程；异常退出、超时、人工停止的卡片评论与现有路径一致；现有 JSONL 测试全过。

### 阶段 2：进程和会话生命周期

- 把 `Run` 的进程管理抽成 `exec`/`acp` 两种实现，共用停止、超时、日志清理和 `finish_run`。
- ACP 先按“一个派发一个 session”实现，避免把卡片边界和 session 上下文混在一起；之后再评估同一 session 连续 prompt。
- 增加 adapter 崩溃、协议错误、`session/cancel` 超时、权限请求未决的分类和评论。
- 在 macOS、Windows、WSL/Git Bash 上覆盖启动、停止、PID/进程树、临时 `BOARD_CONFIG` 清理。

验收：任何路径都不会留下 adapter 或底层 agent 进程；`board host` 退出时按现有规则上报 `host_exit`；停止后卡片不会继续被写；同一 agent 的 exec/acp 派发可以并存。

### 阶段 3：小范围灰度

- 灰度开关放在机器/agent 配置，先只允许手动派发，不进入自动领取。
- 记录 ACP 启动失败率、协议错误、停止耗时、事件缺失、token/费用缺失和卡片结果分布；与 exec 路径对比。
- 达到“没有更多 stuck/crashed、停止不退化、事件完整、费用/用量可解释”后，才允许自动领取的一台内部机器使用。
- 保留一键回退；回退只改 transport，不迁移卡片、不改服务端状态。

### 阶段 4：Claude，然后 Pi/DSH

- Claude 接入前先做登录/自检：确认 adapter 使用 Board 已安装的 Claude CLI 还是自带版本，API key/Claude Code 登录和模型供应商如何映射。
- Pi 只做评估卡：用固定版本 `pi-acp` 和若干真实卡片测工具、会话、取消、错误和 MCP；MVP 的限制写进结论，不直接进默认路径。
- DeepSeek Harness 跟踪官方/Registry 支持；没有稳定入口前不改。

### 暂不做

- 不把 `board next` 从 skill 迁到 Host。先保留现有连续做卡逻辑，ACP 只负责单张派发的协议和进程；Host 接管排卡要另立卡。
- 不默认使用 ACP 的 fs/terminal 委托。Board 的 agent 自己在 worktree 里读写和执行，声明不支持这些 Client 能力可以缩小安全面。
- 不在生产接 v2。等 v2 stable、至少 Codex/Claude/Pi 三个适配器都明确支持后再单独立项。

## 可直接拆成卡片的后续任务

| 卡片标题 | 做什么 | 验收 |
|---|---|---|
| `host: 增加 ACP v1 客户端骨架（Codex 实验）` | 引入官方 Rust SDK；增加 `transport` 配置；启动 `codex-acp`；实现 initialize/session/new/prompt/update 和权限回调；默认 `exec` 不变。 | 假 ACP agent 的单测覆盖握手、消息、工具、权限和服务端错误；一张手动 Codex 卡在 macOS 跑通；关闭开关仍走原路径。 |
| `host: ACP 派发进程停止、超时和异常退出` | 抽出 exec/acp 共用的进程管理；实现 `session/cancel` 后限时强杀；映射 stopReason、协议错误、进程退出码；处理 WSL/Git Bash 和临时文件。 | 停止 10 秒内完成且无残留；异常退出/超时/人工停止的评论与现有路径一致；macOS、Windows、WSL 各有测试或手工验收记录。 |
| `host: ACP 事件接入“过程”和“原始输出”` | 把 message/tool/plan/usage/error 映射到 `RunEvent`；决定 token/退出码缺失时的显示；从事件渲染原始输出，保留 adapter stderr。 | 同一张 Codex 卡在 exec/acp 两条路径的消息、工具调用数量一致；工具失败可见；超过 4 KB/2000 条仍按现有上限截断；UI 不重复显示。 |
| `host: Codex ACP 灰度与回退开关` | 增加机器/agent 级实验开关，先只允许手动派发；记录启动失败、协议错误、停止耗时、事件缺失和卡片结果；提供即时回退。 | 同一机器可同时跑 exec/acp 派发；回退不改服务端状态；灰度指标能按派发查询；默认关闭。 |
| `host: 接入 Claude ACP 适配器` | 在 Codex 路径稳定后支持 `claude-agent-acp`；处理已有 Claude CLI/API key/模型配置和登录自检；只发 v1。 | `claude-agent-acp` 的版本和登录状态在机器详情/自检里可见；一张需要使用工具和一张需要停止的卡跑通；失败时回退 exec 并保留原因。 |
| `host: 评估 Pi ACP 适配器` | 固定 `pi-acp` 版本做工具、会话、取消、错误、MCP 和重启测试；对比现有 `pi --mode json`。 | 输出能力矩阵和 go/no-go 结论；不改变默认路径；明确列出 MVP 限制对 Board 的影响。 |
| `host: 评估 Host 接管连续卡片调度（可选）` | 研究由 Host 在 ACP session 之间调用服务端下一张，替代 skill 里的 `board next`；保留旧路径。 | 给出协议/服务端改动清单和失败回滚方案；至少验证同一进程连续两张卡；不先改默认行为。 |

## 需要人拍板的取舍

1. **是否接受双路径维护。** ACP 和现有 JSONL 会并行一段时间；这是为了可控回退，但会让测试矩阵更大。
2. **无人值守时权限怎么批。** 推荐“只在明确 full-access 模式下自动允许，无法判定就失败”，不要按 agent 自报的 title/name 盲批。这需要产品和安全一起确认。
3. **先投入 Codex 试点还是继续补现有解析器。** ACP 的确定收益是统一事件和工具调用；如果近期更重要的是新增功能而不是接入规范，可以把试点排在后面，但不能在没试点时直接全量迁移。
4. **Pi 和 DSH 的优先级。** Pi 有 MVP 适配器但没有完整能力；DSH 没有官方 ACP。两者都不适合作为第一批 ACP 生产路径。
5. **v2 的观察窗口。** 现在只跟踪，不排生产任务；等 v2 稳定后再决定是否把 Host 的 prompt 生命周期改成后台任务模型。

## 引用和核实来源

以下来源均在 **2026-11-16** 核实；发布日期、版本和页面内容以当时为准。

### ACP 官方规范与发布

1. [ACP Introduction](https://agentclientprotocol.com/get-started/introduction.md)（官方介绍：本地/远程、stdio、JSON-RPC 2.0）
2. [ACP Architecture](https://agentclientprotocol.com/get-started/architecture.md)（Client/Agent、MCP、streaming、permission）
3. [ACP and other protocols](https://agentclientprotocol.com/get-started/related-protocols.md)（ACP 与 MCP/A2A 的区别；另一个 agent 也可以做 Client）
4. [ACP v1 Overview](https://agentclientprotocol.com/protocol/v1/overview.md)（v1 方法、notification、错误和能力）
5. [ACP Initialization](https://agentclientprotocol.com/protocol/v1/initialization.md)（protocolVersion、capabilities、auth、implementation info）
6. [ACP Session Setup](https://agentclientprotocol.com/protocol/v1/session-setup.md)（session/new、load、resume、close、cwd、MCP servers）
7. [ACP Prompt Turn](https://agentclientprotocol.com/protocol/v1/prompt-turn.md)（prompt、session/update、usage、stopReason、cancellation）
8. [ACP Tool Calls](https://agentclientprotocol.com/protocol/v1/tool-calls.md)（tool_call、tool_call_update、permission）
9. [ACP Cancellation](https://agentclientprotocol.com/protocol/v1/cancellation.md)（$/cancel_request、session/cancel）
10. [ACP Transports](https://agentclientprotocol.com/protocol/v1/transports.md)（stdio 是当前稳定传输；HTTP/WebSocket 仍在 RFD）
11. [ACP v2 Draft](https://agentclientprotocol.com/announcements/acp-v2-draft.md)（2026-07-20；v2 仍是 Draft，要求同时支持 v1）
12. [Rust and TypeScript SDKs reach 1.0](https://agentclientprotocol.com/announcements/sdk-1-0-releases.md)（2026-06-25）
13. [Official Agents](https://agentclientprotocol.com/get-started/agents.md)（当前 agents 清单）
14. [Official Clients](https://agentclientprotocol.com/get-started/clients.md)（当前 clients 清单）
15. [ACP Registry](https://agentclientprotocol.com/get-started/registry.md) 和 [registry JSON](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)（当前可安装 agents）

### ACP 仓库、SDK 和当前版本

16. [agentclientprotocol/agent-client-protocol](https://github.com/agentclientprotocol/agent-client-protocol)（README 明确当前稳定协议版本为 `1`）
17. [Rust crate v1.18.0](https://github.com/agentclientprotocol/agent-client-protocol/releases/tag/v1.18.0)（2026-11-14）
18. [Schema v1.32.0](https://github.com/agentclientprotocol/agent-client-protocol/releases/tag/schema-v1.32.0)（2026-11-14）
19. [Schema v2.0.0-alpha.16](https://github.com/agentclientprotocol/agent-client-protocol/releases/tag/schema-v2.0.0-alpha.16)（2026-11-14，草案）
20. [Official Rust SDK](https://github.com/agentclientprotocol/rust-sdk)（roles、connection builders、v1/v2 API）
21. [agent-client-protocol on crates.io](https://crates.io/crates/agent-client-protocol)（当前 Rust SDK）

### 当前四个 agent 的适配器

22. [codex-acp README](https://github.com/agentclientprotocol/codex-acp)（stdio ACP server、Codex App Server、能力）
23. [codex-acp v2.6.0](https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.6.0)（2026-11-08）
24. [claude-agent-acp README](https://github.com/agentclientprotocol/claude-agent-acp)（Claude Agent SDK、工具/权限、v2 实验）
25. [claude-agent-acp v0.107.2](https://github.com/agentclientprotocol/claude-agent-acp/releases/tag/v0.107.2)（2026-11-16）
26. [pi-acp README](https://github.com/svkozak/pi-acp)（MVP、限制、能力）
27. [pi-acp v0.0.34](https://github.com/svkozak/pi-acp/releases/tag/v0.0.34)（2026-09-24）

### Zed 的一手发布材料

28. [Bring Your Own Agent to Zed — Featuring Gemini CLI](https://zed.dev/blog/bring-your-own-agent-to-zed)（2025-08-27，ACP 发布）
29. [Claude Code: Now in Beta in Zed](https://zed.dev/blog/claude-code-via-acp)（2025-09-03）
30. [Zed External Agents 文档](https://zed.dev/docs/ai/external-agents)（当前 ACP External Agents 和 Registry 用法）

### Board 仓库依据

31. `crates/board-core/src/agentdef.rs`：`AGENT_DEFS`、四个内置命令、安装/更新、skills、MCP 和模型相关元数据。
32. `crates/board-cli/src/host/`（`runs.rs`）：`spawn`、`agent_args`、`dispatch_prompt`、`Run::stop`、`reap`、`finish_run`、`enforce_timeouts`、`shutdown`。
33. `crates/board-cli/src/logfmt.rs`、`crates/board-cli/src/events.rs`：四种 JSONL 的转写和结构化事件解析；`stream_for` 的识别范围。
34. `crates/board-core/src/runlog.rs`：`RunEvent` 字段、事件保留上限和界面展示模型。
35. `crates/board-core/src/hosts/`（`run.rs`）：`plan_session`、`dispatch_next`、`successor`、派发结果和会话状态。
36. `docs/agents.md`：Board 的启动环境、结构化输出、安装更新、MCP、模型供应商和内置模板说明。

[acp-overview]: https://agentclientprotocol.com/protocol/v1/overview.md
[acp-repo]: https://github.com/agentclientprotocol/agent-client-protocol
[acp-related]: https://agentclientprotocol.com/get-started/related-protocols.md
[acp-init]: https://agentclientprotocol.com/protocol/v1/initialization.md
[acp-session]: https://agentclientprotocol.com/protocol/v1/session-setup.md
[acp-prompt]: https://agentclientprotocol.com/protocol/v1/prompt-turn.md
[acp-tools]: https://agentclientprotocol.com/protocol/v1/tool-calls.md
[acp-cancel]: https://agentclientprotocol.com/protocol/v1/cancellation.md
[acp-transports]: https://agentclientprotocol.com/protocol/v1/transports.md
[acp-v2]: https://agentclientprotocol.com/announcements/acp-v2-draft.md
[acp-sdk]: https://agentclientprotocol.com/announcements/sdk-1-0-releases.md
[acp-agents]: https://agentclientprotocol.com/get-started/agents.md
[acp-clients]: https://agentclientprotocol.com/get-started/clients.md
[acp-registry]: https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json
[acp-v1-release]: https://github.com/agentclientprotocol/agent-client-protocol/releases/tag/v1.18.0
[codex-acp]: https://github.com/agentclientprotocol/codex-acp
[codex-release]: https://github.com/agentclientprotocol/codex-acp/releases/tag/v2.6.0
[claude-acp]: https://github.com/agentclientprotocol/claude-agent-acp
[claude-release]: https://github.com/agentclientprotocol/claude-agent-acp/releases/tag/v0.107.2
[pi-acp]: https://github.com/svkozak/pi-acp
[pi-release]: https://github.com/svkozak/pi-acp/releases/tag/v0.0.34
[zed-launch]: https://zed.dev/blog/bring-your-own-agent-to-zed
