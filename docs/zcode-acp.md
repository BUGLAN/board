# ZCode 经 ACP 接入：调研与实测

> 核实日期：2026-11-28。
> 范围：ZCode 能不能用 ACP 驱动、桌面端和 CLI 的区别、用 `board acp` 在本机实测的结果，以及接进 board host 要做的事。本文不改代码。
> Board 现在怎么用 ACP 启动 agent 见 [agents.md](agents.md)，ACP 本身的调研见 [acp.md](acp.md)。

## 结论

- **ZCode 官方没有 ACP。** 桌面端和它自带的 CLI 都不讲 ACP（安装包里搜不到 `agentclientprotocol`、`session/prompt`、`session/update`），[ACP Registry][acp-registry] 里也没有 ZCode。
- **能用社区适配器 `zcode-acp-server` 接。** 它启动 ZCode 自带 CLI 的 `zcode app-server --stdio`（ZCode 私有的 JSON-RPC 协议），翻成 ACP v1。
- **本机实测跑通。** `board acp` + `zcode-acp-server` 0.97.0 + ZCode 3.16.3：握手、开会话、切模型和思考强度、调工具写文件、`board-skills` 自检都通过，退出码 0。用的模型是 ZCode 体验套餐（Start Plan）的 GLM-5.3-Flash。
- **接进 board host 前要补的：**
  - 模型调用失败的一轮被报成 `end_turn`，Board 会当成正常结束。
  - Board 认不出 ZCode 的登录身份，也查不了额度。
  - Start Plan 的模型用 `--set` 选不上，而 Board 对 v1 agent 选不上模型时会悄悄按默认模型跑。
  - 机器上必须装着桌面端并登录过。

  详见下面「接进 Board 要做的事」。

## ZCode 的形态

ZCode 是智谱（Z.ai）出的「智能体开发环境」桌面应用。官方只发桌面安装包：macOS、Windows，Linux 有 AppImage、deb、rpm 三种。CLI 不单独发，打包在桌面端里。核实时最新版是 4.0.0（2026-11-23），版本发得很勤：10-12 到 11-23 之间发了 8 个版本。

| | 桌面端（ZCode.app） | 自带 CLI（`zcode.cjs`） |
|---|---|---|
| 在哪 | 从官网下载安装 | macOS：`/Applications/ZCode.app/Contents/Resources/glm/zcode.cjs`，不加进 `PATH`。下面两个是适配器 README 写的，没有实测：Windows `%LOCALAPPDATA%\Programs\ZCode\resources\glm\zcode.cjs`；Linux deb/rpm `/opt/ZCode/resources/glm/zcode.cjs`（Linux 上 `PATH` 里的 `zcode` 是桌面端启动器，不是 CLI） |
| 实测版本 | 3.16.3 | 0.17.0（随桌面端） |
| 两者关系 | 桌面端用自己的可执行文件，带 `ELECTRON_RUN_AS_NODE=1` 运行 `zcode.cjs app-server --stdio`，通过 ZCode Protocol 驱动 agent | 和桌面端是同一个 agent（ZCode Agent） |
| 无人值守入口 | 没有：只有界面、Bot Channel、手机远程控制、SSH / WSL / Docker 远程开发 | 全屏 TUI（默认）；`-p <提示词>` 跑一次就退出（默认 `--mode yolo`）；`--json`、`--resume <sessionId>`、`-c`；`app-server --stdio` |
| ACP | 没有：既不能当 ACP client 驱动别的 agent，也不能当 ACP agent 被驱动 | 没有 `acp` 子命令 |
| 配置 | 两边共用 `~/.zcode/` | 同左 |

`~/.zcode/` 里几个要紧的文件：

- `cli/config.json`：MCP、权限默认值、插件、Hooks。
- `v2/provider_config.json`：模型供应商。
- `v2/setting.json`：桌面端设置。
- `v2/credentials.json`：登录凭据，按设备加密，拷到别的机器上解不开。

