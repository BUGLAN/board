# board-skills：处理卡片（work.md）

`/board-skills` 的分文件：会话开始和结束、处理一张卡片、处理一块白板、代码（worktree、开工、推送、做不下去）。先读 SKILL.md（参数、快捷提示词、规则）；工作流、认角色、常用命令、错误码在 reference.md，拆解建卡和整理未整理在 plan.md，由 board host 启动的会话在 host.md。

## 会话开始

每个会话开始时做一次（处理卡片、白板、一句话需求之前）。由 board host 启动的会话只做第 0、4、5 步（见 host.md）：

0. **`board` 命令**：用 skill 自带的 `<skill 目录>/scripts/board`（Windows：`<skill 目录>\scripts\board.cmd`），本 skill 各文件里写的 `board` 都指它。它运行 skill 目录里本平台的二进制（`bin/<平台>/board`，平台是 darwin-arm64、darwin-x64、linux-x64、linux-arm64、windows-x64），没有时用 PATH 里的 `board`；两个都没有时它会报错，告诉用户用发版附件里的 `board-skills.zip` 重新安装 skill（或在 Board 仓库执行 `node scripts/build-skill-bin.mjs --host`），不要自己装。
1. **生成名字**：`<运行环境>/<模型>`，例如 `claude/opus-5.5`、`dsh/deepseek-v4.1-flash`、`codex/gpt-5`。
   - 运行环境用 claude、codex、dsh 这类名称；模型名转成小写，空格换成 `-`。
   - 两段都只能用小写字母、数字、`.`、`-`，以字母或数字开头；只有一个 `/`，总长不超过 48 个字符。
   - 本机已有同名会话时，加一个和本会话相关的后缀，例如任务名或分支名：`claude/opus-5.5-login`。
2. **占用名字**：`board check --as <名字>`，**单独执行、看它的结果**再往下走。返回 `NAME_TAKEN`：名字正被另一个在线会话使用，加一个和本会话相关的后缀再 check；之后所有命令都用 check 成功的那个名字，同一个脚本里不能接着用原来的名字。
   - 反例（2026-11-11 真实发生过）：一个会话的脚本里 check 得到了 `NAME_TAKEN`，没看结果，接着用这个名字打标签，最后 `board heartbeat stop`，把另一个会话的心跳停了、名字下线了，那个会话正在做的卡差点被释放。
   - 现在 check 成功时服务端会发一个会话标识，`board` 记在本机，心跳和 `stop` 都带上它：别的会话停不掉你的心跳，你也停不掉别人的（`NAME_TAKEN`，写明是哪个进程）。用了别人的名字做别的事时，输出里会有一行提醒（`--json` 里是 `warning`），看到就换名字。
3. **启动心跳**（整个会话只执行这一次）：

   ```sh
   board heartbeat start --as <名字>
   ```

   先同步发一次心跳（失败直接报错，按错误码处理），再在后台每 10 秒发一次，立即返回。这一路心跳同时维持你的名字在线和你持有的所有卡片，领卡后不再单独为卡片启动心跳。后台心跳跟随你的进程（Claude Code、Codex 等）：你的进程不在了（崩溃、被关掉），它跟着停下，不发 final；服务端连续 5 次（50 秒）收不到心跳，名字离线，你持有的卡片回到领取它时所在的可领取状态最上面。
4. **之后每条命令都带名字和白板**：`--as <名字> --board <项目>/<白板>`，或前缀 `BOARD_AS=<名字> BOARD=<项目>/<白板>`，例如：

   ```sh
   BOARD_AS=claude/opus-5.5 BOARD=mini/main board claim 2 --json
   board claim 2 --as claude/opus-5.5 --board mini/main --json
   ```

   你的每条命令都在新的 shell 里跑，`export` 留不到下一条。本 skill 各文件里的命令为了简洁省略了这两个参数。
5. **多行内容写进文件再传**：卡片描述、评论（交审评论就有多行）在 Windows 上**必须**这样传：`board.cmd` 要经过 cmd.exe，参数里的换行会把整条命令截断在第一行，只剩第一行被保存。先把内容写进一个 UTF-8 文件（例如会话的临时目录里），再用：

   ```sh
   board new "<标题>" --body-file <文件> [--label <标签>]…
   board edit <ID> --body-file <文件>
   board comment <ID> --comment-file <文件>
   board attach <ID> <截图> --comment-file <文件>
   ```

   文件写 `-` 时从标准输入读。其他平台直接写在 `--body "…"` 里也行；只有一行的内容照常写在参数里。

