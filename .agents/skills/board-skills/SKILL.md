---
name: board-skills
description: 用 board 命令行处理 Board 白板上的卡片：领取、按描述干活、记录进度、交给人审核。用户发来 `/board-skills <引用>`（例如 `开始执行 mini项目之中的main白板中的#2任务`），或贴出 `项目/白板#ID`、`项目/白板` 这样的 Board 引用时使用。
---

# Board

Board 是看板式白板：人在界面上排优先级、审核；你用 `board` 命令行领取卡片、干活、交审。人和你读写的是同一块白板。

## 参数

| 用户给的 | 例子 | 做什么 |
|---|---|---|
| 卡片 | `mini/main#2`，或 `开始执行 mini项目之中的main白板中的#2任务` | 处理这一张卡片 |
| 白板 | `mini/main`，或项目只有一块白板时的 `mini` | 循环执行不带 ID 的 `board claim`，直到 `EMPTY_TODO` |
| 一句话需求 | `README 加一个英文版` | 先 `board new "<标题>" --body "<描述和验收标准>"` 记到首个可领取状态，再按卡片处理。用户没说放哪块白板时，环境里有 `BOARD` 就用它，没有就问用户 |
| 快捷提示词 | `开始做 mini项目之中的main白板中待办里的所有任务` | 按下文「快捷提示词」处理（先于「一句话需求」判断） |

## 快捷提示词

界面上「提示词」菜单复制出来的四种句子。`X项目之中的Y白板` 里 X、Y 是标识，白板就是 `X/Y`（每条命令带 `--board X/Y`，或前缀 `BOARD=X/Y`）。句子里的「待办」是那块白板领取起点的名称，按 `board flow` 里的名称对应到状态。

| 句子 | 做什么 |
|---|---|
| `把下面的需求拆解成可以独立完成的任务，记到X项目之中的Y白板的待办里：<需求>` | 读懂需求（需要时看代码），拆成能各自独立完成、独立验收的任务，每个任务 `board new "<标题>" --body "<背景、范围，以及 - [ ] 验收标准>"`，按先后顺序新建。只建卡片，不领取、不开始做。最后列出新建的卡片 ID 和标题 |
| `把我们刚才讨论出的任务上传到X项目之中的Y白板的待办里，每个任务一张卡片` | 把这次对话里已经定下的任务原样记成卡片：每个任务一次 `board new`，描述写清背景和验收标准。不重新拆分、不增删任务、不领取。最后列出新建的卡片 ID 和标题 |
| `开始做X项目之中的Y白板中待办里的所有任务` | 只做这一个状态里的卡片：循环执行 `board ls --column <待办> --json`，按列出的顺序取第一张没人持有的卡片，`board claim <ID>` 后按「处理一张卡片」处理；列表为空（或剩下的都被别人持有）时停止，汇报处理了哪些卡片 |
| `开始做X项目之中的Y白板中待办里所有标签带<标签>的任务` | 同上，但列卡片时加 `--label <标签>`，只领带这个标签的卡片 |

- 每做完一张都重新 `board ls`：人可能调了顺序，别的 agent 可能领走了卡片。
- `board claim <ID>` 返回 `NOT_IN_TODO`、`NOT_HOLDER`（被别人抢先领走）时跳过这张，接着列下一张。

`开始执行 X项目之中的Y白板中的#Z任务` 是卡片详情「复制为提示词」复制出的完整写法：X 是项目标识、Y 是白板标识、Z 是卡片编号，对应的引用是 `X/Y#Z`。

## 会话开始

每个会话开始时做一次（处理卡片、白板、一句话需求之前）：

1. **生成名字**：`<运行环境>/<模型>`，例如 `claude/opus-5.5`、`dsh/deepseek-v4.1-flash`、`codex/gpt-5`。
   - 运行环境用 claude、codex、dsh 这类名称；模型名转成小写，空格换成 `-`。
   - 两段都只能用小写字母、数字、`.`、`-`，以字母或数字开头；只有一个 `/`，总长不超过 48 个字符。
   - 本机已有同名会话时，加一个和本会话相关的后缀，例如任务名或分支名：`claude/opus-5.5-login`。
