# board-skills：处理卡片（work.md）

`/board-skills` 的分文件：会话开始和结束、处理一张卡片、处理一块白板、代码（worktree、开工、推送、做不下去）。先读 SKILL.md（参数、快捷提示词、规则）；工作流、命令、错误码在 reference.md，拆解建卡和整理未整理在 plan.md，发版在 release.md，由 board host 启动的会话在 host.md。

## 会话开始

每个会话开始时做一次（处理卡片、白板、一句话需求之前）。由 board host 启动的会话只做第 0、2、3 步（见 host.md）：

0. **`board` 命令**：用 skill 自带的 `<skill 目录>/scripts/board`（Windows：`<skill 目录>\scripts\board.cmd`），本 skill 各文件里写的 `board` 都指它。它运行 skill 目录里本平台的二进制（`bin/<平台>/board`，平台是 darwin-arm64、darwin-x64、linux-x64、linux-arm64、windows-x64），没有时用 PATH 里的 `board`；两个都没有时它会报错，告诉用户用发版附件里的 `board-skills.zip` 重新安装 skill（或在 Board 仓库执行 `node scripts/build-skill-bin.mjs --host`），不要自己装。它说这个 `board`「是旧版，没有 start / submit / giveup / land / base」或「运行不了」时，同样停下来把这句话告诉用户，不要改用别的命令拼出同样的步骤。
1. **名字和心跳**：处理卡片的会话第一次 `board start <ID> --agent <你实际运行的 agent 标识>`（claude、codex、dsh）时，Board 按这台机器的设置生成名字：`<机器名>/<模型名>-<推理强度>`（例如 `win/opus-5.5-high`），机器上没给这个 agent 设模型时第二段是 agent 标识（`mbp/codex`），认不出机器时是 `<agent>/<agent>`（`claude/claude`）；名字被别的在线会话用着时在末尾加 `-1`、`-2`……。名字记在本机，后台心跳随之启动（跟随你的进程，维持名字在线和你持有的卡片），输出里写着名字（`--json` 里是 `session.name`）；之后的命令不用再带 `--agent`、`--as`。输出说「没找到 agent 进程」（`session.follows` 为 false）时，心跳会一直跑到 `board heartbeat stop`：会话结束时一定要执行。
   - 不领卡的会话（只建卡、拆解、整理）：`board check --agent <标识>`，**单独执行、看结果**，成功后 `board heartbeat start`（整个会话一次）。
   - `--as` 自己起的名字得到 `NAME_TAKEN`（正被另一个在线会话用着）：加一个和本会话相关的后缀重新来；输出里有用了别人名字的提醒（`--json` 里是 `warning`）时，换成自己成功占用的名字。之后所有命令都用成功的那个。
2. **之后每条命令带白板**（名字不写时用记在本机的那个）：`--board <项目>/<白板>`，或前缀 `BOARD=<项目>/<白板>`；想明确写名字时也带 `--as <名字>` 或前缀 `BOARD_AS=<名字>`，例如：

   ```sh
   BOARD=mini/main board start 2 --agent dsh --json
   board comment 2 "进展" --board mini/main --as win/opus-5.5-high
   ```

   你的每条命令都在新的 shell 里跑，`export` 留不到下一条；名字记在本机的状态文件里，不用每条都带。本 skill 各文件里的命令为了简洁省略了这两个参数。
3. **多行或带双引号的文字写进文件再传**：卡片描述、评论（交审评论就有多行）在 Windows 上有换行或双引号 `"` 时**必须**这样传：`board.cmd` 要经过 cmd.exe，参数里的换行会把整条命令截断在第一行；单行里同时有 `"` 和 `& | < >` 时，PowerShell 转义出来的 `\"` 在 cmd 看来可能是没配对的引号，后面的文字会被截断或当成重定向。先把内容写进一个 UTF-8 文件（例如会话的临时目录里），再用：

   ```sh
   board new "<标题>" --body-file <文件> [--label <标签>]…
   board edit <ID> --body-file <文件>
   board comment <ID> --comment-file <文件>
   board submit <ID> --comment-file <文件> [<截图>…]
   board giveup <ID> --comment-file <文件> [<日志>…]
   ```

   文件写 `-` 时从标准输入读。其他平台直接写在 `--body "…"` 里也行；Windows 上只有一行、也不含 `"` 的内容才照常写在参数里。

