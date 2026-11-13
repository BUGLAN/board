# board-skills：由 board host 启动的会话（host.md）

`/board-skills` 的分文件：环境变量里有 `BOARD_HOST` 时先读这一份，再按它说的读 work.md。

## 由 board host 启动的会话

Board 可以把卡片派给一台机器：那台机器上的 `board host` 先以生成的名字（`<agent 标识>-<机器名>/<模型名>`，例如 `dsh-win/deepseek-v4.1-flash`）check、领取卡片，再启动你，提示词就是「开始执行 X项目之中的Y白板中的#Z任务」那一行；心跳由 board host 替你发。

**怎么认**：环境变量里有 `BOARD_HOST`（机器名），并且 `board show <ID> --json` 的 `holder` 是 `BOARD_AS`。这时：

- **不 check 名字、不 claim、不启动也不停止心跳**：跳过 work.md「会话开始」第 1–3 步、work.md「处理一张卡片」第 1 步里的 `board claim`（改用 `board show <ID> --json` 读卡片，同样先读动态、下载附件）、work.md「会话结束」里的 `board heartbeat stop`。
- 名字和白板已经在环境变量 `BOARD_AS`、`BOARD` 里，命令不用再带 `--as`、`--board`。
- 其余步骤不变：worktree、干活、提交推送、交审（`board heartbeat final <ID>` 后马上沿连线移到交审状态），或者做不下去（评论、`board heartbeat final <ID>`、移到错误节点；没有错误节点时移到需要人处理的状态）。
- 会话里没有人和你对话：要停下来问用户的情况（需要人确认、`NOT_LOGGED_IN` 之类），按 work.md「做不下去」处理，把问题写进评论，不要等回答。
- **不要创建 goal**：board host 的 agent 是非交互的一次性运行；goal 的自动续轮会在任务结束后继续跑，dsh 的 headless 会因此以退出码 1 结束。用 `todo_write` 记进度就够了。
- **接着做下一张**：交审或做不下去（卡片已经不由你持有）之后执行 `board next --json`：
  - 返回卡片：服务端已经结算上一张、以你的名字领好了这个会话排着的下一张（开会话时就排好了，多半和第一张标签相近）。按 work.md「处理一张卡片」第 2–6 步接着做：不 claim，接着用同一个 worktree（开工时照样先切到最新的集成分支）。做完再 `board next --json`。
  - 返回 `SESSION_DONE`：这个会话不再接着做（排着的卡片都做完了或取消了、手动派发的会话、有人停止、暂停接单、机器已经没有领取规则）：结束。
  - 返回 `ALREADY_HOLDING`：上一张还由你持有，先交审或交给人处理。
  - 觉得上下文已经不够做好下一张时，可以不执行 `board next`，直接结束：排着的会被取消，由新会话接手。
- 不要用 `board claim` 领别的卡片。你退出后，卡片仍由你持有时 board host 会把它移到错误节点（没有时移到需要人处理的状态），记为异常退出。

`BOARD_HOST` 有、但卡片不归 `BOARD_AS` 持有（被人强制释放、已经移走）时，不要动这张卡片，评论说明后直接结束。拆解的提示词（「拆解 …#Z需求」）除外：拆解本来就不领取，按下一节做。

## 由 board host 启动的拆解

「未整理」里的条目也能派给一台机器拆解：提示词是「拆解 X项目之中的Y白板中的#Z需求」那一行。board host 已经以生成的名字 check、替你发心跳，**这一条不由你持有**（拆解不领取、不移动它）。这时：

- 不 check 名字、不 claim、不启动也不停止心跳；名字和白板在 `BOARD_AS`、`BOARD` 里。
- 按 plan.md「拆解未整理里的需求」做：每张 `board new --inbox --from Z`，最后在 #Z 上评论拆成了哪几张。不改、不移动 #Z。
- 不开 worktree、不改代码、不 `board next`：评论完直接结束，退出码 0。board host 按这次建了几张「拆自 #Z」的卡记结果（已拆解 / 没拆出卡）；异常退出时它会在 #Z 上评论。
- 信息不够、拆不了：评论写明缺什么，不建卡，直接结束。