2. **占用名字**：`board check --as <名字>`。返回 `NAME_TAKEN`：名字正被另一个在线会话使用，加一个和本会话相关的后缀再 check。
3. **启动心跳**（整个会话只执行这一次）：

   ```sh
   BOARD_AS=<名字> sh <skill 目录>/scripts/heartbeat.sh start                     # macOS / Linux
   $env:BOARD_AS='<名字>'; powershell -NoProfile -ExecutionPolicy Bypass -File <skill 目录>\scripts\heartbeat.ps1 start   # Windows
   ```

   脚本先同步发一次 `board beat`（失败直接报错，按错误码处理），再在后台每 10 秒发一次，立即返回。这一路心跳同时维持你的名字在线和你持有的所有卡片，领卡后不用再为卡片单独启动心跳。你的进程不在了（崩溃、被关掉），脚本跟着停下：服务端连续 5 次（50 秒）收不到心跳，名字离线，你持有的卡片回到领取它时所在的可领取状态最上面。找不到脚本时告诉用户（skill 需要连同 `scripts/` 目录一起安装）。
4. **之后每条命令都带名字和白板**：`--as <名字> --board <项目>/<白板>`，或前缀 `BOARD_AS=<名字> BOARD=<项目>/<白板>`，例如：

   ```sh
   BOARD_AS=claude/opus-5.5 BOARD=mini/main board claim 2 --json
   board claim 2 --as claude/opus-5.5 --board mini/main --json
   ```

   你的每条命令都在新的 shell 里跑，`export` 留不到下一条。下文的命令为了简洁省略了这两个参数。

**连接服务端**：`board` 只通过服务端读写白板。服务端地址和访问令牌由用户用 `board login` 写进 `~/.board/config.toml`，你不用设置。

- 得到 `NOT_LOGGED_IN` 时停下来，请用户运行 `board login --url <服务端地址> --token <令牌>`（令牌在界面「设置 → Agent 接入」新建）。不要自己去找令牌。
- 令牌是密钥：不要写进代码、提交、卡片评论或输出里。
- 名字只是你的身份，和令牌无关；同一个令牌可以被多个不同名字的 agent 使用。

## 工作流

动一块白板上的卡片之前，先 `board flow` 看它的工作流：状态，以及每个状态能去哪里（连线）。每块白板可以自定义，默认是：

```
1. todo    待办        可领取  → 进行中
2. doing   进行中      锁定    → 待审核、需人工校验
3. review  待审核              → 已完成、返工
4. done    已完成      终点    → （没有连线）
5. rework  返工        可领取  → 进行中
6. check   需人工校验          → 待办
```

- **你只能沿连线移动卡片**：`board move` 的目标必须是当前状态 `→` 后面列出的状态，否则被拒绝（`FORBIDDEN_COLUMN`，原因里列出能去的状态）。在同一个状态内调整顺序不受限制。
- **可领取**：`board claim` 从这里领，领取后卡片沿连线进入锁定状态。可以有多个（默认是「待办」和「返工」）；不带 ID 时按工作流顺序，先领排在前面的。
- **锁定**：只能靠领取进入。有多个锁定状态（例如「开发中 → 测试中」）时，你可以沿连线把自己的卡片 move 到下一个锁定状态，领取保持。
- **终点**：只有人能移入、移出。
- **未整理**：只有人能移入、移出；你不领取，也不移动。
- **交审**：沿连线移到等人审核的状态（默认是 `review`「待审核」）。
- **需要人处理**：沿连线移到等人处理的状态（默认是 `check`「需人工校验」）。**不要把卡片放回可领取状态**，那样人看不到它出了问题。
- 白板不是默认工作流时，按 `board flow` 里的状态名称判断哪个是交审、哪个是需要人处理；当前状态没有合适的连线时，评论说明情况、`heartbeat.sh final <ID>` 结束这张卡片的计时，然后停下来告诉用户（final 50 秒后，卡片自动回到领取它时所在的状态）。
- `move`、`ls --column` 都可以写状态的标识或名称。