`app-server` 讲的是 ZCode 自己的 JSON-RPC 协议，CLI 帮助里叫「ZCode Protocol」，没有公开文档。它的方法有 `session/create`、`session/send`、`session/event`、`session/setModel`、`session/setThoughtLevel`、`session/setMode`、`session/stop` 等，和 ACP 的 `session/new`、`session/prompt`、`session/update` 不兼容。

**自带 CLI 不能单独当 agent 用。** 直接跑 `zcode.cjs -p` 会先报找不到内置供应商配置；用 `ZCODE_BUILTIN_PROVIDER_CONFIG_FILE` 指向 `Resources/config/provider/zcode-builtin.json` 之后，又报「Select a model before sending a message」。账号里的供应商和模型，平时是桌面端（或适配器）在启动后推给它的。

## 能用 ACP 接的几条路

| | 做法 | 现状 |
|---|---|---|
| `zcode-acp-server`（[william0wang/zcode-acp][zcode-acp]） | 启动 `zcode app-server --stdio`，把私有事件流翻成 ACP v1。找 CLI 的顺序是 `ZCODE_BIN` → `PATH` 里的 `zcode` → 桌面端里的 `zcode.cjs`。登录凭据直接读桌面端的 `~/.zcode/v2` | npm 上 0.97.0（2026-11-28 发版，共 198 个版本），Apache-2.0，个人维护。要求 ZCode CLI ≥ 0.14.8、Node.js ≥ 22。**本文实测用的就是它** |
| `zcode-acp`（[supermomonga/zcode-acp][zcode-acp-sm]） | Bun 打成的单文件，在后台启动桌面端的 host 服务 | 只认 ZCode 3.11.2 / CLI 0.16.5，校验哈希，不符就拒绝；2026-10 之后没更新，现在的版本用不了。README 自己提醒可能违反 ZCode 服务条款 |
| `glm-acp-agent` | 自己直接调 GLM API，跟 ZCode 无关 | 收录在 ACP Registry 里 |

适配器在 `initialize` 里公布的能力（实测）：

- 协议版本 1；支持 `loadSession`；`sessionCapabilities` 有 `list`、`resume`、`fork`。
- `mcpCapabilities`：`http: true`，`sse: false`。
- `promptCapabilities`：支持图片和嵌入资源。
- `_meta.steering`：运行中插话，时机有 `now`、`next`。
- `authMethods` 只有 `zcode-credentials`，即读桌面端存下的凭据。

## 本机实测

环境：

- macOS arm64，Node.js 24。
- ZCode 3.16.3，自带 CLI 0.17.0。
- `zcode-acp-server` 0.97.0，用 `npm install --ignore-scripts` 装在临时目录。
- `board` 是当时 main 上编出来的 0.1.62。

| 项 | 结果 |
|---|---|
| 握手 | Board 要 v2，适配器回 1，之后按 v1 走（和 dsh 一样） |
| 开会话 | 正常，默认模式是 `yolo`（不问权限）。`configOptions` 有三项：`model`（类别 `model`）、`mode`（类别 `mode`）、`thought`（类别 `thought_level`，可选 `low` / `high` / `max`） |
| 切模型、强度 | `--set @model=…`、`--set @thought_level=low` 都生效：Board 在 `session/new` 之后发 `session/set_config_option`，适配器回 `config_option_update` |
| 一轮干活 | 用 Start Plan 的 GLM-5.3-Flash：调 `Write` 工具建了文件，回复 `DONE`，`stopReason` 是 `end_turn`，退出码 0。prompt 回应里带 `usage`（`inputTokens`、`outputTokens`、`cachedReadTokens`、`totalTokens`） |
| `board-skills` 自检（[agents.md](agents.md)「本机验证」里那句提示词） | 通过：ZCode 用自己的 `Skill` 工具加载了 `board-skills`，答出了名字和 SKILL.md 的第一个二级标题 |
| 身份（`--auth only`） | 返回 `{"kind":"unknown","label":"这个版本的 agent 不报身份"}`：适配器没实现 `auth/status` |
| 模型调用失败 | 见下表。两种失败都只作为一条 `agent_message_chunk` 发出来（内容是 `[Turn failed: <代码> …]`），这一轮照样回 `stopReason: end_turn`，`board acp` 退出码 0 |

