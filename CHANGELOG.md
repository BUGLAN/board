# 更新日志

版本号规则见 [AGENTS.md](AGENTS.md)「发版」。每一版由 `npm run release` 从上一版之后的提交生成。

## v0.1.6 · 2026-11-09

### 新功能

- **web**：图片预览支持多图左右切换（67c399e）
- 标签支持「仅人可打」类型（449da22）
- **web**：标签胶囊去掉圆点、新建卡片标题栏更紧凑（ed005c4）
- 支持 agent/CLI 将新任务记入未整理（b09331d）
- **web**：强制释放增加二次确认（78c3440）

### 修复

- **cli**：测试不再执行本机真实的 codex、dsh、pi（575e6c9）
- **web**：切换 Tab 后白板与机器页内容不再抖动（1283f52）
- **web**：详情分隔条去掉中间按钮，滚动条只在显示时靠近变粗（8a3ee16）
- **web**：新建时不再出现两张卡片，「设为默认」不再被冲回，保存中的对象先不能再改（89f1fa5）
- **cli**：dsh 派发禁用 goal 续轮并明示 board host 身份（e192889）
- **web**：拖动卡片靠近白板右边缘时内容区不再整体左移（f01f4e9）

## v0.1.5 · 2026-11-09

### 新功能

- **web**：界面写操作改为乐观更新：当帧生效、保存中、失败回滚（af6bc0b）

### 修复

- macOS 图标按模板留白；评论里的长代码块、表格不再撑出动态栏（7beab0e）

### 其他

- **cli**：host 端到端测试不再执行本机装的 codex（f1282fd）
- **cli**：结束进程组的测试等到组里的进程都被回收（e872069）

## v0.1.4 · 2026-11-09

### 新功能

- board host 按启动环境启动 agent，内置 codex（be40028）

### 修复

- **cli**：board host 在 Windows 上让 pi 找到 Git Bash（8f9c033）

## v0.1.3 · 2026-11-09

### 新功能

- **web**：机器页的 agent 标记换成各自的图标（53af45b）
- **cli**：board host 内置 dsh、pi，dsh 设为默认 agent；新增接入 agent 指南（d58e6a4）

## v0.1.2 · 2026-11-09

### 新功能

- 任务下发：board host 常驻接任务、手动派发、定时领取，界面加「机器」页（9b6a515）

### 修复

- **release**：同一张卡片的多个提交在更新日志里只留最新一条（f4c062e）

## v0.1.1 · 2026-11-09

### 新功能

- **web**：卡片详情底部操作栏：删除移到左边，新增「移动到」（dae2ef6）
- **web**：卡片详情左右栏分隔线可拖拽（a8b1aa8）
- 人可以原地修改自己发的评论（38bef7c）
- **web**：评论乐观发送，长评论输入区改成上下排（06920eb）
- **web**：全局滚动条静止时隐藏，滚动时显示发丝线、靠边展开成滑块（4548e7a）

### 修复

- **desktop**：修复 Windows 任务栏图标模糊（83fce8b）
- **web**：拖到锁定状态和下面一块的交界处时，锁的提示不再来回闪（7806c75）
- **skill**：约束 agent 先领卡再干活，避免抢卡和耗时失真（c59a33e）
- **desktop**：Windows 任务栏图标模糊（fe402a8）
- **release**：附注 tag 的说明里带上这一版的 changelog（ddc4e2e）
- **web**：最近动态占满侧栏剩余高度并压缩为单行（178632f）
- **web**：快捷提示词按可领取状态生成带标签提示词（0df5818）

### 重构

- **cli**：心跳整合进 board 命令行，skill 自带各平台的 board（73945bd）

### 其他

- **release**：更新日志和 GitHub Release 不再带卡片号（98279de）
- **desktop**：撤销记住窗口大小、位置和最大化状态（0660637）

## v0.1.0 · 2026-11-08