- 代码在当前工作目录（当前仓库）里改；卡片描述指定了别的位置时以描述为准。
- `board` 不在 PATH 里时，告诉用户安装（在 Board 仓库执行 `cargo install --path crates/board-cli`），不要自己装。
- 得到 `NOT_LOGGED_IN`、`INVALID_TOKEN`、`UNREACHABLE` 时停下来告诉用户（见文末错误码），不要反复重试。

## 处理一张卡片

`board claim <ID>` → 干活 → `heartbeat.sh final <ID>` → 50 秒内 `board move <ID> review`。

1. **领取**：`board claim <ID> --json`。返回卡片的全部字段：标题、描述（验收标准在这里）、标签、动态、附件。**先读动态**：人的意见、被打回的原因都在里面。**有附件时先 `board files <ID>` 下载到本地并查看**（截图、日志常常就是问题本身）。
   - 领取后不用单独为这张卡片启动心跳：会话开始时启动的后台心跳会维持它。领取后的第一次心跳开始这一次计时（动态里记「开始第 n 次」），final 结束，卡片上显示累计耗时。
   - `NOT_IN_TODO`：用 `board show <ID> --json` 看它在哪：
     - 在锁定状态且持有者是你：接着做
     - 「未整理」（inbox）：按下文「整理未整理的条目」处理
     - 在其他状态，或别人持有：不要动，告诉用户它现在的状态
   - `ALREADY_HOLDING`：你在这块白板上还拿着另一张卡片，先把那张做完，或按第 4 步交给人处理
2. **干活**：按描述和验收标准改代码、跑测试。耗时较长时用 `board comment <ID> "<进展>"` 记进度。后台心跳在，干多久都不会被释放。
3. **交审**：评论写清楚改了什么、怎么验证的（测试命令和结果、提交号），**结束这张卡片的计时**，再马上沿连线交审：

   ```sh
   BOARD_AS=<名字> BOARD=<项目>/<白板> sh <skill 目录>/scripts/heartbeat.sh final <ID>
   board move <ID> review
   ```

   Windows：`$env:BOARD_AS='<名字>'; $env:BOARD='<项目>/<白板>'; powershell -NoProfile -ExecutionPolicy Bypass -File <skill 目录>\scripts\heartbeat.ps1 final <ID>`。
   final 之后后台心跳不再刷新这张卡片，**50 秒内没有移走，领取会被释放**；final 之后再发 `board beat <ID>` 会被拒绝。界面改动可以附上截图：`board attach <ID> <截图> --comment "<说明>"`。
4. **做不下去**：先 `board comment <ID> "<原因，或需要人确认的问题>"`，`heartbeat.sh final <ID>`，再马上沿连线移到需要人处理的状态：默认是 `board move <ID> check`（需人工校验）。不要放回可领取状态。

## 处理一块白板

循环：`board claim --json`（不带 ID，按工作流顺序领可领取状态最上面的）→ 按上面 2–4 处理 → 再 claim，直到返回 `EMPTY_TODO`。最后汇报处理了哪些卡片、各自的结果。

## 会话结束

所有卡片都处理完（都已交审或交给人）后，停掉后台心跳，名字立刻离线：

```sh
BOARD_AS=<名字> sh <skill 目录>/scripts/heartbeat.sh stop
$env:BOARD_AS='<名字>'; powershell -NoProfile -ExecutionPolicy Bypass -File <skill 目录>\scripts\heartbeat.ps1 stop   # Windows
```

会话中途得到 `NAME_OFFLINE`：你的名字已经离线（心跳中断超过 50 秒，或心跳脚本停了），你持有的卡片可能已经被释放。重新 `board check --as <名字>`（被占用就加后缀），再 `heartbeat.sh start`，然后 `board show <ID> --json` 看卡片现在的状态再决定。

## 整理未整理的条目

卡片在「未整理」时，它还只是个想法。把它改写成可执行的卡片：