实测遇到的两种模型调用失败：

| 情况 | 报错 |
|---|---|
| 个人 Coding Plan 没额度 | `1113 余额不足或无可用资源包，请充值` |
| Start Plan 服务出问题 | `MODEL_ERROR … empty_model_response`。同一时段桌面端自己发起的会话也是同样的失败，几分钟后恢复 |

### 用 Start Plan（体验套餐）

适配器按账单接口查出账号里有哪些套餐，而且要等 ZCode 后端启动之后才查。所以：

- `session/new` 的回应里，模型列表只有个人 Coding Plan 的模型（`account:bigmodel-individual-coding-plan\GLM-5.4` 等）。
- 后端起来、把账号里的供应商推上来以后，`config_option_update` 里才出现 `account:bigmodel-start-plan\GLM-5.3-Flash`。

由此带来的影响和办法：

- **用 Board 的 `--set @model=account:bigmodel-start-plan\GLM-5.3-Flash` 选不上。** `session/new` 的回应里没有这个值，`board acp` 跳过它、按默认模型跑，也不报错（见下面「接进 Board 要做的事」第 3 条）。
- **办法一：启动适配器时固定供应商和模型。** 设 `ZCODE_PROVIDER=account:bigmodel-start-plan`、`ZCODE_MODEL=GLM-5.3-Flash`。`session/new` 的回应里仍然先报默认模型，后端起来后 `config_option_update` 会切到 Start Plan，这一轮用的就是 Start Plan。
  - 适配器 README 里写的 `builtin:bigmodel-start-plan` 写法不生效：这种写法按 `~/.zcode/v2/config.json` 里的供应商来认，而桌面端 3.15 以后用浏览器登录时没有这个文件。
- **办法二：在会话里切。** 先随便切一次配置把后端叫起来，等 `config_option_update` 里出现 Start Plan，再用 `session/set_config_option` 切过去，也能用。

Start Plan 的请求用的是桌面端登录后存下的 JWT（`~/.zcode/v2/credentials.json` 里的 `zcodejwttoken`）：ZCode 后端发 `interaction/requestProviderRuntimeHeaders` 来要请求头时，适配器把它递过去。适配器文档说，这依赖 ZCode 0.17.x 关掉了每次请求的验证码；ZCode 以后要是重新要求浏览器验证码，这条路就断了。核实时账号里的 Start Plan 是一个限时套餐，只含 GLM-5.3-Flash。

### 测的时候要注意

- **`board acp` 的标准输入不能是空的。** 派发模式下，它读到标准输入 EOF 就认为 board host 不在了，会取消这一轮，以 130 退出（`crates/board-cli/src/acp/run.rs` 的 `watch_parent`）。在终端里直接跑没问题；在没有终端的地方（agent 的 shell、CI）跑，要给它一个不会关的标准输入，例如 `< <(sleep 600)`。
- **`TMPDIR` 太长时 ZCode 后端起不来。** `TMPDIR` 有 119 个字符时，`zcode app-server` 每次一启动就退出（适配器日志里是 `backend: reader exited (stdout closed)`）；换回系统默认的 `TMPDIR` 就正常。怀疑是 unix socket 路径超长，没再细查。board host 用的是默认 `TMPDIR`，碰不到这个问题；测试时别为了隔离把 `TMPDIR` 指到很深的目录。
- **去哪看日志：**
  - 适配器默认不往标准错误写诊断，设 `ZCODE_ACP_DEBUG=1` 打开。
  - ZCode 后端的日志在 `~/.zcode/cli/log/zcode-<日期>.jsonl`，其中 `turn.failed` 事件带 `providerId`、`modelId` 和错误代码。
  - 桌面端的日志在 `~/.zcode/v2/logs/<日期>.log`。