### 新功能

- **skills**：board-skills 按颗粒度拆卡、直推集成分支、合入后复盘（511bec9）
- **不兼容** agent 名字查重、agent 级心跳与 CLI 配置文件（063982f）
- **web**：状态可选卡片形态（卡片 / 单行 / 磁贴），终点默认磁贴（f50b154）
- **web**：状态块标题栏增加排序菜单，只改显示顺序（e5ecd31）
- **web**：标签区支持拖拽调整顺序，卡片上的标签跟着变（f6a9812）
- **web**：设置页给出按平台的 PATH 命令与 board doctor 自检入口（888cce2）
- **web**：最近动态改滚动分页，滚到底加载更早的动态（c157cd0）
- **server**：支持保存标签顺序 PUT /api/labels/order（43c06c2）
- **web**：快捷提示词按可领取状态生成，菜单默认收起单行（f55528a）
- **cli**：新增 board doctor 环境自检命令（ddb5431）
- **server**：新增动态分页接口 GET /api/events（4f40fa2）
- **web**：动态与头像显示 GitHub 头像和昵称（b03a53f）
- **web**：标签区去掉悬停底色，双击空白才进入编辑态（6bb9a52）
- **web**：工作流条随白板内容滚走，不再常驻顶部（f8b5865）
- **deploy**：GitHub Actions 自动部署：push main → 测试 → GHCR 镜像 → 服务器拉取重启（b0d6251 · 卡片 default/main#46）
- 记录并显示心跳次数（e2090bc · 卡片 default/main#44）
- **web**：卡片详情骨架屏：动态、附件加载中占位，区分没有动态与加载失败（df2d26b · 卡片 default/main#37）
- **web**：拖标签乐观更新：松手当帧出现 pending 标签，确认后转正，失败撤回（caad77d · 卡片 default/main#35）
- **web**：拖动标签加强目标高亮与落位动画（f06808b · 卡片 default/main#34）
- **web**：侧栏骨架屏：快照未到时标签、正在工作、最近动态、未整理、项目树占位（c47653c · 卡片 default/main#38）
- **web**：内容区骨架屏：Skeleton 组件 + 白板首屏与切换白板（0d52510 · 卡片 default/main#36）
- **desktop**：冷启动先显示本地启动页，修复慢网白屏（520d5a9 · 卡片 default/main#33）
- **desktop**：安装包自带 board 命令行，Windows 安装时写进 PATH（5f72af4）
- **web**：白板页头加「提示词」菜单，一键复制四种快捷提示词（e73856c · 卡片 default/main#31）
- **canvas**：去掉项目画布左下角的操作说明（941ce08）
- **prompt**：复制为提示词改成一句完整的话（807697b）
- **web**：工作流条画成有向图，拖拽连线（a32f26b）
- **flow**：工作流改为有向图，agent 只能沿连线移动（8190112）
- **heartbeat**：去掉 30 分钟领取时长，改成心跳计时（计时胶囊）（b85c1b3）
- **desktop**：新建令牌后在桌面端一键写入 BOARD_URL、BOARD_TOKEN（37debb7）
- **desktop**：记住窗口大小、位置和最大化状态（8f19679）
- **projects**：项目颜色默认随机，色板最后一个是取色器；编辑白板时可改项目颜色（4546d6a）
- **labels**：按标签筛选支持多选，不再往筛选框里写 label:（721c1ee）
- **heartbeat**：agent 心跳——skill 脚本每 10 秒报到，结束发 final，连续 5 次没收到判定异常退出（bed2bea）
- **settings**：设置页底部显示服务端版本和提交号（57b71ae）
- **settings**：新建令牌后给出一条写好环境变量的命令（72cfaf2）
- **board**：状态块里的卡片改成瀑布流排列（85a66e5）
- **labels**：侧栏标签胶囊悬停显示 ×，确认后删除标签（abffa35）
- **labels**：拖动侧栏的标签到卡片上，直接给卡片加上标签（113791b）
- **attachments**：卡片支持图片和附件（09ea69d）
- **deploy**：一条命令部署到服务器 deploy/deploy.sh（4513b28）
- 加入 GitHub 登录与访问令牌，CLI 支持连接服务端（ae7a833）

