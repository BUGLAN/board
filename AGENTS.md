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

由人决定，agent 不打 tag。推 `main` 只跑测试；推 `vX.Y.Z` tag 才构建镜像、部署线上服务端，打 tag 前先把 `Cargo.toml` 里 `workspace.package.version` 改成 `X.Y.Z` 并提交（见 `docs/deploy.md`「自动部署」）。
