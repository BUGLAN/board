# board-skills：工作流、命令、错误码（reference.md）

`/board-skills` 的分文件：工作流、常用命令、错误码。处理卡片的流程在 work.md、host.md。

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

状态的规则（可领取、锁定、终点、错误节点）和能做什么见 SKILL.md「规则」；`move`、`ls --column` 都可以写状态的标识或名称。默认工作流的名字只是例子，以 `board flow` 为准。

`board flow` 还列出这块白板所在项目的**代码仓库**（`repo`）和**集成分支**（`branch`，空表示按仓库的 AGENTS.md / CLAUDE.md），见 work.md「代码」。

**交审、做不下去时卡片去哪**：由 `board submit`、`board giveup` 在服务端认，`board start --json` 的 `review`、`giveup` 就是结果。交审状态看状态设置里的「交审状态」开关（`board flow` 的规则写「交审」，`--json` 里 `"review": true`），一块白板上都没打开时看名称里的「审核」这类关键词；错误节点只看 `board flow` 里的「错误节点」规则（`--json` 里 `"error": true`），不看名称；需要人处理的状态按名称认（例如带「人工」）。`board giveup` 先去错误节点，没有时去需要人处理的状态；两种都没有时只评论、结束计时，50 秒后卡片回到领取前的状态，这时停下来告诉用户。卡片所在的锁定状态连不到交审状态时 `board submit` 返回 `FORBIDDEN_COLUMN`：连着下一个锁定状态时（两级锁定，`board start --json` 的 `next_lock`）先 `board move` 过去再交审；真的没有交审状态时改用 `board giveup`。

- 代码仓库是当前工作目录所在的仓库；卡片描述指定了别的位置时以描述为准。
- 得到 `NOT_LOGGED_IN`、`INVALID_TOKEN`、`UNREACHABLE` 时停下来告诉用户（见文末错误码），不要反复重试。

## 常用命令

```sh
board start [<ID>] --agent <标识> --json  # 开始做一张卡：占用名字、启动后台心跳（已经有了就接着用）、领取、下载附件，返回卡片和交审 / 做不下去去哪、下一个锁定状态（next_lock）、仓库、集成分支
board submit <ID> --comment-file <文件> [<截图>…] [--no-code]   # 交审：上传附件、评论、结束计时、移到交审状态，一步做完；改了代码的卡片评论里要有固定的四行
board giveup <ID> --comment-file <文件> [<日志>…]               # 做不下去：上传附件、评论原因、结束计时、移到错误节点（没有时移到需要人处理的状态）
board flow                                # 工作流：状态的顺序、标识、名称、规则；以及白板所在项目的仓库、集成分支
board new "<标题>" --body "<描述>" [--inbox] [--label <标签>]… [--from <ID>]  # 新建卡片，进入首个可领取状态；--inbox 记到「未整理」最下面；--label 带上已有的标签；--from 记下拆自哪张
                                          # 多行描述用 --body-file <文件>（Windows 上必须，见 work.md「会话开始」第 3 步）；edit 同样
                                          # --image <图片>、--file <文件> 可以写多次：附件随卡片一起建，按顺序接在描述后面（图片直接显示）
board show <ID> --json                    # 全部字段和动态；split_from 拆自哪张，split_into 拆出了哪几张
board ls [--column todo] [--label bug] [--search <文字>]
board comment <ID> "<内容>"               # 多行内容：board comment <ID> --comment-file <文件>
board check --agent <标识> && board heartbeat start   # 不领卡的会话（只建卡、拆解、整理）开始时：占用名字、启动后台心跳（check 单独执行、看结果）
board heartbeat stop                      # 会话结束：停掉后台心跳，名字立刻离线
board doctor --as <名字> --json           # 环境自检；checks 里 board_as 一项是名字是否在线
board next --json                         # 只在 board host 启动的会话里：上一张交审或交给人之后领这个会话排着的下一张；SESSION_DONE 时结束
board base <ID> [--branch <分支>]         # 开工：卡片要由你持有；建好或接着用会话的 worktree，切到最新集成分支（有 wip 分支时切过去 rebase），按输出的 step / next 做
board land <ID> --message-file <文件> --checked "<检查和结果>" [--touched-ok]  # 推送：rebase、跑完检查后用。压成一个提交、touched、hold、推集成分支、删接着做的 wip 分支，输出交审评论四行
board land <ID> --wip                     # 做不下去：没做完的改动提交、推到 wip/<白板标识>/<ID>，输出「未完成的改动：」一行
board touched [--base <提交>]             # 查这次提交有没有删掉别的卡最近加的行、加回别的卡最近删的行（只用 git；board land 里已经带了）
board hold <ID>                           # 确认领取仍有效（board land 里已经带了；发版打 tag 前单独用）；连不上时每 30 秒重试、最多等 5 分钟
board move <ID> <状态标识或名称> [--top]
board label <ID> +<标签> -<标签>         # 标签必须已在项目里存在，先 board labels 看有哪些；「仅人可打」的标签不能加、去（建卡时按 plan.md「拆解和新建」第 4 步可以带）
board labels
board boards                              # 列出所有白板
board files <ID> [--out <目录>]           # 下载卡片的附件到本地，列出路径
board attach <ID> <文件>… [--comment "<说明>" | --comment-file <文件>]   # 上传附件（图片、日志等，≤ 10 MB），作为一条评论
board attach <ID> <截图> --comment-file <交审评论文件>                    # 交审：证据和交审评论放在同一条里
```