worktree 不在这里创建：由「代码 → 开工」的 `board base` 建。

**连接服务端**：`board` 只通过服务端读写白板。服务端地址和访问令牌由用户用 `board login` 写进 `~/.board/config.toml`，你不用设置。

- 得到 `NOT_LOGGED_IN` 时停下来，请用户运行 `board login --url <服务端地址> --token <令牌>`（令牌在界面「Agent → 访问令牌」新建）。不要自己去找令牌。
- 令牌是密钥：不要写进代码、提交、卡片评论或输出里。

## 处理一张卡片

`board start <ID>` → 开工 → 干活 → 推送 → 交审（`board submit <ID>`）；做不下去时 `board giveup <ID>`。领到之后按卡片内容分三种，走哪几步不一样：

| 卡片 | worktree | 流程 |
|---|---|---|
| 改仓库里的文件：功能、修复，以及文案、文档、图片 | `board base` 给你一个（接手空闲的，没有才新建） | 开工 → 干活 → 推送 → 交审 |
| 只测试、验证：跑测试、验收、复现问题，不改文件 | 同上。测的是最新的集成分支，不在主仓库目录里测（那里的代码可能是旧的，也可能是人正在改的） | 开工 → 测试 → `board submit <ID> --no-code`，不推送 |
| 不碰仓库：调研、整理、拆解 | 不用，不执行 `board base` | 干活 → `board submit <ID> --no-code` |

**先领卡，再干活**：拿到卡片引用（带 ID 的卡片，或从 `board ls` 里挑中的那张）后，第一条和这张卡片有关的命令必须是 `board start <ID>`。领取成功之前：

- 不 `board show` 这张卡片，不读它的代码、不调研、不开工；
- 不发和这张卡片有关的进展（例如「开始读取 #69」「已定位到……」）。名字上线不等于领到了卡片。

原因：领取之前卡片随时可能被别的 agent 领走，之前读的代码、做的调研和改动都白费；卡片耗时从领取后的第一次心跳开始算，领取前做的事不计入，耗时会偏短、失真。卡片描述、动态、附件都在 `board start --json` 的返回里，领到之后再读。

1. **领取**：`board start <ID> --agent <你实际运行的 agent 标识> --json`（会话里第一次带 `--agent`，之后可以不带；由 board host 启动的会话不用带）。一条命令做完：这个会话还没有名字时占用名字并启动后台心跳（见「会话开始」第 1 步）、领取、下载附件，然后返回卡片的全部字段：标题、描述（验收标准在这里）、标签、动态。**先读动态**：人的意见、被打回的原因都在里面。返回里还有：
   - `files`：附件已经下载到本地，每个带 `path`。**有附件先看**（截图、日志常常就是问题本身）。
   - `review`、`giveup`：交审、做不下去时卡片会去的状态（`board submit`、`board giveup` 自己移过去）；`giveup` 为空表示没有错误节点、也没有需要人处理的状态。
   - `next_lock`：卡片所在锁定状态连着的下一个锁定状态（两级锁定的白板，例如 进行中 → 测试中 → 待审核）。`review` 为空、`next_lock` 不为空时，做完先 `board move <ID> <next_lock 的名称>`，到那里再交审，不要 giveup。
   - `repo`、`branch`：项目的代码仓库和集成分支（`branch` 为空时按「代码」一节的顺序认）。
   - `session.resumed` 为 true：这张卡本来就在你手里（上次没做完的），接着做。
   - 领取后的第一次心跳开始这一次计时（动态里记「开始第 n 次」），交审或做不下去时结束，卡片上显示累计耗时。
   - `NOT_IN_TODO`：用 `board show <ID> --json` 看它在哪：
     - 「未整理」（inbox）：按 plan.md「整理未整理的条目」处理
     - 在其他状态，或别人持有：不要动，告诉用户它现在的状态
   - `ALREADY_HOLDING`：你在这块白板上还拿着另一张卡片，先把那张做完，或按第 6 步交给人处理
