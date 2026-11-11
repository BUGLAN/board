# Board 仓库约定

给在这个仓库里写代码的 agent（board-skills 从这里读集成分支和推送前检查）。

## 集成分支

集成分支是 `main`。不建卡片分支，也没有 dev：在自己的 worktree 里做完，rebase 到 `origin/main`、跑完下面的检查后直接 `git push origin HEAD:main`。推送被拒（别人先推了）就重新 rebase、重新跑检查再推，不要 `--force`。

## 推送前检查

在 rebase 之后的代码上跑全部检查，不只跑改到的部分：

```sh
unset BOARD_URL BOARD_TOKEN   # 去掉指向线上白板的环境变量，免得测试写到线上
npm test                      # 即 cargo test && npm --prefix web run typecheck && npm --prefix web test（界面单元测试，vitest）
```

新 worktree 里第一次跑之前先 `npm ci`（postinstall 会装 `web/` 的依赖）。

## 本地开发

不部署、只在本机跑时（命令以 bash 为例；PowerShell 里用 `$env:BOARD_URL=$null`、`$env:BOARD_TOKEN=$null` 清空环境变量）：

- 服务端：`unset BOARD_URL BOARD_TOKEN && cargo run -p board-server -- --local --demo --db .dev/dev.db`。本机模式只监听 `127.0.0.1:7420`；`--demo` 在空库时写入示例项目；数据库放在不进 git 的 `.dev/`，不要用 `~/.board/board.db`。
- 界面：另开一个终端 `npm run dev:web`，开发服务器在 `127.0.0.1:1420`，`/api`、`/auth` 转发到 `BOARD_API`（默认 `http://127.0.0.1:7420`）。
- CLI 连本地服务端：先在「设置 → Agent 接入」新建令牌，然后在同一条命令里覆盖可能指向线上的环境变量：

  ```sh
  BOARD_URL=http://127.0.0.1:7420 BOARD_TOKEN=<令牌> board ls --as <名字> --board default/main
  ```

- 桌面端：`npm run desktop:dev`（会先编译 sidecar 和界面；Windows 上会先结束上次开发遗留的 `board-desktop.exe` 和 `board host`，它们会锁住 `target/debug/board.exe`）。
- 测 GitHub 登录：先 `python scripts/github-mock.py 8000` 启动模拟 GitHub，再用不带 `--local` 的服务端指向它：

  ```sh
  BOARD_PUBLIC_URL=http://127.0.0.1:7420 BOARD_GITHUB_CLIENT_ID=test BOARD_GITHUB_CLIENT_SECRET=test \
  BOARD_GITHUB_USER=buglan BOARD_GITHUB_OAUTH_URL=http://127.0.0.1:8000 BOARD_GITHUB_API_URL=http://127.0.0.1:8000 \
  cargo run -p board-server -- --db .dev/dev.db
  ```

  打开 `http://127.0.0.1:7420`，授权页选 `buglan` 能登录，选 `someone-else` 会被拒绝。

## 界面写操作：乐观更新 + loading

`web/` 里会改数据的操作都走 `web/src/store/data.ts` 的乐观更新，不要 `await api.xxx()`、`refresh()` 之后才更新界面：

- 卡片命令用 `exec(cmd, patch)`，其他接口用 `optimistic(patch, () => api.xxx(…))`。补丁当帧生效；请求按发起顺序排队；成功后刷新再撤补丁，失败只撤自己这一层（不覆盖之后的操作）并抛出错误。现成的补丁：`editCard`、`movePatch`、`removeCard`、`draftCard`（新建）、`patchProject`、`editLabelPatch`、`deleteLabelPatch`。
- 补丁是纯函数，每次刷新都会重新叠到新数据上：要能重复执行，目标不在了就原样返回。
- 失败由调用方 `toastError(e)` 提示，人输入的内容不能丢（存回草稿，或带着草稿重新打开编辑框）。
- 要等服务端返回才画得出来的操作（新建项目、白板、标签、状态，派发）用 `optimistic(null, …)`，按钮加 `is-busy` 并禁用，直到返回。
- 还没确认的卡片、评论在 store 的 `saving` 里，已经画成「保存中」；新界面沿用它，不另做一套 loading。
- 保存完之前同一个对象不能再改：补丁的 `cards` / `keys`（`hostKey`、`flowKey`、`labelKey`、`projectKey`）记进 `saving`，界面用 `useCardBusy` / `useBusy` 禁用对它的操作，漏掉的由 store 拒绝（`BUSY`，不发请求）。新对象的补丁要带 `keys`。
- 不走写队列的读（轮询、单独取详情）发出后又有写操作，就丢掉结果（store 里的 `writes` 计数），免得旧数据冲掉刚保存的修改。
- 这些约定有单元测试守着：补丁在 `web/src/store/patches.test.ts`，写队列在 `web/src/store/queue.test.ts`。新加补丁时照着补用例（重复执行结果相同、目标不在了原样返回）。

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

发版前先在最新的 `origin/main` 上（工作区干净、`HEAD` 就是 `origin/main`，`scripts/release.mjs` 第一步也会核对）：

```sh
git switch main && git pull --ff-only   # 本地和远端有分叉时改用：git fetch origin && git rebase origin/main
```

确认无误后再执行 `npm run release`。

人让你发版、没给版本号时：

1. `npm run release -- <上一版的 z 加 1> --dry-run` 看这一版有哪些提交（不改任何文件）。
2. 里面有重构或大升级（`refactor` 提交、标了「不兼容」的提交，或者你判断算大升级的）：停下来，列出这些提交，问人升 y 还是 z。没有就升 z。
3. `npm run release -- <版本号> --push`：改版本号（Cargo.toml、Cargo.lock、package.json、package-lock.json、web/package.json、web/package-lock.json、tauri.conf.json）、从上一个 tag 之后的每个提交生成 `CHANGELOG.md` 里这一版的一节，提交「chore: 发版 vX.Y.Z」、打 tag，再推 `main` 和 tag。

推 `main` 只跑测试；推 tag 才构建镜像、部署线上服务端，部署成功后用 `CHANGELOG.md` 里这一版的一节建 GitHub Release（见 `docs/deploy.md`「升级」「自动部署」）。tag 推出去后不改、不删、不挪，发错了就再发一个版本。