## 错误码

所有命令都支持 `--json`。被拒绝时退出码为 1，JSON 里是 `{"error": {"code", "message"}}`。

| 错误码 | 含义 |
|---|---|
| `NO_IDENTITY` / `NO_BOARD` | 没有名字 / 白板：先 `board start <ID> --agent <标识>`（不领卡的会话 `board check --agent <标识>`）；白板用 `--board <项目>/<白板>` |
| `NAME_TAKEN` | 名字正被另一个在线会话用着（`heartbeat start / stop` 得到它：你用了别的会话的名字）：换成自己占用成功的名字，或 `--as` 加一个和本会话相关的后缀 |
| `NAME_OFFLINE` | 名字不在线（心跳中断超过 50 秒）：见 work.md「会话结束」最后一段 |
| `BOARD_NOT_FOUND` | 白板不存在；或项目有多块白板却只写了项目标识（原因里列出了可选的白板） |
| `NOT_FOUND` | 卡片不存在 |
| `EMPTY_TODO` | 可领取的状态里都没有能领的卡片 |
| `NOT_IN_TODO` | 卡片不在可领取状态，也不是你持有的 |
| `ALREADY_HOLDING` | 你已经持有另一张卡片 |
| `NOT_HOLDER` | 卡片被别人持有；`board land`、`board hold`：领取已经失效，不推集成分支，改推 wip 分支（见 work.md「推送」第 3 步） |
| `FORBIDDEN_COLUMN` | 不沿连线移动；直接移进锁定状态；移入、移出终点状态或「未整理」；移出错误节点；从没有连到锁定状态的可领取状态领取；`board submit` 时卡片所在状态连不到交审状态（按原因做：先 `board move` 到下一个锁定状态，或改用 `board giveup`） |
| `EMPTY_TITLE` / `USAGE` | 标题为空 / 命令或参数不对（例如不存在的状态或标签、空评论；`board submit` 的交审评论缺固定的四行，原因里写明缺哪几行） |
| `NOT_LOGGED_IN` | 还没登录：请用户运行 `board login --url <服务端地址> --token <令牌>`（令牌在「Agent → 访问令牌」新建） |
| `INVALID_TOKEN` | 令牌写错或已被吊销：请用户在「Agent → 访问令牌」新建一个，再 `board login --token <令牌>` |
| `UNREACHABLE` | 连不上服务端：告诉用户服务端连不上，先确认地址和网络（`board doctor` 能看出地址来自哪里）；`board land`、`board hold` 返回它时已经等过 5 分钟，按 work.md「推送」第 3 步改推 wip 分支 |
| `DISPATCHED` | 卡片已经派发给一台机器、正在排队（在「进行中」这类锁定状态里，没有持有者），或人强制释放后旧派发还在跑（卡片在可领取状态，没有持有者）：跳过这张，不要领、不要移动 |
| `SESSION_DONE` | `board next`：这个会话不再接着做（排着的做完了、手动派发、有人停止、暂停接单、没有领取规则）：结束会话 |
