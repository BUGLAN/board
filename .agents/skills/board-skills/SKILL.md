---
name: board-skills
description: 用 board 命令行处理 Board 白板上的卡片：领取、按描述干活、记录进度、交给人审核。用户发来 `/board-skills <引用>`，或贴出 `项目/白板#ID`、`项目/白板` 这样的 Board 引用时使用。
---

# Board

Board 是看板式白板：人在界面上排优先级、审核；你用 `board` 命令行领取卡片、干活、交审。人和你读写的是同一块白板。

## 参数

| 用户给的 | 例子 | 做什么 |
|---|---|---|
| 卡片 | `mini/main#2` | 处理这一张卡片 |
| 白板 | `mini/main`，或项目只有一块白板时的 `mini` | 循环执行不带 ID 的 `board claim`，直到 `EMPTY_TODO` |
| 一句话需求 | `README 加一个英文版` | 先 `board new "<标题>" --body "<描述和验收标准>"` 记到首个可领取状态，再按卡片处理。白板用 `BOARD`；没有就问用户放哪块白板 |

## 准备

```sh
export BOARD=mini/main      # 引用里 # 前面的部分
export BOARD_AS=<你的名字>  # 已经设置就沿用；否则用你自己的名字，例如 deepseek、claude、codex，整个会话保持一致
```

**连接服务端**：白板在线上的服务端（例如 `https://board.buglan.com`）时，还需要：

```sh
export BOARD_URL=https://board.buglan.com   # 服务端地址
export BOARD_TOKEN=<访问令牌>                # 用户在界面「设置 → Agent 接入」新建
```

- `BOARD_URL`、`BOARD_TOKEN` 由用户提供：环境里已经有就沿用；没有时问用户要，不要自己去找令牌。
- 令牌是密钥：不要写进代码、提交、卡片评论或输出里。
- `BOARD_AS` 只是你的名字，和令牌无关；同一个令牌可以被多个不同名字的 agent 使用。
- 没有设置 `BOARD_URL` 时，`board` 直接读写本机的数据库。

然后执行 `board flow` 看这块白板的工作流：状态，以及每个状态能去哪里（连线）。每块白板可以自定义，默认是：

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
- 白板不是默认工作流时，按 `board flow` 里的状态名称判断哪个是交审、哪个是需要人处理；当前状态没有合适的连线时，评论说明情况、结束心跳，然后停下来告诉用户（结束心跳 50 秒后，卡片自动回到领取它时所在的状态）。
- `move`、`ls --column` 都可以写状态的标识或名称。

- 代码在当前工作目录（当前仓库）里改；卡片描述指定了别的位置时以描述为准。
- `board` 不在 PATH 里时，告诉用户安装（在 Board 仓库执行 `cargo install --path crates/board-cli`），不要自己装。
- 得到 `NOT_LOGGED_IN`、`INVALID_TOKEN`、`UNREACHABLE` 时停下来告诉用户（见文末错误码），不要反复重试。

## 处理一张卡片

1. **领取**：`board claim <ID> --json`。返回卡片的全部字段：标题、描述（验收标准在这里）、标签、动态、附件。**先读动态**：人的意见、被打回的原因都在里面。**有附件时先 `board files <ID>` 下载到本地并查看**（截图、日志常常就是问题本身）。
   - `NOT_IN_TODO`：用 `board show <ID> --json` 看它在哪：
     - 在锁定状态且持有者是你：接着做（心跳没在跑就先 `heartbeat.sh start <ID>`）
     - 「未整理」（inbox）：按下文「整理未整理的条目」处理
     - 在其他状态，或别人持有：不要动，告诉用户它现在的状态
   - `ALREADY_HOLDING`：你在这块白板上还拿着另一张卡片，先把那张做完，或按第 5 步交给人处理