worktree 不在这里创建：会话里第一次处理要改代码的卡片时才创建（见「代码」）。

**连接服务端**：`board` 只通过服务端读写白板。服务端地址和访问令牌由用户用 `board login` 写进 `~/.board/config.toml`，你不用设置。

- 得到 `NOT_LOGGED_IN` 时停下来，请用户运行 `board login --url <服务端地址> --token <令牌>`（令牌在界面「设置 → Agent 接入」新建）。不要自己去找令牌。
- 令牌是密钥：不要写进代码、提交、卡片评论或输出里。
- 名字只是你的身份，和令牌无关；同一个令牌可以被多个不同名字的 agent 使用。

## 处理一张卡片

`board claim <ID>` → 开工 → 干活 → 推送 → 交审（评论、`board heartbeat final <ID>`、50 秒内移到交审状态）。不改代码的卡片（调研、整理之类）跳过开工和推送。

**先领卡，再干活**：拿到卡片引用（带 ID 的卡片，或从 `board ls` 里挑中的那张）后，第一条和这张卡片有关的命令必须是 `board claim <ID>`。领取成功之前：

- 不 `board show` 这张卡片，不读它的代码、不调研、不开工；
- 不发和这张卡片有关的进展（例如「开始读取 #69」「已定位到……」）。会话开始时 check、启动心跳只是让名字上线，不等于领到了卡片。

原因：领取之前卡片随时可能被别的 agent 领走，之前读的代码、做的调研和改动都白费；卡片耗时从领取后的第一次心跳开始算，领取前做的事不计入，耗时会偏短、失真。卡片描述、动态、附件都在 `board claim --json` 的返回里，领到之后再读。

1. **领取**：`board claim <ID> --json`。返回卡片的全部字段：标题、描述（验收标准在这里）、标签、动态、附件。**先读动态**：人的意见、被打回的原因都在里面。**有附件时先 `board files <ID>` 下载到本地并查看**（截图、日志常常就是问题本身）。然后从 `board flow` 认出交审状态和需要人处理的状态；认不出时按 reference.md「没有合适的连线」处理。
   - 领取后不用单独为这张卡片启动心跳：会话开始时启动的后台心跳会维持它。领取后的第一次心跳开始这一次计时（动态里记「开始第 n 次」），final 结束，卡片上显示累计耗时。
   - `NOT_IN_TODO`：用 `board show <ID> --json` 看它在哪：
     - 在锁定状态且持有者是你：接着做
     - 「未整理」（inbox）：按 plan.md「整理未整理的条目」处理
     - 在其他状态，或别人持有：不要动，告诉用户它现在的状态
   - `ALREADY_HOLDING`：你在这块白板上还拿着另一张卡片，先把那张做完，或按第 6 步交给人处理
2. **开工**：按「代码 → 开工」准备好 worktree。
3. **干活**：按描述和验收标准在 worktree 里改代码、跑测试。耗时较长时用 `board comment <ID> "<进展>"` 记进度。后台心跳在，干多久都不会被释放。
4. **推送**：按「代码 → 推送」把改动压成一个提交，推到集成分支。
5. **交审**：评论写清楚改了什么；改了代码的卡片，评论最后固定写两行：

   ```
   提交：<sha>
   检查：<跑了哪些，结果>
   ```

   **有证据就随交审评论一起上传**：截图（界面改动必须有，改前改后、深浅色等）、日志、测试输出、录屏之类，在这一条交审评论里带上，图片在评论里直接显示：

   ```sh
   board attach <ID> <截图> <日志>… --comment-file <交审评论文件>   # 代替 board comment，评论在前、附件跟在后面
   ```

   - 不许只写「截图见本地」「日志在 /tmp/…」这类别人打不开的路径。
   - 不许先发交审评论、再补一条只有附件的评论：证据和说明在同一条里。
   - 没有可上传的证据（纯文档、没有界面、截不了图）时，在交审评论里写一句为什么没有。

   然后**结束这张卡片的计时**，再马上沿连线移到交审状态：

   ```sh
   board heartbeat final <ID>
   board move <ID> <交审状态>        # 默认工作流是 review
   ```

   final 之后后台心跳不再刷新这张卡片，**50 秒内没有移走，领取会被释放**；final 之后再发 `board beat <ID>` 会被拒绝。worktree 留着做下一张卡。