### 修复

- **web**：拖到不允许的状态时锁提示从第一帧起居中，锁图标定尺寸（3bc78a7）
- **web**：拖动卡片时占位位置只由指针决定，修复瀑布流里左右来回跳动（a91f151）
- **web**：logo 背景跟随主题，不再反色（9a17008）
- **web**：命令面板搜索行固定高度，不再被长列表压扁（986f1aa）
- **web**：去掉项目卡片左侧的项目色竖条（25b1e50）
- **deploy**：先读 GHCR 令牌再备份，避免 docker compose exec 吃掉 stdin（b96800c）
- **heartbeat**：final 之后不再出现「开始第 2 次」幽灵记录（e1b2f14）
- **web**：拖动标签时 ghost 停在指针下面（3f46f3d）
- **web**：首帧按主题上底色，深色主题加载时不再先显示白色（20f9df4）
- **web**：界面主题色跟随当前项目的颜色（dd9c875 · 卡片 default/main#21）
- **web**：卡片描述不再双击进入编辑（db7c589 · 卡片 default/main#45）
- **web**：空状态块至少一张卡片高（626516c · 卡片 default/main#32）
- **card**：去掉进行中卡片底部的领取倒计时条（74c822d）
- **ui**：卡片详情顶部去掉「复制 board show」按钮（a3b52de）
- **ui**：Windows 上快捷键显示成 Ctrl+1、Ctrl+Shift+N（c182747）
- **settings**：Agent 接入补充 Windows PowerShell 写法（2821727）
- **card**：领取倒计时条缩短时保持居中（368157c）
- **topbar**：悬停 agent 头像时搜索框不再抖动（4aa40a6）
- **deploy**：修复 deploy.sh --check 在 UTF-8 环境下报 unbound variable（37e36b2）
- **web**：移除「项目」侧栏底部的说明条（e47bd2f）
- **web**：移除「未整理」侧栏底部的说明条（df2c804）
- **desktop**：Windows 去掉系统标题栏，GitHub 登录改在默认浏览器完成（f378939）
- windwos 上 npm install 失败（80aaf99）

### 重构

- **cli**：必须配置 BOARD_URL/BOARD_TOKEN，删除直连本地库的分支（cc36b7d）
- **skills**：board-skills 挪到 .agents/skills，各家 agent 自动识别（c5747c9）

### 文档

- PRD v0.7 写入 agent 名字、agent 级心跳与 board login，更新 README 与部署说明（410b684）
- 重写 README 并加入界面截图（9c97ad1）
- 同步 CLI 必须配置 BOARD_URL/BOARD_TOKEN，删除「没设 BOARD_URL 就读写本地库」的说法（4484c7f）
- **deploy**：新增 GitHub Actions 自动部署需求文档（1ee7125）

### 其他

- 发版脚本 npm run release，推 tag 部署后建 GitHub Release（a9729be）
- 推 main 只跑测试、推 v* tag 才部署，新增 AGENTS.md（2e04988）
- gitignore（5c28464）
- **deploy**：删除手动部署脚本，DEPLOY_HOST 改从 Secret 读（9d15a61）
- **deploy**：仓库里不再写服务器 IP 和登录名（e3af861）
- **desktop**：提交 remember_theme 的自动生成权限文件（6e76e8c · 卡片 default/main#33）
- 清理构建产物与包管理器（d170087）
- 统一圆角，样式里不再写死圆角值（f8ddffe）
- 增加 Docker Compose 部署（6fd7bd7）
- 初始提交（PRD v0.4，本机运行版本）（894334b）