2. **开工**：`board base <ID> --json`，按它的 `next` 做（见「代码 → 开工」）。
3. **干活**：按描述和验收标准在 worktree 里改代码。这一步只跑和改动相关的测试；仓库规定的全部检查留到「推送」里 rebase 之后跑一次，不要在这里先跑一遍（上游一更新就白跑了）。耗时较长时用 `board comment <ID> "<进展>"` 记进度。后台心跳在，干多久都不会被释放。
4. **推送**：按「代码 → 推送」rebase、跑检查，再用 `board land` 压成一个提交推到集成分支。
5. **交审**：评论写清楚改了什么；改了代码的卡片，评论最后固定写四行：

   ```
   提交：<sha>
   集成分支：<推到的分支>
   检查：<跑了哪些，结果>
   碰到的别的卡：<board touched 的最后一行；有的话逐条写是不是有意的、为什么>
   ```

   这四行照抄 `board land` 最后的输出（见「代码 → 推送」第 3 步）。「碰到的别的卡」一行让复盘的人看出这次提交有没有动到别的卡刚做的改动。

   **交审用一条命令**，证据作为文件参数一起带上（截图、日志、测试输出、录屏之类；界面改动必须有截图，改前改后、深浅色等），图片在评论里直接显示：

   ```sh
   board submit <ID> --comment-file <交审评论文件> [<截图> <日志>…]
   ```

   它上传附件、评论、结束计时、移到交审状态，一步做完。

   - 不许只写「截图见本地」「日志在 /tmp/…」这类别人打不开的路径。
   - 不许先交审、再补一条只有附件的评论：证据和说明在同一条里。
   - 没有可上传的证据（纯文档、没有界面、截不了图）时，在交审评论里写一句为什么没有。
   - 被拒绝并写着「交审评论缺这几行」：改了代码的卡片，上面四行一行都不能少，补上再交；这张卡确实没改代码（调研、整理）时加 `--no-code`。
   - `FORBIDDEN_COLUMN`，原因里写「下一步是锁定状态「…」：先 board move …」：两级锁定的白板，这一级连不到交审状态。照它 `board move` 到下一个锁定状态，再执行同一条 `board submit`。
   - `FORBIDDEN_COLUMN`，原因里建议 `board giveup`：这块白板没有交审状态。按第 6 步 `board giveup`，评论里写明已经做完、推到了哪个提交，等人处理。

   交审之后卡片不再由你持有。worktree 留着做下一张卡。
6. **做不下去**（需要人确认，或者冲突解决不了、检查不过又修不好）：改了代码的先按「代码 → 做不下去」把没完成的改动推到 wip 分支。然后一条命令交给人：

   ```sh
   board giveup <ID> --comment-file <原因文件> [<报错截图> <日志>…]
   ```

   评论写原因，或需要人确认的问题（推了 wip 分支的，再写一行 `未完成的改动：wip/<白板标识>/<ID>`）。它上传附件、发评论、结束这张卡片的计时、把卡片移到错误节点（没有错误节点时移到需要人处理的状态）。输出里说「没有错误节点和需要人处理的状态」或 `--json` 返回 `moved: false` 时，卡片没有移走，会在 50 秒后回到领取前的状态：停下来告诉用户。不要自己把卡片放回可领取状态。

**做卡时发现的新需求**：干活时发现的、不属于这张卡的改动（顺手看到的 bug、要另做的功能、文档缺口），一律 `board new "<标题>" --inbox --body-file <描述文件>` 记进「未整理」，不直接建进可领取状态，也不顺手在这张卡里做；描述里写明是做哪张卡时发现的（例如 `做 default/main#12 时发现：…`）。放不放进工作流由人决定。

## 代码

卡片不需要改代码时，跳过本节。

| 项 | 规定 |
|---|---|
| 集成分支 | 按这个顺序认：项目设置里的「集成分支」（`board flow --json` 的 `branch`，不是空的时候）→ 仓库根目录 AGENTS.md / CLAUDE.md 写明的分支 → 远端的默认分支（`git symbolic-ref --short refs/remotes/origin/HEAD` 得到 `origin/<分支>`；没有输出时用 `git ls-remote --symref origin HEAD` 查）。下文以 `main` 为例。不建卡片分支 |
| 检查 | AGENTS.md / CLAUDE.md 里规定的推送前检查，跑全部，不只跑改到的部分。仓库没写时停下来问用户 |
| 快检 | AGENTS.md / CLAUDE.md 里写的「快检」（把代码编译一遍、类型检查之类，不跑测试）。只在「推送」第 3 步用：全部检查通过之后上游又更新了、rebase 没有冲突时，用它代替重跑全部检查。仓库没写时没有快检，照旧全部重跑 |
| 更新代码后 | AGENTS.md / CLAUDE.md 里「更新代码后」一节写的命令（重装依赖之类）：每次 `board base` 之后、动手之前执行。仓库没写就不做 |
| 提交 | 每次领取压成集成分支上的一个提交。第一行按仓库的提交规范写；正文写改了什么、跑了哪些检查；最后一行是 `卡片：<项目>/<白板>#<ID>` |
| 发版 | 由人决定：不主动发版、不顺手打 tag、不改版本号；人新建的卡明确要求发版时读 release.md |

