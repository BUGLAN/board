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
      "detail": "codex-mbp/codex 在线",
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

`--agent` 由 Board 按机器设置生成名字，`modeled` 是它有没有套用机器里的模型设置；`--as` 自己起名字时 `modeled` 为 `null`：

```json
{
  "beat_at": "2026-11-13T15:55:16-05:00",
  "cards": [],
  "checked_at": "2026-11-13T15:55:16-05:00",
  "modeled": false,
  "name": "codex-mbp/codex"
}
```

### `board heartbeat start`

立即返回，后台进程每 `interval_ms` 毫秒发一次心跳；`agent_pid` 是它跟随的 agent 进程，没找到时为 `null`：

```json
{
  "agent_pid": 48211,
  "interval_ms": 10000,
  "loop_pid": 48210,
  "name": "codex-mbp/codex"
}
```

### `board heartbeat stop`

停掉后台心跳并发一次 `final`，返回和 `board beat --final` 相同的 agent 状态。名字本来就不在线时是 `{"already_offline": true, "name": "codex-mbp/codex"}`：

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

每个状态带规则和能去的状态；`repo`、`branch` 是白板设置里的仓库地址和集成分支，没设时为空字符串：

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

项目里所有标签；`human_only` 是仅人可打，`priority` 是优先标签，`cards` 是被多少张卡片使用：

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
    "path": "/tmp/board-files/1-run.log",
    "size": 35,
    "url": "/api/files/1"
  }
]
```

## 机器与派发

### `board host`

`board host` 是常驻进程，没有一次性的 JSON 结果；启动后按行输出机器、环境、agent 和白板目录，之后输出每次派发和同步的进展：

```text
✓ mbp 已上线：https://board.example.com
  系统    Windows 11 Pro · board 0.1.25
  环境    cmd、Windows PowerShell、PowerShell 7、Git Bash、WSL（Ubuntu-24.04）、直接启动
  agent   dsh DeepSeek Harness  0.2.2-rc.1 · PowerShell 7（默认）
          claude Claude Code  2.1.322 (Claude Code) · PowerShell 7
          pi pi  1.5.2 · Git Bash
          codex Codex  codex-cli 0.167.1 · PowerShell 7
  工作空间C:\Users\you\repo
  白板    oss/main → C:\Users\you\repo\oss\main
          oss/release → C:\Users\you\repo\oss\release
  Ctrl+C 退出（先结束本机由它启动的 agent）
```

`--install`、`--uninstall`、`rename <新机器名>` 也输出这类进度和结果行，没有一次性的 JSON。

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
  "message": "本机有 board host 在跑：同步由它来做（库一变它马上跟上，界面「机器 → 共用配置」看结果）",
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