2. **开始心跳**（必须）：领取成功后马上在后台开始心跳，表明你在工作。**领取后 50 秒内没收到心跳，领取会被释放**：

   ```sh
   sh <skill 目录>/scripts/heartbeat.sh start <ID>                      # macOS / Linux
   powershell -NoProfile -ExecutionPolicy Bypass -File <skill 目录>\scripts\heartbeat.ps1 start <ID>   # Windows
   ```

   脚本先同步发一次 `board beat <ID>`（失败直接报错，按错误码处理），再在后台每 10 秒发一次，立即返回。第一次心跳开始计时，final 结束，这一次的耗时记进动态，卡片上显示累计耗时。心跳在，干多久都不会被释放；你的进程不在了（崩溃、被关掉），脚本跟着停下，服务端**连续 5 次（50 秒）收不到心跳就判定你异常退出**，卡片回到领取它时所在的可领取状态最上面。找不到脚本时告诉用户（skill 需要连同 `scripts/` 目录一起安装）。
3. **干活**：按描述和验收标准改代码、跑测试。耗时较长时用 `board comment <ID> "<进展>"` 记进度。评论、编辑不能代替心跳。
4. **交审**：评论写清楚改了什么、怎么验证的（测试命令和结果、提交号），**结束心跳**（`heartbeat.sh stop <ID>`，发送 final，正常退出），再马上沿连线 `board move <ID> review`（交审；final 之后 50 秒内没有移走，领取会被释放）。界面改动可以附上截图：`board attach <ID> <截图> --comment "<说明>"`。
5. **做不下去**：先 `board comment <ID> "<原因，或需要人确认的问题>"`，结束心跳（`heartbeat.sh stop <ID>`），再马上沿连线移到需要人处理的状态：默认是 `board move <ID> check`（需人工校验）。不要放回可领取状态。

## 处理一块白板

循环：`board claim --json`（不带 ID，按工作流顺序领可领取状态最上面的）→ 按上面 2–5 处理 → 再 claim，直到返回 `EMPTY_TODO`。最后汇报处理了哪些卡片、各自的结果。

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
- 持有期间必须一直发心跳：连续 5 次（50 秒）没收到，领取释放
- 被持有的卡片只有持有者能编辑、移动；别人的卡片你只能评论、加或去标签
- 不能把卡片移入或移出终点状态，那由人确认；不能删除卡片

## 常用命令

```sh
board flow                                # 工作流：状态的顺序、标识、名称、规则
board show <ID> --json                    # 全部字段和动态
board ls [--column todo] [--label bug] [--search <文字>]
board comment <ID> "<内容>"
board beat <ID> [--final]                 # 心跳 / 正常结束；一般由 scripts/heartbeat.sh 调用，不用手动发
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
| `NO_IDENTITY` / `NO_BOARD` | 没有设置 `BOARD_AS` / `BOARD` |
| `BOARD_NOT_FOUND` | 白板不存在；或项目有多块白板却只写了项目标识（原因里列出了可选的白板） |
| `NOT_FOUND` | 卡片不存在 |
| `EMPTY_TODO` | 可领取的状态里都没有能领的卡片 |
| `NOT_IN_TODO` | 卡片不在可领取状态，也不是你持有的 |
| `ALREADY_HOLDING` | 你已经持有另一张卡片 |
| `NOT_HOLDER` | 卡片被别人持有 |
| `FORBIDDEN_COLUMN` | 不沿连线移动；直接移进锁定状态；移入、移出终点状态或「未整理」；从没有连到锁定状态的可领取状态领取 |
| `EMPTY_TITLE` / `USAGE` | 标题为空 / 命令或参数不对（例如不存在的状态或标签、空评论） |
| `NOT_LOGGED_IN` | 设置了 `BOARD_URL` 却没有 `BOARD_TOKEN`：请用户提供访问令牌 |
| `INVALID_TOKEN` | 令牌写错或已被吊销：请用户在「设置 → Agent 接入」新建一个 |
| `UNREACHABLE` | 连不上 `BOARD_URL`：告诉用户服务端连不上，先确认地址和网络 |