提交信息例如：

```
fix(web): 设置页、登录页按钮圆角统一为 8px

- 设置页、登录页所有按钮圆角改成 8px，深色、浅色主题一致
- 检查：cargo test 通过；npm --prefix web run typecheck 通过

卡片：board/main#21
```

### 会话的 worktree

每个会话一个 worktree，一张卡接一张卡地在里面做，都在主仓库旁边的 `<仓库目录名>.wt/` 里，锁着谁的名字就是谁在用（规则和 board host 一样）。**用完留着，下一个会话接手**：`board base` 先找锁着你名字的，没有就接手一个别的会话留下的空闲 worktree（没锁的，或者锁着的名字已经离线、里面又没有没推的改动），都没有才新建。接手来的 worktree 里依赖、构建产物都在，不用从头安装、从头编译。

- 找、接手、新建、清理都由 `board base` 做，不要自己 `git worktree add` / `lock` / `remove`，不碰别人锁着的 worktree。
- 会话结束、`board heartbeat stop` 之后，在主仓库目录 `git worktree unlock <目录>` 把自己的解锁，**不删**；board host 备好的（提示词里写着「代码已经备好」）由 board host 解锁，你不用管，见 host.md。
- 空闲的 worktree 超过 36 小时没用、或者多于 4 个时，`board base` 和 board host 会自己清理（输出里的「清理空闲」、`--json` 的 `cleaned`），不用你删。

### 开工

每张要用仓库的卡（改文件的、只测试的）动手之前（第一张、`board next` 接到的下一张、从 wip 分支接着做的都一样）在项目的仓库里执行 `board base <ID> --json`（集成分支和项目设置里的不同时加 `--branch <分支>`）。它确认卡片由你持有、`git fetch`、找到你的 worktree（见上一节：接着用、接手空闲的或新建，`--json` 里 `adopted` / `created`），切到最新的集成分支（评论里有 `未完成的改动：wip/…` 时切到那条分支、rebase 上去），输出 `worktree`、`base`、`next`。之后改代码、测试、推送都在 `worktree` 里。先按仓库 AGENTS.md / CLAUDE.md「更新代码后」一节执行（没写就跳过），再按 `step` 做：

- `code`：直接改代码。
- `wip`：先看 `git diff origin/main...HEAD` 和动态里上一次留下的说明，弄清做到哪了再接着做，不要从头重做。
- `resolve`（退出码 1）：wip 分支 rebase 时冲突。按「推送」第 1 步的规则逐块合，`git add` 后 `git rebase --continue`；合不了就 `git rebase --abort`，按「做不下去」交给人。
- `submit`：已经推到集成分支、动态里还没有交审（上一个 agent 推完就中断了）。不重做，确认提交满足卡片后直接交审。
- `rework`：交审过又被打回。按动态里人的意见接着修，再提交一次；不改写推上去的提交，要回滚时 `git revert --no-commit <sha>`。
- `save`（退出码 1）：worktree 里有没推的改动，没有切换。看是谁的：
  - 上一张卡没收尾的：`board land <上一张卡 ID> --wip`，在那张卡上评论它输出的那一行；
  - 就是这张卡从 wip 接着做的：`board land <ID> --wip`；
  - 分不清、或与本卡无关：`git add -A`、`git commit` 后推成快照 `wip/<白板标识>/<ID>-presync-<短提交号>`，在这张卡上评论「开工前发现与本卡无关的未收尾改动，未带入本卡，已保存到 <分支>」。

  不要 `git stash` 后丢掉，不要 `reset --hard`、`checkout -- .`、`clean`；保存失败就按「做不下去」说明，现场原样保留。保存好再 `board base`。