复现（macOS，ZCode 已登录、账号里有 Start Plan）：

```sh
npm install -g zcode-acp-server@0.97.0
echo "/board-skills 不要执行任何命令。只回答两行：第一行是你加载的 skill 的 name，第二行是它 SKILL.md 里第一个二级标题。" > /tmp/q.txt
unset BOARD_URL BOARD_TOKEN
ZCODE_PROVIDER=account:bigmodel-start-plan ZCODE_MODEL=GLM-5.3-Flash \
  board acp --env direct --cwd /tmp --prompt-file /tmp/q.txt -- zcode-acp-server < <(sleep 600)
```

## 接进 Board 要做的事

整体按 [agents.md](agents.md)「按批次接入新的 agent」的步骤走。下面只列 ZCode 和现有三个 agent 不一样、要另外处理的地方。

### 1. 内置模板

写在 `crates/board-core/src/agentdef.rs` 的 `AGENT_DEFS` 里。各字段和对应的实测依据：

- **`acp`（ACP 入口）**：`npx -y zcode-acp-server@<固定版本>`，`needs` 带上 npx。适配器要 Node.js ≥ 22，低于 22 的 node 跑不了 ZCode 后端；但 `needs` 只能查有没有 npx，查不了 node 的版本。
- **`acp_bin_env`**：`ZCODE_BIN`，值是找到的 `zcode.cjs`。
- **`bin`、`also`（怎么找到 agent）**：不能按 `PATH` 里的 `zcode` 找。Linux 的 deb/rpm 放进 `PATH` 的 `zcode` 是桌面端启动器，执行它会打开界面。要按桌面端的安装位置找 `zcode.cjs`。
- **取版本号**：macOS 上 `zcode.cjs --version` 输出 `zcode 0.17.0`（文件有 `#!/usr/bin/env node`，可以直接执行）；Windows 上 `.cjs` 不能直接执行，取版本要经 `node`。
- **`install`（安装、更新）**：没有 npm 包，只能从官网装桌面端，写不出安装命令。CLI 跟着桌面端更新。
- **`acp_mode`**：`yolo`。适配器默认就是 `yolo`，但用户在 `~/.config/zcode-acp/config.json` 里改了 `session.mode` 就会变，所以要显式切一次。可选的模式有 `plan`、`build`、`edit`、`guarded`、`yolo`。
- **`skills`、`project`（技能目录）**：
  - `skills`：用户级目录 `~/.zcode/skills`。ZCode 同时兼容 `~/.agents/skills`，实测的 `board-skills` 就是从 `~/.agents/skills` 读到的，和 dsh 共用。
  - `project`：`.zcode/skills`、`.agents/skills`。
- **`efforts`**：`low`、`high`、`max`，即 ZCode 的 `thought` 选项。
- **`protocols`**：ZCode 的模型供应商由它自己的账号和 `provider_config.json` 管，Board 的「模型供应商」能不能、怎么传给它还没查。先只支持「本机配置」角色。
- **`mcp`**：ZCode 的 MCP 配置在 `~/.zcode/cli/config.json`，格式没核实，先不同步。另一条路是 `session/new` 的 `mcpServers`：适配器能接 stdio、http，不支持 sse；Board 现在传的是空列表。

### 2. 认出失败的一轮

适配器把模型调用失败报成一条 `agent_message_chunk` 加 `end_turn`，`board acp` 会记成正常结束。如果是做卡的派发，Board 会续 3 轮，每轮都同样失败，最后记「异常退出」。