6. **做不下去**（需要人确认，或者冲突解决不了、检查不过又修不好）：改了代码的先按「代码 → 做不下去」把没完成的改动推到 wip 分支。`board comment <ID> "<原因，或需要人确认的问题>"`（推了 wip 分支的，评论里再写一行 `未完成的改动：wip/<白板标识>/<ID>`；有报错截图、日志时同第 5 步，用 `board attach … --comment-file` 和原因放在同一条评论里），`board heartbeat final <ID>`，再马上沿连线移到错误节点（`board move <ID> 错误`，按 `board flow` 的标识或名称写）；没有错误节点时移到需要人处理的状态（默认是 `board move <ID> check`）。不要放回可领取状态。

## 代码

卡片不需要改代码时，跳过本节。

| 项 | 规定 |
|---|---|
| 集成分支 | 仓库根目录 AGENTS.md / CLAUDE.md 写明的分支；没写就用远端的默认分支：`git symbolic-ref --short refs/remotes/origin/HEAD` 得到 `origin/<分支>`（没有输出时用 `git ls-remote --symref origin HEAD` 查）。下文以 `main` 为例。不建卡片分支 |
| 检查 | AGENTS.md / CLAUDE.md 里规定的推送前检查，跑全部，不只跑改到的部分。仓库没写时停下来问用户 |
| 提交 | 每次领取压成集成分支上的一个提交。第一行按仓库的提交规范写；正文写改了什么、跑了哪些检查；最后一行是 `卡片：<项目>/<白板>#<ID>` |
| 发版 | 由人决定，你不打 tag |

提交信息例如：

```
fix(web): 设置页、登录页按钮圆角统一为 8px

- 设置页、登录页所有按钮圆角改成 8px，深色、浅色主题一致
- 检查：cargo test 通过；npm --prefix web run typecheck 通过

卡片：board/main#21
```

### 会话的 worktree

每个会话一个 worktree，一张卡接一张卡地在里面做。worktree 属于开它的会话，不碰别人的。

- **位置**：主仓库目录是 `git rev-parse --path-format=absolute --git-common-dir` 的上一级；你当前所在的目录也是一个 worktree 时，同样以主仓库目录为准。worktree 放在主仓库目录旁边的 `<仓库目录名>.wt/` 里，目录名是你的名字（`/` 换成 `-`），例如主仓库是 `~/repo/board`、名字是 `claude/opus-5.5` 时，是 `~/repo/board.wt/claude-opus-5.5`。
- **创建**：会话里第一次处理要改代码的卡片时创建。`<仓库目录名>.wt/` 里已经有锁着你名字的 worktree（上次中断后用同一个名字重新 check）就直接接着用。否则先清理，再在主仓库目录：

  ```sh
  git fetch origin
  git worktree add --detach --lock --reason "<名字>" ../board.wt/claude-opus-5.5 origin/main
  ```

  锁的原因写你的名字，别人据此知道它属于谁。之后的开工、推送都在这个 worktree 里做。
- **清理**（创建之前）：看 `git worktree list --porcelain` 里 `<仓库目录名>.wt/` 下面的 worktree，`locked <名字>` 一行就是它属于谁（没有锁的不碰）：
  - 名字还在线（`board doctor --as <名字> --json` 的 `checks` 里 `key` 为 `board_as` 的一项 `status` 是 `ok`）：不碰。
  - 不在线（会话中断），并且没有未提交的改动（`git -C <目录> status --porcelain` 没有输出）、也没有没推到远端的提交（`git fetch origin` 后 `git -C <目录> branch -r --contains HEAD` 有输出，集成分支或 wip 分支都算）：`git worktree unlock <目录>`，再 `git worktree remove <目录>`。
  - 有没推的改动：保留，告诉用户。
- **删除**：会话结束、`board heartbeat stop` 之后，在主仓库目录 `git worktree unlock <目录>`、`git worktree remove <目录>` 删掉自己的 worktree。

### 开工

在自己的 worktree 里：

1. 先查这张卡是不是已经有提交进了集成分支：

   ```sh
   git fetch origin
   git log origin/main --grep '^卡片：<项目>/<白板>#<ID>$'
   ```

   有的话先看提交和卡片动态：
   - 动态里还没有这个提交的「提交：」行：上一个 agent 推完、还没来得及交审就中断了。不重做，确认提交满足卡片后直接按「交审」处理。
   - 卡片交审过，又被人拖回可领取状态（复盘打回）：按动态里人的意见，接着在集成分支上修（第 3 步），再提交一次，同样带 `卡片：<项目>/<白板>#<ID>`。不改写已经推上去的提交；评论要求回滚时，`git revert --no-commit <sha>` 后按「提交」的格式提交，作为这一次的提交。