```sh
board edit <ID> --title "<一句话说清要做什么>" --body "<背景、范围，以及 - [ ] 验收标准>"
board comment <ID> "<整理说明；信息不够时列出需要人确认的问题>"
```

不要移动它：放不放进工作流由人决定。

## 规则（违反会被拒绝）

- 只有可领取状态里的卡片能领取；进入锁定状态只能靠 claim（持有者沿连线在锁定状态之间移动除外）
- 你只能沿 `board flow` 里的连线移动卡片；不能移入、移出「未整理」
- 你在一块白板上同时只能持有一张卡片
- 你的名字要先 check，并且一直有心跳：连续 5 次（50 秒）没收到，名字离线，你持有的卡片释放；名字不在线时卡片命令被拒绝（`NAME_OFFLINE`）
- final 之后 50 秒内要沿连线移走卡片，否则领取释放
- 被持有的卡片只有持有者能编辑、移动；别人的卡片你只能评论、加或去标签
- 不能把卡片移入或移出终点状态，那由人确认；不能删除卡片

## 常用命令

```sh
board flow                                # 工作流：状态的顺序、标识、名称、规则
board show <ID> --json                    # 全部字段和动态
board ls [--column todo] [--label bug] [--search <文字>]
board comment <ID> "<内容>"
board check --as <名字>                   # 会话开始时占用名字
board beat [--final]                      # agent 级心跳 / 名字立刻离线；由 scripts/heartbeat.sh start / stop 调用
board beat <ID> --final                   # 结束这张卡片的计时；由 scripts/heartbeat.sh final <ID> 调用
board move <ID> <状态标识或名称> [--top]
board label <ID> +<标签> -<标签>         # 标签必须已在项目里存在，先 board labels 看有哪些
board labels
board boards                              # 列出所有白板
board files <ID> [--out <目录>]           # 下载卡片的附件到本地，列出路径
board attach <ID> <文件>… [--comment "<说明>"]   # 上传附件（图片、日志等，≤ 10 MB），作为一条评论
```

所有命令都支持 `--json`。被拒绝时退出码为 1，JSON 里是 `{"error": {"code", "message"}}`。

| 错误码 | 含义 |
|---|---|
| `NO_IDENTITY` / `NO_BOARD` | 命令没带名字 / 白板：先 `board check --as <名字>`，之后每条命令都带 `--as <名字>`（或前缀 `BOARD_AS=<名字>`）和 `--board <项目>/<白板>`（或前缀 `BOARD=<项目>/<白板>`） |
| `NAME_TAKEN` | 名字正被另一个在线会话使用：加一个和本会话相关的后缀再 check |
| `NAME_OFFLINE` | 名字不在线：没有 check，或心跳中断超过 50 秒。重新 check 并启动心跳（见「会话结束」最后一段） |
| `BOARD_NOT_FOUND` | 白板不存在；或项目有多块白板却只写了项目标识（原因里列出了可选的白板） |
| `NOT_FOUND` | 卡片不存在 |
| `EMPTY_TODO` | 可领取的状态里都没有能领的卡片 |
| `NOT_IN_TODO` | 卡片不在可领取状态，也不是你持有的 |
| `ALREADY_HOLDING` | 你已经持有另一张卡片 |
| `NOT_HOLDER` | 卡片被别人持有 |
| `FORBIDDEN_COLUMN` | 不沿连线移动；直接移进锁定状态；移入、移出终点状态或「未整理」；从没有连到锁定状态的可领取状态领取 |
| `EMPTY_TITLE` / `USAGE` | 标题为空 / 命令或参数不对（例如不存在的状态或标签、空评论） |
| `NOT_LOGGED_IN` | 还没登录：请用户运行 `board login --url <服务端地址> --token <令牌>`（令牌在「设置 → Agent 接入」新建） |
| `INVALID_TOKEN` | 令牌写错或已被吊销：请用户在「设置 → Agent 接入」新建一个，再 `board login --token <令牌>` |
| `UNREACHABLE` | 连不上服务端：告诉用户服务端连不上，先确认地址和网络（`board doctor` 能看出地址来自哪里） |
