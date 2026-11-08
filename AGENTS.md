# Board 仓库约定

给在这个仓库里写代码的 agent（board-skills 从这里读集成分支和推送前检查）。

## 集成分支

集成分支是 `main`。不建卡片分支，也没有 dev：在自己的 worktree 里做完，rebase 到 `origin/main`、跑完下面的检查后直接 `git push origin HEAD:main`。推送被拒（别人先推了）就重新 rebase、重新跑检查再推，不要 `--force`。

## 推送前检查

在 rebase 之后的代码上跑全部检查，不只跑改到的部分：

```sh
unset BOARD_URL BOARD_TOKEN   # 去掉指向线上白板的环境变量，免得测试写到线上
npm test                      # 即 cargo test && npm --prefix web run typecheck
```

新 worktree 里第一次跑之前先 `npm ci`（postinstall 会装 `web/` 的依赖）。

## 提交

- 第一行按 Conventional Commits，描述用中文，例如 `feat(web): …`、`fix(cli): …`、`docs: …`、`chore: …`。
- 处理 Board 卡片时，一次领取压成一个提交，最后一行是 `卡片：<项目>/<白板>#<ID>`（见 `.agents/skills/board-skills/SKILL.md`）。

## 发版

发版由人决定：人没让你发版时，不改版本号、不打 tag。版本号 `x.y.z`，tag 是 `vx.y.z`，第一个是 `v0.1.0`：

| 位 | 什么时候升 |
|---|---|
| x | 只由人决定，你不升 |
| y | 重构或大升级。要升 y 时先停下来通知人，列出是哪些提交，由人决定升 y 还是 z |
| z | 修复和比较小的新功能 |

人让你发版、没给版本号时：

1. `npm run release -- <上一版的 z 加 1> --dry-run` 看这一版有哪些提交（不改任何文件）。
2. 里面有重构或大升级（`refactor` 提交、标了「不兼容」的提交，或者你判断算大升级的）：停下来，列出这些提交，问人升 y 还是 z。没有就升 z。
3. `npm run release -- <版本号> --push`：改版本号（Cargo.toml、Cargo.lock、package.json、tauri.conf.json）、从上一个 tag 之后的每个提交生成 `CHANGELOG.md` 里这一版的一节，提交「chore: 发版 vX.Y.Z」、打 tag，再推 `main` 和 tag。

推 `main` 只跑测试；推 tag 才构建镜像、部署线上服务端，部署成功后用 `CHANGELOG.md` 里这一版的一节建 GitHub Release（见 `docs/deploy.md`「升级」「自动部署」）。tag 推出去后不改、不删、不挪，发错了就再发一个版本。
