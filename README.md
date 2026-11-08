<p align="center">
  <img src="src-tauri/icons/icon.png" width="96" alt="Board">
</p>

<h1 align="center">Board</h1>

<p align="center">
  一块给人看、也给 agent 干活的看板。<br>
  人在界面上排优先级、审核结果；agent 用命令行领取任务、记录进度、交审。
</p>

<p align="center">
  <a href="https://board.buglan.com"><strong>在线实例</strong></a> ·
  <a href="#接入-agent">接入 agent</a> ·
  <a href="#cli-速查">CLI 速查</a>
</p>

<p align="center">
  <img src="docs/images/board.webp" alt="Board 界面">
</p>

## 为什么

让一个 agent 干活不难；难的是同时带几个 agent 时，人还能随时知道——

- 现在有哪些事在做，分别是谁在做？
- 哪些事已经做完、等着审？
- 哪些 agent 掉线了，任务该回到队列？
- 下一步该做什么，优先级由谁定？

Board 把这些问题的答案放到一块**人和 agent 都能读写**的白板上。两边看到的是同一份数据，任何一方的改动都会实时出现在另一方的界面上。

它只管两件事：**卡片在哪个状态、谁在做**。agent 怎么干活、什么时候来领，不归 Board 管。

## 适合什么场景

- 一个人同时带几个 coding agent，各自领任务、并行做事；人只负责排优先级和验收。
- 维护开源项目：issue 监听、社区反馈、定期清理都可以先在 Board 之外发现，再用 CLI 把任务写进来。
- 把零散想法先记进「未整理」，想清楚后再整理成带验收标准的卡片。

## 核心能力

- **人和 agent 共用一块白板**：界面和 CLI 操作同一份数据；CLI 新建一张卡片，已打开的界面 2 秒内出现。
- **领取互斥，不会重复开工**：多个 agent 同时领取同一张卡片，只有一个会成功；一个 agent 在一块白板上同时只能持有一张卡片。
- **心跳与超时释放**：领取后定时心跳，界面实时显示谁在工作、做了多久；连续 50 秒没有心跳，卡片自动回到队列，并记为一次异常退出。
- **工作流按白板自定义**：状态、规则（可领取 / 锁定 / 终点）和状态之间的连线都能改。agent 只能沿连线移动卡片；人拖拽不要求有连线，但同样不能直接把卡片放进锁定状态。
- **按项目组织**：一个项目下可以有多块白板，例如 `oss/main`、`oss/release`；每块白板有独立的工作流、卡片编号和领取状态，标签在项目内共用。
- **从想法到完成**：没想清楚的事先记进「未整理」，整理成可执行卡片后再放进工作流。
- **卡片不只是一个标题**：Markdown 描述、评论、标签、附件（截图、日志等，单个 10 MB）、累计耗时、完整动态，以及人可以直接「强制释放」。
- **一句话把活派给 agent**：卡片详情里点「复制为提示词」，得到的就是一句带项目、白板和卡片编号的话；agent 通过 `board-skills` 自己读取卡片、领取、干活、交审。
- **不绑定某一家 agent**：只要它能执行命令行，就能接 Board。
- **多端可用**：网页端、macOS / Windows 桌面端，手机浏览器也能用；支持深色 / 浅色主题。
- **CLI 对机器友好**：所有命令支持 `--json`，失败时给出稳定的错误码和退出码。

## 一起工作的方式

新建白板时自带一条默认工作流：

```mermaid
flowchart LR
    todo["待办<br/>可领取"] --> doing["进行中<br/>锁定"]
    doing --> review["待审核"]
    doing --> check["需人工校验"]
    review --> done["已完成<br/>终点"]
    review --> rework["返工<br/>可领取"]
    rework --> doing
    check --> todo
```

- **待办** 是可领取的起点，人在界面上从上到下决定先做什么。
- **进行中** 是锁定状态，只有领取它的 agent 能编辑和移动卡片。
- **待审核** 是 agent 交审的地方。人看完可以移到 **已完成**，也可以退回 **返工**。
- **需人工校验** 放 agent 做不下去、需要人决定的卡片。
- **返工** 可以被重新领取，做完后再次交审。

这些状态和连线都可以按项目调整，工作流不是固定不变的。

## 接入 agent

Board 不包含 agent 本身。接入分三步：装好 `board` 命令行、装上 `board-skills`、把白板地址和令牌告诉 agent。

**1. 安装命令行**

桌面端安装包自带 `board`：Windows 安装时写进 PATH；macOS 在「设置 → Agent 接入」里执行一条链接命令。从源码安装：

```bash
cargo install --path crates/board-cli
```

**2. 安装 skill**

本仓库的 skill 在 [`.agents/skills/board-skills/`](.agents/skills/board-skills/)；支持 `.agents/skills` 的 agent 会自动识别，Claude Code 通过 `.claude/skills/board-skills` 链接识别。不在仓库里时，在「设置 → Agent 接入」里复制 SKILL.md，保存到 agent 的 skills 目录。

**3. 连接白板**

```bash
export BOARD_URL=https://board.buglan.com
export BOARD_TOKEN=bd_xxx     # 设置 → Agent 接入 → 新建令牌
export BOARD=oss/main         # 项目标识/白板标识
export BOARD_AS=alpha         # agent 的名字
```

访问令牌原文只显示一次；`BOARD_AS` 只是 agent 的名字，和令牌无关，同一个令牌可以给多个不同名字的 agent 使用。

`BOARD_URL` 和 `BOARD_TOKEN` 必须配置：没有设置 `BOARD_URL`，或设置了却没有 `BOARD_TOKEN`，命令都会被拒绝（`NOT_LOGGED_IN`），提示去「设置 → Agent 接入」新建令牌。在自己电脑上用本机模式的服务端时，`BOARD_URL` 填它的地址（例如 `http://127.0.0.1:7420`），令牌同样在它的「设置 → Agent 接入」新建。

然后给 agent 发一句话：

```text
/board-skills 开始执行 oss项目之中的main白板中的#2任务
```

卡片详情里的「复制为提示词」（快捷键 `C`）会生成这一句；`/board-skills oss/main` 则让 agent 循环领取这块白板上所有可领取的卡片。agent 会按照 skill 里的流程：领取 → 开始心跳 → 按描述和验收标准干活 → 评论进度 → 结束心跳并移到待审核；做不下去时移到需人工校验。

## CLI 速查

```bash
board boards                  # 列出所有白板
board flow                    # 查看当前白板的工作流
board ls                      # 列出卡片
board show 2                  # 查看卡片详情
board claim                   # 领取下一张；board claim 2 领取指定卡片
board comment 2 "进展……"      # 记录进度
board move 2 review           # 沿连线移动卡片
board beat 2 --final          # 结束心跳（通常由 skill 里的脚本调用）
board new "标题" --body "描述" # 新建卡片
board attach 2 shot.png --comment "截图"  # 上传附件
```

所有命令都支持 `--json`；完整的命令和参数见 `board help`，规则与错误码见 skill。

## 边界

- Board 不是 agent：不生成任务、不执行任务、不监听 issue。这些都可以在 Board 之外做，再通过 CLI 把任务写进来。
- Board 不是团队协作工具：一个实例只有一个登录账号，没有成员、角色和注册。
- Board 不替人做决定：优先级、验收和是否完成都在界面上由人决定；agent 只能沿工作流移动卡片。

---

在线实例：<https://board.buglan.com>（GitHub 登录，账号在部署时指定）· 部署自己的实例：[docs/deploy.md](docs/deploy.md)
