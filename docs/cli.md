# CLI 的 `--json` 输出

所有命令都支持 `--json`。成功时退出码为 0，标准输出是一段 JSON；被拒绝时退出码为 1，输出一个带错误码的对象：

```json
{
  "error": {
    "code": "USAGE",
    "message": "项目里没有标签「CLI」，可选：bug、文档、重构、社区、发布、Windows"
  }
}
```

下面按 `board help` 的命令顺序列出输出样例，命令和参数见 [README 的「CLI 速查」](../README.md#cli-速查)。示例以 demo 白板 `oss/main` 为例；编号、时间、名字和路径会随实际环境变化。为了读起来短一些，卡片动态和列表只保留了一部分条目，字段名和结构与实际输出一致。写操作返回的通常是一整张卡片，和 `board show --json` 的形状相同。

## 登录与升级

### `board login`

校验令牌通过后写入配置文件。`env_overrides` 是当前 shell 里会覆盖配置文件的变量：

```json
{
  "config": "/home/you/.board/config.toml",
  "env_overrides": [],
  "ok": true,
  "url": "https://board.example.com"
}
```

### `board logout`

只删令牌，保留服务端地址。没有可删的令牌时 `removed` 为 `false`：

```json
{
  "config": "/home/you/.board/config.toml",
  "env_overrides": [],
  "ok": true,
  "removed": true
}
```

### `board update`

已经是最新版本时 `updated` 为 `false`；替换成功后为 `true`。桌面端自带的 `board` 会拒绝，并在错误里说明跟着桌面端升级：

```json
{
  "ok": true,
  "path": "/home/you/.local/bin/board",
  "updated": false,
  "version": "0.1.25"
}
```

### `board doctor`

`checks` 里每项都有 `status`（`ok` / `warn` / `fail`）和给人看的 `detail`、`next`。有 `fail` 时 `ok` 为 `false`、`exit_code` 为 1：

```json
{
  "checks": [
    {
      "detail": "/home/you/.local/bin/board",
      "key": "board_path",
      "next": null,
      "ok": true,
      "source": null,
      "status": "ok",
      "title": "board 自身路径"
    },
    {
      "detail": "/home/you/.local/bin/board",
      "key": "path",
      "next": null,
      "ok": true,
      "source": null,
      "status": "ok",
      "title": "PATH 里的 board"
    },
    {
      "detail": "https://board.example.com（来自 /home/you/.board/config.toml）",
      "key": "board_url",
      "next": null,
      "ok": true,
      "source": "file",
      "status": "ok",
      "title": "服务端地址"
    },
    {
      "detail": "连上了：board 0.1.25（提交 9e843fc）",
      "key": "server",
      "next": null,
      "ok": true,
      "source": null,
      "status": "ok",
      "title": "服务端版本"
    },
    {
      "detail": "令牌有效（来自 /home/you/.board/config.toml；不显示令牌原文）",
      "key": "board_token",
      "next": null,
      "ok": true,
      "source": "file",
      "status": "ok",
      "title": "访问令牌"
    },
    {
      "detail": "mbp/codex 在线",
      "key": "board_as",
      "next": null,
      "ok": true,
      "source": null,
      "status": "ok",
      "title": "名字"
    },
    {
      "detail": "oss/main",
      "key": "board",
      "next": null,
      "ok": true,
      "source": null,
      "status": "ok",
      "title": "白板"
    }
  ],
  "exit_code": 0,
  "ok": true,
  "server": {
    "commit": "9e843fc",
    "reachable": true,
    "version": "0.1.25"
  },
  "summary": {
    "failed": 0,
    "passed": 7,
    "skipped": 0,
    "total": 7
  },
  "version": "0.1.25"
}
```

## 身份与心跳

### `board check`

`--agent` 由 Board 按这台机器生成名字（`<机器名>/<模型名>-<推理强度>`），`agent` 是这个名字跑在哪个 agent 里（界面画图标用），`modeled` 是名字第二段有没有用上模型（board host 上报的、这个 agent 配置文件里现在的模型；没有时第二段是 agent 标识）；`--as` 自己起名字时 `modeled` 为 `null`：

```json
{
  "beat_at": "2026-11-13T15:55:16-05:00",
  "cards": [],
  "checked_at": "2026-11-13T15:55:16-05:00",
  "modeled": false,
  "name": "mbp/codex"
}
```

### `board heartbeat start`

立即返回，后台进程每 `interval_ms` 毫秒发一次心跳；`agent_pid` 是它跟随的 agent 进程，没找到时为 `null`：

```json
{
  "agent_pid": 48211,
  "interval_ms": 10000,
  "loop_pid": 48210,
  "name": "mbp/codex"
}
```

### `board heartbeat stop`

停掉后台心跳并发一次 `final`，返回和 `board beat --final` 相同的 agent 状态。名字本来就不在线时是 `{"already_offline": true, "name": "mbp/codex"}`：

```json
{
  "beat_at": "2026-11-13T15:51:32-05:00",
  "cards": [],
  "checked_at": "2026-11-13T15:50:29-05:00",
  "modeled": null,
  "name": "docs/writer"
}
```

### `board heartbeat final`

结束这张卡片这一次的计时，返回整张卡片（完整字段同 `board show --json`）。节选：

```json
{
  "beat_at": "2026-11-13T15:51:36-05:00",
  "board": "oss/main",
  "column": "doing",
  "holder": "docs/writer",
  "id": 13,
  "run_started_at": null,
  "work": {
    "beats": 3,
    "lost": 0,
    "runs": 1,
    "total_ms": 27858
  }
}
```

### `board beat`

不带 ID 是 agent 级心跳，`cards` 是这个名字正在做的卡片；带 ID 时返回那张卡片：

```json
{
  "beat_at": "2026-11-13T15:51:32-05:00",
  "cards": [
    {
      "beats": 2,
      "board": "oss/main",
      "column": "doing",
      "id": 13,
      "run_started_at": "2026-11-13T15:51:08-05:00",
      "title": "示例：CLI 输出（改）"
    }
  ],
  "checked_at": "2026-11-13T15:50:29-05:00",
  "modeled": null,
  "name": "docs/writer"
}
```

## 看板与卡片

### `board boards`

`counts` 按状态标识统计，包含「未整理」的 `inbox`；`flow` 是这块白板的工作流顺序：

```json
[
  {
    "board": "oss/main",
    "counts": {
      "check": 0,
      "doing": 1,
      "done": 1,
      "inbox": 4,
      "review": 2,
      "rework": 0,
      "todo": 3
    },
    "flow": [
      "todo",
      "doing",
      "review",
      "done",
      "rework",
      "check"
    ],
    "name": "维护主线",
    "project": "Board 开源仓库"
  }
]
```

### `board flow`

每个状态带规则和能去的状态；`repo`、`branch` 是白板所在项目的仓库地址和集成分支（一个项目一个仓库），没设时为空字符串：

```json
{
  "branch": "",
  "repo": "",
  "statuses": [
    {
      "cards": 5,
      "claim": true,
      "error": false,
      "final": false,
      "key": "todo",
      "lock": false,
      "name": "待办",
      "review": false,
      "to": ["doing"]
    },
    {
      "cards": 0,
      "claim": false,
      "error": false,
      "final": false,
      "key": "doing",
      "lock": true,
      "name": "进行中",
      "review": false,
      "to": ["review", "check"]
    },
    {
      "cards": 2,
      "claim": false,
      "error": false,
      "final": false,
      "key": "review",
      "lock": false,
      "name": "待审核",
      "review": true,
      "to": ["done", "rework"]
    },
    {
      "cards": 1,
      "claim": false,
      "error": false,
      "final": true,
      "key": "done",
      "lock": false,
      "name": "已完成",
      "review": false,
      "to": []
    },
    {
      "cards": 0,
      "claim": true,
      "error": false,
      "final": false,
      "key": "rework",
      "lock": false,
      "name": "返工",
      "review": false,
      "to": ["doing"]
    },
    {
      "cards": 0,
      "claim": false,
      "error": false,
      "final": false,
      "key": "check",
      "lock": false,
      "name": "需人工校验",
      "review": false,
      "to": ["todo"]
    }
  ]
}
```

### `board ls`

按「未整理」、工作流顺序、状态内顺序排列。`holder` 是当前持有者，`labels` 是标签名，`priority_labels` 是其中的优先标签：

```json
[
  {
    "column": "inbox",
    "holder": null,
    "id": 9,
    "labels": [],
    "priority_labels": [],
    "title": "调研：一个仓库是否需要多块白板"
  },
  {
    "column": "todo",
    "holder": null,
    "id": 2,
    "labels": ["bug", "Windows"],
    "priority_labels": [],
    "title": "Windows 下路径分隔符导致快照测试失败"
  },
  {
    "column": "doing",
    "holder": "beta",
    "id": 3,
    "labels": ["bug"],
    "priority_labels": [],
    "title": "无网络时 `sync` 命令直接 panic"
  }
]
```

### `board search`

跨白板搜索，不用 `--board`。`matched` 说明命中的位置（`id`、`label`、`title`、`body`、`comment`），`status` 是状态名称：

```json
[
  {
    "board": "oss/main",
    "column": "todo",
    "id": 2,
    "labels": ["bug", "Windows"],
    "matched": "title",
    "status": "待办",
    "title": "Windows 下路径分隔符导致快照测试失败",
    "updated_at": "2026-11-13T15:50:28-05:00"
  },
  {
    "board": "oss/release",
    "column": "todo",
    "id": 2,
    "labels": ["发布", "Windows"],
    "matched": "title",
    "status": "待办",
    "title": "各平台安装包签名（macOS / Windows）",
    "updated_at": "2026-11-13T15:50:28-05:00"
  }
]
```

### `board show`

卡片的全部字段：描述、标签、附件、动态、累计耗时和网址。`split_from` / `split_into` 是拆解关系：

```json
{
  "activity": [
    {
      "action": "create",
      "actor": "issue-watcher",
      "actor_kind": "agent",
      "at": "2026-11-13T09:54:38-05:00",
      "text": "新建了卡片",
      "type": "system"
    },
    {
      "action": "claim",
      "actor": "beta",
      "actor_kind": "agent",
      "at": "2026-11-13T15:23:38-05:00",
      "text": "领取了卡片，从「待办」进入「进行中」",
      "type": "system"
    },
    {
      "action": "comment",
      "actor": "beta",
      "actor_kind": "agent",
      "at": "2026-11-13T15:24:08-05:00",
      "text": "已定位到 `snapshot::normalize`，准备统一替换分隔符。",
      "type": "comment"
    }
  ],
  "attachments": [],
  "beat_at": "2026-11-13T15:23:40-05:00",
  "board": "oss/main",
  "body": "gh#140 · 来自 issue 监听\n\n在 Windows runner 上 `tests/snapshot.rs` 有 3 个用例失败，路径里出现了 `\\\\`。\n\n**验收标准**\n- [ ] 快照里统一使用 `/`\n- [ ] CI 增加 windows-latest",
  "claimed_at": "2026-11-13T15:23:38-05:00",
  "column": "doing",
  "created_at": "2026-11-13T09:54:38-05:00",
  "holder": "beta",
  "id": 2,
  "labels": ["bug", "Windows"],
  "priority_labels": [],
  "run_started_at": "2026-11-13T15:23:40-05:00",
  "split_from": null,
  "split_into": [],
  "title": "Windows 下路径分隔符导致快照测试失败",
  "updated_at": "2026-11-13T15:24:08-05:00",
  "url": "https://board.example.com/oss/main/2",
  "work": {
    "beats": 156,
    "lost": 0,
    "runs": 1,
    "total_ms": 1558000
  }
}
```

### `board new`

建好后返回整张卡片，`url` 是新卡片的网址。`--inbox` 建的卡片 `column` 是 `inbox`；`--from` 会写进动态和 `split_from`；`--image`、`--file` 只是把 Markdown 引用接在描述后面，附件会出现在 `attachments` 里：

```json
{
  "activity": [
    {
      "action": "create",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:50:57-05:00",
      "text": "新建了卡片",
      "type": "system"
    }
  ],
  "attachments": [],
  "beat_at": null,
  "board": "oss/main",
  "body": "给 docs/cli.md 用的示例卡片。",
  "claimed_at": null,
  "column": "todo",
  "created_at": "2026-11-13T15:50:57-05:00",
  "holder": null,
  "id": 13,
  "labels": ["文档"],
  "priority_labels": [],
  "run_started_at": null,
  "split_from": null,
  "split_into": [],
  "title": "示例：CLI 输出",
  "updated_at": "2026-11-13T15:50:57-05:00",
  "url": "https://board.example.com/oss/main/13",
  "work": {
    "beats": 0,
    "lost": 0,
    "runs": 0,
    "total_ms": 0
  }
}
```

### `board edit`

返回修改后的整张卡片，动态里会多出 `edit` 条目：

```json
{
  "activity": [
    {
      "action": "create",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:50:57-05:00",
      "text": "新建了卡片",
      "type": "system"
    },
    {
      "action": "edit",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:51:00-05:00",
      "text": "把标题从「示例：CLI 输出」改为「示例：CLI 输出（改）」",
      "type": "system"
    },
    {
      "action": "edit",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:51:00-05:00",
      "text": "修改了描述",
      "type": "system"
    }
  ],
  "attachments": [],
  "beat_at": null,
  "board": "oss/main",
  "body": "更新后的描述。",
  "claimed_at": null,
  "column": "todo",
  "created_at": "2026-11-13T15:50:57-05:00",
  "holder": null,
  "id": 13,
  "labels": ["文档"],
  "priority_labels": [],
  "run_started_at": null,
  "split_from": null,
  "split_into": [],
  "title": "示例：CLI 输出（改）",
  "updated_at": "2026-11-13T15:51:00-05:00",
  "url": "https://board.example.com/oss/main/13",
  "work": {
    "beats": 0,
    "lost": 0,
    "runs": 0,
    "total_ms": 0
  }
}
```

### `board claim`

领取成功后卡片进入锁定状态，`holder` 是你，`claimed_at` 有值；返回整张卡片。不带 ID 时领取工作流里最上面一张可领取的卡片：

```json
{
  "activity": [
    {
      "action": "create",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:50:57-05:00",
      "text": "新建了卡片",
      "type": "system"
    },
    {
      "action": "claim",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:51:04-05:00",
      "text": "领取了卡片，从「待办」进入「进行中」",
      "type": "system"
    }
  ],
  "attachments": [],
  "beat_at": null,
  "board": "oss/main",
  "body": "更新后的描述。",
  "claimed_at": "2026-11-13T15:51:04-05:00",
  "column": "doing",
  "created_at": "2026-11-13T15:50:57-05:00",
  "holder": "docs/writer",
  "id": 13,
  "labels": ["文档"],
  "priority_labels": [],
  "run_started_at": null,
  "split_from": null,
  "split_into": [],
  "title": "示例：CLI 输出（改）",
  "updated_at": "2026-11-13T15:51:04-05:00",
  "url": "https://board.example.com/oss/main/13",
  "work": {
    "beats": 0,
    "lost": 0,
    "runs": 0,
    "total_ms": 0
  }
}
```

### `board start`

开始做一张卡，把会话开始和领取合成一条命令：这个会话（调用它的 agent 进程）还没有在线的名字时先占用名字并启动后台心跳，已经有了就接着用；然后领取、下载附件，返回整张卡片。

```sh
board start 13 --agent claude --json     # 会话里第一次：--agent 是你实际运行的 agent，名字由 Board 生成
board start --json                       # 之后：接着用名字和心跳；不带 ID 时领最上面一张可领取的
```

返回的字段和 `board claim --json` 一样，另外多这几项：

```json
{
  "session": { "name": "claude-mbp/opus-5.5", "started": true, "resumed": false, "follows": true },
  "review": { "key": "review", "name": "待审核" },
  "giveup": { "key": "check", "name": "需人工校验" },
  "repo": "https://github.com/o/r.git",
  "branch": "main",
  "files": [
    {
      "created_at": "2026-11-13T15:51:24-05:00",
      "created_by": "docs/writer",
      "id": 1,
      "image": true,
      "mime": "image/png",
      "name": "shot.png",
      "path": "/tmp/board-files-x1Yz/1-shot.png",
      "size": 20480,
      "url": "/api/files/1"
    }
  ]
}
```

- `session.started`：这一次是不是新占用的名字；`session.resumed`：卡片本来就由你持有（上次没做完的），没有重新领取；`session.follows`：后台心跳跟着调用它的 agent 进程（进程不在了心跳就停）。认不出 agent 进程时（祖先进程全是 shell、容器里 agent 是 1 号进程、`BOARD_HEARTBEAT_PID` 指向的进程不在）为 `false`，文字输出多一行提醒：心跳会一直跑到 `board heartbeat stop`，这台机器上同时只能开一个这样的会话；这时同一个会话里再 `board start` 不用重带 `--agent`（本机只有一条这样的记录时接着用它）。
- `review`、`giveup`：`board submit`、`board giveup` 会把卡片移到哪个状态；白板上没有对应的状态时为 `null`。
- `files`：附件已经下载到 `path`；`--out <目录>` 指定放哪，不写时是只有自己能读的临时目录。
- 会话里第一次用、又没带 `--agent`（或 `--as`）时是 `USAGE`；没有可领的卡片时 `EMPTY_TODO`。
- 先占名字、启动心跳，再领取：领取失败（`EMPTY_TODO`、`NOT_FOUND`、`NOT_IN_TODO`、`ALREADY_HOLDING`）时名字已经在线、后台心跳已经在跑，不再领卡时 `board heartbeat stop`。报错里写着用的名字：`--json` 是 `error.detail.session`，文字输出在这一次新占用名字时多一行「名字 <名字> 已占用，后台心跳在跑；不再领卡时 board heartbeat stop」：

  ```json
  {
    "error": {
      "code": "EMPTY_TODO",
      "message": "可领取的状态里没有能领取的卡片",
      "detail": { "session": { "name": "mbp/opus-5.5-high", "started": true } }
    }
  }
  ```
- `board host` 启动的会话里（环境里有 `BOARD_HOST`）：不占名字、不启动心跳、不领取，只读卡片和附件；要带 ID，卡片不归 `BOARD_AS` 持有时 `NOT_HOLDER`。

### `board submit`

交审：上传附件、发交审评论、结束这张卡片这一次的计时、把卡片移到交审状态，一条命令做完（服务端在一次写入里完成，不会出现结束计时之后没移走、领取被释放）。

```sh
board submit 13 --comment-file review.md shot.png check.log
```

```
✓ #13 已交审 → 待审核（review）  带 2 个附件
```

`--json` 返回移动后的整张卡片，另有 `moved`（是否移走了）和 `uploaded`（这次上传的附件）。

- 只有持有者能交审（`NOT_HOLDER`）；评论不能为空（`USAGE`）；白板没有交审状态时 `FORBIDDEN_COLUMN`，改用 `board giveup`。
- 项目设了代码仓库时，评论里要有以 `提交：`、`集成分支：`、`检查：`、`碰到的别的卡：` 开头的四行；缺了被拒绝（`USAGE`），原因里写明缺哪几行，附件不上传、卡片不动。这张卡没改代码（调研、整理）时加 `--no-code`。
- 连旧服务端（不认这条命令）时，命令行自己按评论、final、移动三步做，结果一样。

### `board giveup`

做不下去、交给人：上传附件、评论原因、结束这一次的计时、把卡片移到错误节点（没有错误节点时移到需要人处理的状态），一条命令做完。

```sh
board giveup 13 --comment "检查不过：cargo test 里 hosts_sync 超时，本机复现不了" run.log
```

```
✓ #13 已交给人处理 → 需人工校验（check）  带 1 个附件
```

白板上两种状态都没有时只评论、结束计时，卡片留在原处，输出里写明：50 秒后领取释放，卡片回到领取前的状态。

### `board next`

只在 `board host` 启动的会话里可用。领到下一张时输出和 `board claim --json` 一样；不再接着做时返回错误 `SESSION_DONE`，退出码为 1：

```json
{
  "error": {
    "code": "SESSION_DONE",
    "message": "这个会话排着的卡片都开始过或取消了：结束这个会话"
  }
}
```

### `board hold`

推集成分支前确认领取仍有效。成功时 `ok` 为 `true`：

```json
{
  "board": "oss/main",
  "column": "doing",
  "holder": "docs/writer",
  "id": 13,
  "ok": true
}
```

### `board base`

开工成功时 `step` 是下一步做什么：`code` 直接改代码、`wip` 从 wip 分支接着做（`from_wip`，已经 rebase 到最新集成分支）、`submit` 已经推过、动态里没有交审、`rework` 交审过又被打回；`next` 是同样意思的一句话。`landed` 是这张卡已经在集成分支上的提交，`reported` 是卡片动态里有没有写过它的「提交：」行：

```json
{
  "base": "31864eb",
  "branch": "main",
  "created": true,
  "from_wip": null,
  "landed": [],
  "next": "直接改代码：cd /home/me/repo/board.wt/docs-writer-1a2b3c4d",
  "step": "code",
  "worktree": "/home/me/repo/board.wt/docs-writer-1a2b3c4d"
}
```

worktree 里有没推的改动时退出码 1，没有切换，`step` 是 `save`，`dirty` 是 `git status --porcelain` 的行，`unpushed` 是没推到远端的提交；从 wip 分支接着做、rebase 冲突时退出码 1，`step` 是 `resolve`，`conflicts` 是冲突的文件。两种情况 `--json` 照样输出，不是 `error`。卡片不由你持有时是 `NOT_HOLDER`，什么都不动。

### `board land`

推送成功时 `pushed` 为 `true`，`review_lines` 是交审评论最后的四行（照抄），`wip_deleted` 是推上去之后删掉的、接着做的 wip 分支（没有时为空）：

```json
{
  "branch": "main",
  "checked": "npm test 通过",
  "commit": "31a6ba4",
  "pushed": true,
  "review_lines": "提交：31a6ba4\n集成分支：main\n检查：npm test 通过\n碰到的别的卡：无",
  "touched": "碰到的别的卡：无",
  "wip_deleted": "wip/main/13",
  "wip_kept": null
}
```

碰到别的卡、推送被拒时退出码 1（`--json` 照样输出，不是 `error`），`pushed` 为 `false`，`reason` 是 `touched` 或 `rejected`，`hint` 写下一步怎么做；碰到别的卡时 `report` 是 `board touched --json` 的完整输出。本地压好的提交（`commit`）留着：

```json
{
  "branch": "main",
  "commit": "7c1e2a9",
  "detail": "[rejected] (fetch first)",
  "hint": "推送被拒（[rejected] (fetch first)）：多半是别人先推了。本地压好的提交 7c1e2a9 还在：git fetch origin、git rebase origin/main、在 rebase 之后的代码上重新跑检查，再 board land 13。不要 --force",
  "pushed": false,
  "reason": "rejected"
}
```

工作区有没提交的文件、HEAD 不在最新的集成分支之上、相对集成分支没有改动时是 `USAGE`，什么都没改；领取已经失效、连不上服务端时和 `board hold` 一样是 `NOT_HOLDER`、`UNREACHABLE`，没有推。

`--wip`：

```json
{
  "branch": "wip/main/13",
  "commit": "6dced04",
  "line": "未完成的改动：wip/main/13"
}
```

### `board move`

返回移动后的整张卡片。移到非锁定状态后 `holder`、`claimed_at` 清空；只能沿 `board flow` 里的连线移动：

```json
{
  "activity": [
    {
      "action": "claim",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:51:04-05:00",
      "text": "领取了卡片，从「待办」进入「进行中」",
      "type": "system"
    },
    {
      "action": "move",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:51:40-05:00",
      "text": "从「进行中」移到「待审核」，领取已释放",
      "type": "system"
    }
  ],
  "attachments": [],
  "beat_at": null,
  "board": "oss/main",
  "body": "更新后的描述。",
  "claimed_at": null,
  "column": "review",
  "created_at": "2026-11-13T15:50:57-05:00",
  "holder": null,
  "id": 13,
  "labels": ["文档", "社区"],
  "priority_labels": [],
  "run_started_at": null,
  "split_from": null,
  "split_into": [],
  "title": "示例：CLI 输出（改）",
  "updated_at": "2026-11-13T15:51:40-05:00",
  "url": "https://board.example.com/oss/main/13",
  "work": {
    "beats": 3,
    "lost": 0,
    "runs": 1,
    "total_ms": 27858
  }
}
```

### `board comment`

返回整张卡片，动态末尾多一条 `comment`：

```json
{
  "activity": [
    {
      "action": "comment",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:51:08-05:00",
      "text": "补完 examples/ 里的示例，等 review。",
      "type": "comment"
    }
  ],
  "attachments": [],
  "beat_at": "2026-11-13T15:51:08-05:00",
  "board": "oss/main",
  "body": "更新后的描述。",
  "claimed_at": "2026-11-13T15:51:04-05:00",
  "column": "doing",
  "created_at": "2026-11-13T15:50:57-05:00",
  "holder": "docs/writer",
  "id": 13,
  "labels": ["文档"],
  "priority_labels": [],
  "run_started_at": "2026-11-13T15:51:08-05:00",
  "split_from": null,
  "split_into": [],
  "title": "示例：CLI 输出（改）",
  "updated_at": "2026-11-13T15:51:08-05:00",
  "url": "https://board.example.com/oss/main/13",
  "work": {
    "beats": 1,
    "lost": 0,
    "runs": 1,
    "total_ms": 45
  }
}
```

### `board label`

返回整张卡片，`labels` 是加、减标签之后的结果：

```json
{
  "activity": [
    {
      "action": "label",
      "actor": "docs/writer",
      "actor_kind": "agent",
      "at": "2026-11-13T15:51:15-05:00",
      "text": "添加了标签 「社区」",
      "type": "system"
    }
  ],
  "attachments": [],
  "beat_at": "2026-11-13T15:51:08-05:00",
  "board": "oss/main",
  "body": "更新后的描述。",
  "claimed_at": "2026-11-13T15:51:04-05:00",
  "column": "doing",
  "created_at": "2026-11-13T15:50:57-05:00",
  "holder": "docs/writer",
  "id": 13,
  "labels": ["文档", "社区"],
  "priority_labels": [],
  "run_started_at": "2026-11-13T15:51:08-05:00",
  "split_from": null,
  "split_into": [],
  "title": "示例：CLI 输出（改）",
  "updated_at": "2026-11-13T15:51:15-05:00",
  "url": "https://board.example.com/oss/main/13",
  "work": {
    "beats": 1,
    "lost": 0,
    "runs": 1,
    "total_ms": 7470
  }
}
```

### `board labels`

项目里所有标签；`human_only` 是仅人可打（agent 不能给已有卡片加、去，建卡时可以用 `--label` 带上），`priority` 是优先标签，`cards` 是被多少张卡片使用：

```json
[
  {
    "cards": 3,
    "color": "#F43F5E",
    "human_only": false,
    "name": "bug",
    "priority": false
  },
  {
    "cards": 4,
    "color": "#3B82F6",
    "human_only": false,
    "name": "文档",
    "priority": false
  },
  {
    "cards": 2,
    "color": "#8B5CF6",
    "human_only": false,
    "name": "重构",
    "priority": false
  },
  {
    "cards": 3,
    "color": "#22C55E",
    "human_only": false,
    "name": "社区",
    "priority": false
  },
  {
    "cards": 4,
    "color": "#EAB308",
    "human_only": false,
    "name": "发布",
    "priority": false
  },
  {
    "cards": 2,
    "color": "#14B8A6",
    "human_only": false,
    "name": "Windows",
    "priority": false
  }
]
```

### `board attach`

`attachments` 是这一次上传的文件，`card` 是上传后的整张卡片：

```json
{
  "attachments": [
    {
      "created_at": "2026-11-13T15:51:24-05:00",
      "created_by": "docs/writer",
      "id": 1,
      "image": false,
      "mime": "text/plain; charset=utf-8",
      "name": "run.log",
      "size": 35,
      "url": "/api/files/1"
    }
  ],
  "card": {
    "activity": [
      {
        "action": "comment",
        "actor": "docs/writer",
        "actor_kind": "agent",
        "at": "2026-11-13T15:51:24-05:00",
        "text": "测试日志\n\n[run.log](/api/files/1)",
        "type": "comment"
      }
    ],
    "attachments": [
      {
        "created_at": "2026-11-13T15:51:24-05:00",
        "created_by": "docs/writer",
        "id": 1,
        "image": false,
        "mime": "text/plain; charset=utf-8",
        "name": "run.log",
        "size": 35,
        "url": "/api/files/1"
      }
    ],
    "beat_at": "2026-11-13T15:51:08-05:00",
    "board": "oss/main",
    "body": "更新后的描述。",
    "claimed_at": "2026-11-13T15:51:04-05:00",
    "column": "doing",
    "created_at": "2026-11-13T15:50:57-05:00",
    "holder": "docs/writer",
    "id": 13,
    "labels": ["文档", "社区"],
    "priority_labels": [],
    "run_started_at": "2026-11-13T15:51:08-05:00",
    "split_from": null,
    "split_into": [],
    "title": "示例：CLI 输出（改）",
    "updated_at": "2026-11-13T15:51:24-05:00",
    "url": "https://board.example.com/oss/main/13",
    "work": {
      "beats": 1,
      "lost": 0,
      "runs": 1,
      "total_ms": 16138
    }
  }
}
```

### `board files`

下载附件到本地，`path` 是保存的位置：

```json
[
  {
    "created_at": "2026-11-13T15:51:24-05:00",
    "created_by": "docs/writer",
    "id": 1,
    "image": false,
    "mime": "text/plain; charset=utf-8",
    "name": "run.log",
    "path": "/tmp/board-files-7Hk2mQ/1-run.log",
    "size": 35,
    "url": "/api/files/1"
  }
]
```

默认下载到系统临时目录下随机命名的私有目录（Unix 上目录 0700、文件 0600），不会沿用可预测的固定路径；`--out` 仍保存到指定的目录。

## 机器与派发

### `board host`

`board host` 是常驻进程，没有一次性的 JSON 结果；启动后按行输出机器、环境、agent 和白板目录，之后输出每次派发和同步配置的进展（启动时写一次，服务器上一改收到同步请求再写一次；只写供应商和模型名称，不打印 API key）：

```text
✓ mbp 已上线：https://board.example.com
  系统    Windows 11 Pro · board 0.1.25
  环境    cmd、Windows PowerShell、PowerShell 7、Git Bash、WSL（Ubuntu-24.04）、直接启动
  agent   dsh DeepSeek Harness  0.2.2-rc.1 · PowerShell 7（默认）
          claude Claude Code  2.1.322 (Claude Code) · PowerShell 7
          codex Codex  codex-cli 0.167.1 · PowerShell 7
  工作空间C:\Users\you\repo
  白板    oss/main → C:\Users\you\repo\oss
          oss/release → C:\Users\you\repo\oss
  Ctrl+C 退出（先结束本机由它启动的 agent）
21:05:12 ✓ 同步配置：skills：装上 ~/.claude/skills/board-skills → 3c66ab9e505f
21:07:40 ✓ 同步配置：把供应商 newapi 写进 claude（模型 claude-sonnet-5-5，强度 high）
21:07:40 ! 同步配置有没写进去的：codex：Codex：config.toml 解析不了：expected `.`, `=` at line 12 column 1
```

`--install`、`--uninstall`、`rename <新机器名>` 也输出这类进度和结果行，没有一次性的 JSON。

### `board acp`

`board host` 启动 agent 用的那一层，不在 `board help` 里：`board acp --env <启动环境> [--env-prog <程序>] --cwd <目录> --prompt-file <文件> [--mode <模式>] [--set <选项>=<值>]… -- <ACP 入口命令> [参数…]`。它在启动环境里执行 ACP 入口，走完 `initialize`、`session/new`、切模式和配置、`session/prompt`，标准输出是一行一帧的 ACP 消息（派发日志），退出码见 `docs/agents.md`「board host 怎么启动 agent」。接入新 agent 时可以直接用它在本机试（同上「本机验证」）。

### `board dispatch`

派发成功时返回这次派发的记录；`state` 是 `queued`、`running` 或 `done`，`result` 是结束原因（`review`、`stuck`、`crashed`、`stopped`、`canceled` 等）：

```json
{
  "agent": "dsh",
  "agentLabel": "DeepSeek Harness",
  "beatAt": null,
  "board": "oss/main",
  "by": {
    "kind": "agent",
    "name": "docs/writer"
  },
  "card": 6,
  "createdAt": 1794603289690,
  "effort": null,
  "endedAt": null,
  "env": null,
  "envName": null,
  "exitCode": null,
  "host": "mbp",
  "hostId": 1,
  "id": 1,
  "kind": "",
  "model": null,
  "name": null,
  "result": null,
  "seq": 1,
  "session": 1,
  "source": "manual",
  "startedAt": null,
  "state": "queued",
  "stopBy": null,
  "title": "论坛反馈：`claim` 超时时间希望可配置",
  "total": 1
}
```

`--cancel` 取消排队或停止正在跑的派发，返回同一条记录；开始时 `startedAt`、`env`、`name`、`path` 才有值。

`--any` 不指定机器，由服务端在能接的机器里挑空位最多的一台：返回的记录带 `"any": true`，挑到机器之前 `host` 是空字符串、`hostId` 是 0，没带 `--agent` 时 `agent` 也是空的（挑到后换成那台的默认 agent）。`--move --host <机器名>`（或 `--move --any`）把还在排队的派发改派，返回改过的记录。

## 共用配置与版本

### `board skills ls`

`skills` 是库里的 skill，`local` 是这台机器上各 agent 目录里的状态（`synced` / `behind` / `local` / `missing`）：

```json
{
  "skills": [
    {
      "local": [
        {
          "agents": ["codex"],
          "dir": "$CODEX_HOME/skills",
          "state": "synced",
          "version": "22dc5299ad51"
        }
      ],
      "skill": {
        "builtin": true,
        "by": "服务端",
        "files": 9,
        "name": "board-skills",
        "size": 53377,
        "source": "随服务端 0.1.25",
        "updatedAt": 0,
        "version": "22dc5299ad51"
      }
    }
  ]
}
```

### `board skills push`

返回上传后的库列表。同名 skill 换成新版本，`version` 是内容哈希：

```json
{
  "ok": true,
  "skills": [
    {
      "builtin": false,
      "by": "docs",
      "files": 1,
      "name": "demo-skill",
      "size": 60,
      "source": "desktop-agiqahi",
      "updatedAt": 1794603156439,
      "version": "ccbc2c38bd60"
    }
  ]
}
```

### `board skills sync`

本机有 `board host` 在跑时由它同步，命令只返回 `by_host: true`；没有时返回每个目录的状态和这次做的改动：

```json
{
  "by_host": true,
  "message": "本机有 board host 在跑：同步由它来做（这台机器的 skills 选了「用服务器的」时库一变它就跟上，结果在界面「机器」里这台机器的设置页）",
  "ok": true
}
```

### `board skills rm`

```json
{
  "ok": true,
  "removed": "demo-skill"
}
```

### `board touched`

有碰到时退出码 1（`--json` 照样输出，不是 `error`）。`base_from` 写基点是怎么认出来的（`--base …`、`开工时切到的 <提交>`、`建好 worktree 时的 <提交>`，或「没找到开工时的基点，往前查 20 个提交」）；`from` 是这次改动从哪个提交算起，`checked` 是看了几个最近的提交；`removed` / `readded` 是这次删掉了它加的、加回了它删的行数，例子最多两行：

```json
{
  "base": "579226297a4323e0a9432b7d1647dc028a949937",
  "base_from": "--base e4a0934^",
  "checked": 1,
  "commit": "d1e88c4b99b463732a5d6dcb5cf86562d955d359",
  "from": "7f93732bfea7ed699a35021ae4c7f4cb33c85c46",
  "mine": ["default/main#328"],
  "summary": "碰到的别的卡：default/main#320（Lightbox.test.tsx、Lightbox.tsx、app.css）",
  "touched": [
    {
      "card": "default/main#320",
      "commits": [{ "sha": "e4a093467b6be1a1db5286a72bf6826343f9d0c3", "subject": "fix(web): 图片预览完整显示高图、去掉下载按钮，未整理展开行不再伸出右边" }],
      "files": [
        {
          "path": "web/src/components/overlays/Lightbox.tsx",
          "readded": 2,
          "readded_samples": ["{/* 底部条：文件名、下载，多张时右边是切换；关闭靠 Esc 或点遮罩 */}", "<a className=\"btn sm\" href={stage.src} download={stage.alt || true}>"],
          "removed": 1,
          "removed_samples": ["{/* 底部条：文件名，多张时右边是切换；关闭靠 Esc 或点遮罩。不放下载按钮（default/main#320）：要存右键图片另存为 */}"]
        }
      ]
    }
  ]
}
```

没有卡片行的提交 `card` 是 `null`。

### `board versions sync`

`--dry-run` 时 `added`、`already`、`created`、`missing` 为 `null`；真正写入后是数量或名称，`created` 表示这次有没有新建版本标签：

```json
{
  "dryRun": false,
  "project": "oss",
  "versions": [
    {
      "added": [],
      "already": 0,
      "cards": 0,
      "commits": 10,
      "created": true,
      "loose": 10,
      "missing": [],
      "name": "v0.1.25",
      "releasedAt": 1794584418000
    }
  ]
}
```

## 帮助

### `board help`

帮助本身是给人看的文本，没有 `--json`；输出所有命令、参数和一行说明，最后是状态、身份、白板、连接和输出格式的提示：

```text
board — 看板式白板的命令行。人在界面上操作，agent 通过它读写同一块白板。

  board login    [--url <地址>] [--token <令牌>]  ...
  board check    [--agent <标识>] [--as <名字>]  ...
  board boards                                    ...
  board flow                                      ...
  ...

  输出：所有命令都支持 --json；被拒绝时退出码为 1，并给出错误码
```