不要跳过：在旧代码上开工，rebase 时会悄悄撤回别的卡刚推的改动（#328）。`board land` 里的 `board touched` 靠 `board base` 那一下切换认开工时的基点。

### 推送

在自己的 worktree 里：

1. 把改动提交（可以是多个提交），然后 `git fetch origin && git rebase origin/main`。冲突自己解：
   - **上游的改动必须保留**：那一边是别的卡推上去的改动。不整份取自己那一版（`git checkout --theirs`、「接受自己的」），逐块把自己的改动叠在上游的新代码上；先 `git log -5 --format='%h %s%n%b' origin/main -- <文件>` 看它最近被哪些卡改过。
   - 两边都新建的同名文件（尤其测试）合成一份；不删、不整份替换别的卡的测试用例（卡片要求的除外，交审评论写明）。
   - 合不了（看不懂上游为什么这么改、两边要求矛盾）：按「做不下去」交给人，不要猜。
2. 在 rebase 之后的代码上跑仓库规定的全部检查，不通过就修好再跑。全部检查只在这里跑：先更新、再检查。
3. `board land <ID> --message-file <提交说明文件> --checked "<跑了哪些检查、结果>"`：确认 HEAD 在最新集成分支之上，压成一个提交（补上「卡片：」行），查 `board touched`、`board hold`，推集成分支，删掉接着做的 wip 分支，最后输出交审评论的四行。按结果：
   - 「不在最新的 origin/main 之上」、推送被拒：检查期间别人先推了。重新 `git fetch origin && git rebase origin/main`，不要 `--force`，然后：
     - rebase 没有冲突、仓库写了「快检」：只跑快检，通过就再 `board land`，不重跑全部检查。`--checked` 里写明，例如 `npm test 通过（在 1a2b3c4 上）；之后 rebase 过 2 个提交，只跑了快检`。快检不过：修好，回到第 2 步跑全部检查。
     - rebase 有冲突，或者仓库没写「快检」：解完冲突，回到第 2 步跑全部检查。
   - 碰到别的卡（没有推）：不是有意的（多半是解冲突时取了自己那一版），对照 `git show <它列的提交> -- <文件>` 找回来，回到第 2 步；有意的（卡片要求、同一功能接着改），加 `--touched-ok` 再执行，交审评论那一行后面逐条写为什么。
   - `NOT_HOLDER`、`UNREACHABLE`（领取失效、连不上，没有推）：`board land <ID> --wip`，在卡片上评论原因，最后一行照抄它输出的 `未完成的改动：…`（连不上就等能连上再补，还不行写进给人的汇报），然后结束这张卡：不 submit、不 giveup。board host 启动的会话也不再 `board next`。

### 做不下去

冲突解决不了、检查不过又修不好时，不推集成分支：`board land <ID> --wip` 把没完成的改动提交、推到 `wip/<白板标识>/<ID>`，再按「处理一张卡片」第 6 步 `board giveup`，评论里带上它输出的 `未完成的改动：…` 一行。wip 分支由接着做的 `board land` 删掉，或者由人处理完后删掉。

## 处理一块白板

循环：`board start --json`（不带 ID，按工作流顺序领可领取状态最上面的；会话里第一次带 `--agent <标识>`）→ 按「处理一张卡片」2–6 处理 → 再 `board start`，直到返回 `EMPTY_TODO`。最后汇报处理了哪些卡片、各自的结果。

## 会话结束

所有卡片都处理完（都已交审或交给人）后，停掉后台心跳，名字立刻离线（由 board host 启动的会话跳过这一步，心跳由 board host 停；一张卡都没领过、也没 check 过的会话没有名字，也跳过）：

```sh
board heartbeat stop
```

然后在主仓库目录把本会话的 worktree 解锁、留给下一个会话：`git worktree unlock <目录>`，不删（见「代码 → 会话的 worktree」）。里面还有没推的改动时不解锁：先按「代码 → 做不下去」保存。

会话中途得到 `NAME_OFFLINE`（名字离线了，持有的卡可能已经被释放）：`board start <ID> --agent <标识>` 重新占名字、启动心跳，卡还在就接着做；返回 `NOT_IN_TODO` 说明卡已经被别人拿走或移走，告诉用户。