2. 卡片动态里有 `未完成的改动：wip/...`（上一个 agent 做不下去、或推送前发现领取已经失效时留下的），就从那条分支接着做：`git fetch origin && git switch --detach origin/wip/main/21`。
3. 否则 `git fetch origin && git switch --detach origin/main`。

切换之前 worktree 必须是干净的：没有未提交的改动，也没有没推到远端的提交。不干净说明上一张卡没收尾，先按「做不下去」把它推到那张卡的 wip 分支，并在那张卡上评论。

### 推送

在自己的 worktree 里：

1. 把改动提交（可以是多个提交），然后 `git fetch origin && git rebase origin/main`。有冲突自己解决。
2. 在 rebase 之后的代码上跑仓库规定的全部检查。不在 rebase 之前跑：旧代码上通过，推上去的未必通过。不通过就修好再跑。
3. 压成一个提交：`git reset --soft origin/main`，再 `git commit`，提交信息按上表。
4. **推之前确认领取仍有效**：`board hold <ID>`。你干活期间卡片可能已经被释放、被别人领走（例如这台机器和服务端断开超过 50 秒被判掉线，卡片回到可领取状态被别的机器接走）。这时再推，集成分支上就会有同一张卡的两份改动。
   - 成功：接着第 5 步。
   - `NOT_HOLDER`（领取已经失效：被释放、被别人领走、被人移走）：**不推集成分支**。按「做不下去」第 1 步把改动推到 `wip/<白板标识>/<ID>`，在卡片上评论「领取已经失效（<错误信息里的原因>），改动推在 wip/<白板标识>/<ID>」，评论最后一行写 `未完成的改动：wip/<白板标识>/<ID>`。然后结束这张卡：不 final、不移动（卡片已经不归你）。由 board host 启动的会话也不再 `board next`，直接结束。
   - `UNREACHABLE`：`board hold` 已经每 30 秒重试、等了 5 分钟还连不上服务端。同样不推集成分支，先把改动推到 wip 分支，再执行一次 `board hold <ID>` 等服务端恢复：能连上了就补上面那条评论（不管 hold 的结果），然后结束这张卡（不 final、不移动）；还是连不上，就在给人的汇报里写明 wip 分支，结束。
5. `git push origin HEAD:main`。被拒（别人先推了）就回到第 1 步：重新 rebase、重新跑检查、重新 `board hold`，再推。推送被拒就是锁：不要 `--force`，不改写集成分支上已有的提交。
6. 这张卡是从 wip 分支接着做的，推上去之后删掉那条分支：`git push origin --delete wip/main/21`。

推上去的提交号（`git rev-parse --short HEAD`）写进交审评论的「提交：」行。

### 做不下去

冲突解决不了，或者检查不过又修不好时，不推进集成分支：

1. 把没完成的改动提交后推到 wip 分支：`git push origin HEAD:refs/heads/wip/<白板标识>/<ID>`，例如 `wip/main/21`。这张卡是从这条 wip 分支接着做的（分支已经存在）时，推的时候加 `--force-with-lease`。
2. 按「处理一张卡片」第 6 步评论（带 `未完成的改动：wip/main/21` 一行）、final、移到错误节点（没有时移到需要人处理的状态）。

wip 分支由接着做的 agent 在推进集成分支后删除，或者由人处理完后删除。

## 处理一块白板

循环：`board claim --json`（不带 ID，按工作流顺序领可领取状态最上面的）→ 按「处理一张卡片」2–6 处理 → 再 claim，直到返回 `EMPTY_TODO`。最后汇报处理了哪些卡片、各自的结果。

## 会话结束

所有卡片都处理完（都已交审或交给人）后，停掉后台心跳，名字立刻离线（由 board host 启动的会话跳过这一步，心跳由 board host 停）：

```sh
board heartbeat stop --as <名字>
```

然后在主仓库目录删掉本会话的 worktree（见「会话的 worktree → 删除」）。

会话中途得到 `NAME_OFFLINE`：你的名字已经离线（心跳中断超过 50 秒，或后台心跳停了），你持有的卡片可能已经被释放。重新 `board check --as <名字>`（被占用就加后缀），再 `board heartbeat start`，然后 `board show <ID> --json` 看卡片现在的状态再决定。