- 实测失败时，`$/zcode/turnState` / `_zcode/turnState` 通知里只有 `{running: false}`，不带错误。
- 所以要么按消息文字 `[Turn failed: <代码> …]` 认（实测见过的代码有 `1113`、`MODEL_ERROR`），要么请适配器改成回错误。
- `1113`（余额不足或无可用资源包）对应 Board 的「额度用完」，但报错里没有重置时间。

### 3. 选不上的模型不能悄悄按默认跑

`board acp` 只对认得 `session/options` 的 v2 agent 检查按类别的 `--set` 有没有设上（`crates/board-cli/src/acp/client.rs` 里 `Pending::New` 那段，注释是「认出了 session/options 的 v2 agent」）。对 v1 agent（dsh、这个适配器），要的值不在选项里时直接跳过、按默认模型跑。这和 [acp.md](acp.md) 2026-11-25 补记（#666）里写的「账号里没有的模型……`board acp` 据此出错结束」不一致。

另外，ZCode 账号里的模型是后来才推上来的。修的时候要等 `config_option_update`（或者在发 prompt 之前再看一次选项），不能只看 `session/new` 的回应。

### 4. 身份

适配器没有 `auth/status`，也没有 `_auth/status_update`，Board 只能报「不报身份」。不过 `session/new` 回应里的 `_meta.zcode.currentProviderId`（例如 `builtin:bigmodel-coding-plan`）和 `providers` 列表，能看出当前用的是哪个套餐。

### 5. 额度

适配器的命令表里查额度的命令叫 `/quota`，没有 `/usage`、`/status`，所以 `board acp --usage` 用不了。本机跑 `zcode-acp quota` 报「Quota info unavailable」。桌面端日志里的 `billing/balance` 有套餐的总量、已用、剩余和起止时间，但这些不经 ACP。

### 6. 技能命令

适配器公布的技能命令带 `$` 前缀（例如 `$board-skills`），但实测提示词以 `/board-skills` 开头 ZCode 也认。Board 派发的提示词不用改。

### 7. 前提和依赖

- 机器上要装着 ZCode 桌面端并登录过：适配器读的是 `~/.zcode/v2` 里的凭据。Linux 上没有系统钥匙串时，桌面端要 ≥ 3.18.2 才登录得上。
- 适配器是个人项目，底下的 ZCode Protocol 没有文档，ZCode 又发得勤，升级可能把它弄断。例如 ZCode CLI 0.16.0 去掉了 steer、rewind，适配器跟着删了对应的扩展。
- 所以要固定适配器的版本，升 ZCode 或升适配器之后，用上面的复现命令跑一次。

### 8. 其他实测到的行为

- 适配器每开一次会话，就在 ZCode 的会话库里建一个会话，桌面端的任务列表里看得到。
- ZCode 每轮的输入约 1.2 万 token（工具和技能的描述），一句「只回复 OK」也是这个量。

## 来源

- ZCode 文档：[常见问题](https://zcode.z.ai/cn/docs/qa)、[安装](https://zcode.z.ai/cn/docs/install)、[远程开发](https://zcode.z.ai/cn/docs/remote-development)、[Skill](https://zcode.z.ai/cn/docs/skill)、[更新日志](https://zcode.z.ai/cn/changelog)
- [Z.ai 文档：在 ZCode 里用 GLM Coding Plan](https://docs.z.ai/devpack/tool/zcode)
- [william0wang/zcode-acp][zcode-acp]（README、`docs/TROUBLESHOOTING.md`、`docs/CONFIG-OPTIONS.md`）、[npm：zcode-acp-server](https://www.npmjs.com/package/zcode-acp-server)
- [supermomonga/zcode-acp][zcode-acp-sm]
- [ACP Registry][acp-registry]
- 本机：ZCode 3.16.3 的安装包（`app.asar`，`glm/zcode.cjs` 的帮助信息和字符串）、`~/.zcode` 下的日志

[acp-registry]: https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json
[zcode-acp]: https://github.com/william0wang/zcode-acp
[zcode-acp-sm]: https://github.com/supermomonga/zcode-acp
