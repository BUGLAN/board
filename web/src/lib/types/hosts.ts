import type { Actor, Card } from '../types';

/* ---------------- 任务下发（board host） ---------------- */

/** 本机 agent：board host 启动时按配置文件和内置模板找到的（没找到的也上报） */
export interface HostAgent {
  /** 标识：claude、dsh、codex … */
  id: string;
  /** 名称：Claude Code … */
  name: string;
  /** `<命令> --version` 的第一行（在实际用的启动环境里） */
  version: string;
  /** 在实际用的启动环境里找到了命令 */
  found: boolean;
  /** 它支持、本机有的启动环境，按默认选择的顺序，各自的检查结果；旧版 board host 没有 */
  envs?: EnvCheck[];
  /** 实际用的启动环境（机器设置里选的，没选时按顺序第一个找到的）；旧版 board host 没有 */
  env?: string | null;
  /** 在实际用的启动环境里找到了命令、却看不到 board-skills：该装到哪里。看得到、旧版 board host 没有 */
  no_skill?: string | null;
  /** 实际用的启动环境里装了 agent、却缺 ACP 入口要的命令（例如「Node.js（npx）」）：这时 found 是 false（#458） */
  missing?: string | null;
  /** 这个 agent 现在用的模型（board host 读它的配置文件上报）：派发名字（<机器名>/<模型名>-<强度>）的第二段 */
  model?: string;
  /** 这个 agent 配置文件里现在的思考强度 */
  effort?: string;
  /** 机器设置里选的启动环境 */
  run_env?: string | null;
  /** 这个 agent 在这台机器上连续异常退出、已经暂停接单：点「恢复」才继续 */
  tripped?: boolean;
  /** 最近一次派发里看到的本机账号额度（#668）；没看到过、旧版服务端没有 */
  quota?: AgentQuota | null;
  /** 额度用完：这台机器上它的本机配置角色暂停接单到这个时间（毫秒），到点自动恢复，点「恢复」提前恢复 */
  quota_until?: number | null;
  /** 人点「刷新额度」的时间（毫秒）：比 quota.at、quota_fail.at 都新时是「正在查」 */
  quota_ask?: number | null;
  /** 最近一次查额度没查到 */
  quota_fail?: { at: number; why: string } | null;
  /** 这台机器的 board host 会按「刷新」查这个 agent 的额度（旧版 board host 不会） */
  quota_probe?: boolean;
  /** agent 自己用的身份（#667）：board host 启动时、版本变了、自检时问的；只有 Claude Code、Codex 有 */
  auth?: AgentAuth | null;
  /** 本机配置下账号能用的模型和每个模型的强度（#666），和身份一起问的；只有 Claude Code、Codex 有 */
  local_models?: LocalModels | null;
  /** 安装方式、最新版本、更新状态（内置 agent 才有；旧版 board host 没有） */
  manage?: AgentManage | null;
}

/** agent 自己用的身份（#667）：account 订阅 · api_key · gateway 网关 / 中转 · external 云服务商 · none 没登录 · unknown 问不出来 */
export interface AgentAuth {
  kind: string;
  /** 「Claude Max」「Anthropic API key」「ChatGPT Plus」 */
  label?: string;
  /** 来源：ANTHROPIC_API_KEY、网关的名字 */
  detail?: string;
  /** 套餐：max、plus… */
  plan?: string;
  /** 账号的邮箱（#669）：订阅登录才有；同一个 agent 报同一个邮箱的机器算同一个账号，额度用完一起暂停 */
  email?: string;
}

/** 本机配置下 agent 账号能用的模型（#666）：ACP session/options 给的 */
export interface LocalModels {
  models: LocalModel[];
  /** 默认模型、它的默认强度 */
  model?: string;
  effort?: string;
  /** 取到的时间（毫秒） */
  at?: number;
  /** 没取到的原因 */
  error?: string;
}

export interface LocalModel {
  id: string;
  name?: string;
  /** 这个模型能设的强度；空 = 不能设 */
  efforts: string[];
}

/** 订阅账号的额度（#668）：board acp 在派发里顺带看到的。Claude 带窗口、已用比例、重置时间；Codex 只在用完时报 */
export interface AgentQuota {
  /** 这一轮因为额度用完结束 */
  exhausted?: boolean;
  /** 什么时候恢复（毫秒） */
  resetsAt?: number | null;
  /** Claude 报的状态：allowed / allowed_warning / rejected */
  status?: string;
  /** 窗口：five_hour、seven_day、seven_day_opus… */
  window?: string;
  /** 这个窗口用了百分之几 */
  used?: number | null;
  /** 额度用完时 agent 的报错原文 */
  text?: string;
  /** 服务端收到的时间（毫秒） */
  at: number;
  /** 订阅账号的两个窗口：5 小时（five_hour）、本周（seven_day）。跑卡时 agent 推的、点「刷新」查到的 */
  windows?: QuotaWindow[];
}

/** 额度的一个窗口 */
export interface QuotaWindow {
  key: 'five_hour' | 'seven_day' | string;
  /** 用了百分之几 */
  used?: number | null;
  /** 什么时候重置（毫秒） */
  resetsAt?: number | null;
  /** Claude 报的这个窗口的状态：allowed / allowed_warning / rejected */
  status?: string;
}

/** 一个内置 agent 在这台机器上的安装和更新状态 */
export interface AgentManage {
  /** npm-global | brew | script | app | config | wsl | unknown | missing */
  method: string;
  /** 界面上显示的安装方式 */
  method_label: string;
  /** 没找到时给的安装命令 */
  install_command?: string;
  /** npm 上查到的最新版本 */
  latest_version?: string;
  /** 最近一次版本检查的时间 */
  checked_at?: number;
  /** 有新版本 */
  update_available?: boolean;
  /** 能由 board host 安装 / 升级 */
  installable?: boolean;
  updatable?: boolean;
  /** 不能代管的原因 */
  reason?: string;
  /** 空 = 空闲；running = 正在安装/更新；failed = 上一次失败 */
  state?: 'running' | 'failed' | '';
  /** 上一次失败的原因 */
  error?: string;
}

/** 一个 agent 的模型怎么配：不改（Board 不碰它的配置文件）/ 指定（这台机器上用哪个供应商、模型、强度、接口） */

/** 一个 agent 在一个启动环境里的检查结果 */
export interface EnvCheck {
  env: string;
  found: boolean;
  version: string;
  /** 找到了命令、却看不到 board-skills：该装到哪里 */
  no_skill?: string | null;
  /** 这个启动环境里的安装方式、最新版本和更新状态 */
  manage?: AgentManage | null;
  /** 装了 agent、却缺 ACP 入口要的命令（例如「Node.js（npx）」）：这时 found 是 false（#458） */
  missing?: string | null;
}

/** 启动环境：board host 在哪种 shell 里启动 agent（直接启动、登录 shell、cmd、PowerShell、Git Bash、WSL …） */
export interface HostEnv {
  /** direct、login、cmd、powershell、pwsh、git-bash、wsl:<发行版> */
  id: string;
  /** 直接启动、登录 shell、Git Bash、WSL（Ubuntu）… */
  name: string;
  /** 程序路径；WSL 是发行版名；直接启动为空 */
  path: string;
}

/** 一台机器的工作空间（PRD §9.1）：每个项目的目录是 `<工作空间>/<项目>`，项目下的白板共用 */
export interface HostWorkspace {
  /** 配置里写的路径（~ 不展开） */
  path: string;
  /** `--dir <项目>=<路径>` 单独指定的项目目录（旧写法 `<项目>/<白板>=<路径>` 只管那一块白板，兼容保留） */
  overrides?: { board: string; path: string }[];
}

export interface HostDir {
  /** 项目/白板 */
  board: string;
  /** 实际用的路径：所在项目的 `<工作空间>/<项目>`（`--dir` 覆盖的是覆盖的路径） */
  path: string;
  /** 命令行用 --dir 单独覆盖：界面只读 */
  fixed?: boolean;
  /** ok | missing | not_git；旧版 board host 没有 */
  status?: 'ok' | 'missing' | 'not_git' | null;
  /** 是 git 仓库时的 origin */
  origin?: string | null;
  /** 是 git 仓库时的当前分支 */
  branch?: string | null;
  /** board host 最近一次检查的时间 */
  checkedAt?: number | null;
  /** 目录不存在时的克隆状态 */
  clone?: HostClone | null;
  /** board host 上报过这块白板的目录状态 */
  reported?: boolean;
}

export interface HostClone {
  /** queued | running | done | failed */
  state: 'queued' | 'running' | 'done' | 'failed';
  url: string;
  message: string;
  updatedAt: number;
  by?: string;
}

/** 一台机器（运行 board host 的进程） */
export interface HostInfo {
  id: number;
  name: string;
  os: string;
  /** board 版本 */
  version: string;
  agents: HostAgent[];
  /** 启动环境（按发现的顺序）；旧版 board host 不上报，为空 */
  envs?: HostEnv[];
  /** 工作空间：每台设备一个目录，每个项目的目录由它派生（`<工作空间>/<项目>`）；还没配时没有 */
  workspace?: HostWorkspace | null;
  dirs: HostDir[];
  /** 服务端工作空间设置的版本 */
  dirsRev?: number;
  /** 最多几个会话：同一时间最多开几个 agent 进程（1–8） */
  max: number;
  /** 自动领取、批量派发排成的会话一个最多做几张（1–8，默认 4）；旧版服务端没有 */
  perSession?: number;
  /** 单次最长运行（分钟，0 = 不限）：所有机器一个值，在 Agent tab「跑多久停」里改（Sched.maxRun） */
  maxRun?: number;
  /** 多久没有输出算卡住（分钟，0 = 不限）：同上 */
  idleRun?: number;
  /** 领取规则（全局的一份，每台机器读到的都一样；active 按这台机器算）：没有一条规则时不自动领卡 */
  rules?: ClaimRule[];
  /** 每个角色在这台机器上能不能跑（按角色的排列顺序）；旧服务端没有 */
  fits?: ProfileFit[];
  /** 可领取、没有匹配任何规则的卡片数 */
  unmatched?: number;
  paused: boolean;
  /** 内置 agent 自动更新（默认关）：有新版本且没有在跑的派发时按实际安装方式更新；旧服务端没有这一项 */
  agentAuto?: boolean;
  /** 默认 agent（设置过且找到了就是它，否则第一个找到的） */
  agent: string | null;
  /** 设置的默认 agent 在这台机器上找不到时是它（例如从别的机器复制来的）：不生效 */
  agentSet?: string | null;
  /** 现在在自动接单：开了自动接单、没暂停、有领取规则、现在在接单时段里 */
  round: boolean;
  /** 参与自动接单；旧服务端没有这一项 */
  auto?: boolean;
  /** 接单时段：一天里的几段 [起, 止)，分钟，每天一样，按服务器时间；旧服务端没有这一项 */
  windows?: [number, number][];
  online: boolean;
  beatAt: number;
  offAt: number | null;
  /** 这次上线的时间：离线变在线、换了 board host 进程时更新（board host 连上来之前就检查过目录时取那次检查的时间）。
      只认不早于它的目录检查结果（`dirFresh`）；旧版服务端没有 */
  onlineAt?: number | null;
  /** 等 board host 处理的远程管理请求（重新检查、重启、升级）；旧版服务端没有 */
  ask?: HostAsk | null;
  /** 正在排空：暂停接单，等在跑的派发结束（都结束了就是「已排空」） */
  drain?: boolean;
  /** 最近一次「重新检查」做完的时间 */
  recheckedAt?: number | null;
  /** 最近的操作记录（新的在前）：谁、什么时间、做了什么 */
  ops?: HostOp[];
  /** 这台机器最近一次上报用的令牌；令牌已经吊销、还没上报过时为空 */
  token?: HostToken | null;
  /** 自检：等 board host 做的请求、最近一次的结果；旧版服务端没有 */
  selftest?: SelfTest;
  /** skills 用服务器的（库里的写进各个 agent 的 skill 目录）；否则不改（只装 board-skills） */
  syncSkills?: boolean;
  /** MCP 用服务器的；否则不改 */
  syncMcp?: boolean;
  /** Git 凭据用服务器的（写进用户级 git 配置，按主机全局生效）；否则不改。旧版服务端没有 */
  syncGit?: boolean;
  /** 同步配置：等 board host 做的请求、最近一次的结果 */
  sync?: ConfigSync;
  /** 从这台机器导入 skills：本机的 skill、等 board host 做的请求、最近一次的结果；旧版服务端没有 */
  skillImport?: SkillImport;
  /** 资源（CPU、内存、GPU、磁盘）：最近一次采样和最近几次的使用率；旧版 board host 不报，没有 */
  res?: HostRes;
}

/** 机器资源（PRD §10.12「资源」）：board host 后台每 10 秒采一次，随上报带给服务端 */
export interface HostRes {
  /** 采样时间（毫秒） */
  at: number;
  /** 取不到时没有 */
  cpu?: { pct: number; cores: number; load?: [number, number, number]; model?: string };
  /** 字节 */
  mem?: { total: number; used: number };
  /** 取不到（系统不支持）时没有；找过、没有 GPU 时是空数组 */
  gpus?: ResGpu[];
  /** 工作空间所在的盘排第一；取不到时没有 */
  disks?: ResDisk[];
  /** 最近几次的使用率（0–100，旧的在前，最多 30 个）；gpu 是第一块 */
  hist: { cpu: number[]; mem: number[]; gpu: number[] };
}

export interface ResGpu {
  name: string;
  vendor?: string;
  /** 使用率；读不到时没有 */
  pct?: number;
  /** 显存（字节）；Apple 芯片和内存共用，没有 */
  vramUsed?: number;
  vramTotal?: number;
}

export interface ResDisk {
  mount: string;
  /** 卷名（macOS 的「Macintosh HD」） */
  label?: string;
  total: number;
  avail: number;
  /** 工作空间在这块盘上 */
  ws?: boolean;
}

/** MCP 服务器的一个环境变量或请求头。列表接口里密钥的 value 是空的，tail 是末尾 4 位；保存时密钥留空表示保留原来的值 */
export interface McpVar {
  key: string;
  value?: string | null;
  secret?: boolean;
  tail?: string | null;
}

/** 同步配置（机器详情与配置同步重构）：服务器上一改就给受影响的机器下发请求，board host 写完上报 */
export interface ConfigSync {
  /** board host 会按请求同步、上报结果；旧版不会 */
  able?: boolean;
  /** 等 board host 做（或正在做）的同步 */
  ask?: { id: number; key: string; by: string; at: number } | null;
  /** 最近一次的结果 */
  last?: SyncResult | null;
}

export interface SyncResult {
  id?: number | null;
  key: string;
  at: number;
  /** skills：synced 库里的都写好了 / off 不改（只装了 board-skills）/ failed 有没写进去的 */
  skills: SkillsSync;
  /** MCP：synced 写好了 / off 不改 / failed 有没写进去的 */
  mcp: McpSync;
  /** Git 凭据：synced 配好了 / off 不改 / none 服务端没设 / failed 没配上；旧版 board host 不报（空） */
  git: GitSync;
}

/** 同步到机器的一类配置这一次的结果（skills、MCP、Git 凭据形状一样，见 syncKinds.ts）：
    共同的部分是状态、数量、原因（时间在外层），各自多出来的明细和它们平铺在同一层 */
export type SyncPart<D> = {
  state: 'synced' | 'off' | 'none' | 'failed' | '';
  /** 写进去几个：skill、MCP 服务器、配好的主机（Git 凭据；旧版 board host 不报） */
  count?: number;
  /** 没写进去的原因 */
  why?: string | null;
} & D;

export type SkillsSync = SyncPart<{
  count: number;
  /** 写进了哪几个 skill 目录（例如 ~/.claude/skills） */
  dirs: string[];
  /** 这一次换掉了本机同名的 */
  replaced: string[];
  /** 本机同名的是软链接，没有覆盖 */
  links: string[];
}>;

export type McpSync = SyncPart<{
  count: number;
  /** 写进了哪几个 agent（名称） */
  agents: string[];
  /** 这一次换掉了本机同名的条目 */
  replaced: string[];
}>;

export type GitSync = SyncPart<{
  /** 凭据的主机 */
  host?: string;
  /** 配好的仓库（目录）；按主机全局配置之后为空，旧版 board host 才报 */
  repos: string[];
  /** 没配的仓库：「目录：原因」（远端是 SSH）；按主机全局配置之后为空 */
  skipped: string[];
}>;

/** 照另一台机器配（机器详情「维护」、添加机器的「上线后照 <机器> 配」）：预览时 done 为 false */
export interface HostCopy {
  from: string;
  to: string;
  /** 要变的项（没有变化的不列）；空 = 已经一样 */
  changes: { key: string; label: string; from: string; to: string }[];
  /** 没照着改、要说一声的：系统不同启动环境没复制、这台机器上还没找到的 agent */
  notes: string[];
  done: boolean;
}

/** 服务端的 Git 凭据（共用配置）：令牌只有末尾 4 位 */
export interface GitCred {
  host: string;
  username: string;
  /** 提交作者的用户名（用户级 git 配置的 user.name） */
  commitName?: string;
  /** 提交作者的邮箱（用户级 git 配置的 user.email） */
  commitEmail?: string;
  /** 保存时填；列表里没有 */
  token?: string | null;
  tail?: string | null;
  version?: string;
  updatedAt?: number;
  by?: string;
  source?: string;
}

/** 一个文件夹（选工作空间时 board host 列出来的） */
export interface DirEntry {
  name: string;
  path: string;
  /** 它本身是 git 仓库 */
  git: boolean;
  /** 它下面一层有几个 git 仓库 */
  repos: number;
}

export interface DirList {
  /** 列的是哪个文件夹（~ 已经展开）；最上面一层为空 */
  path: string;
  /** 从最上面一层（空）到它自己的每一级 */
  chain: string[];
  entries: DirEntry[];
  error?: string | null;
}

/** 共用配置里的一个 MCP 服务器（PRD §10.16「共用配置」） */
export interface McpServer {
  name: string;
  kind: 'stdio' | 'http';
  command: string;
  args: string[];
  env: McpVar[];
  url: string;
  headers: McpVar[];
  /** 同步给哪些 agent；空 = 所有支持的（claude、codex） */
  agents: string[];
  version?: string;
  updatedAt?: number;
  by?: string;
  source?: string;
}

/* ---------------- 模型供应商（PRD §10.16「共用配置」） ---------------- */

/** 一个供应商的一个接口和地址（三种接口各一个地址） */
export interface ProviderEndpoint {
  interface: 'anthropic' | 'openai-chat' | 'openai-responses';
  url: string;
}

/** 共用配置里的一个模型供应商：API key 只存在服务端，列表里只有末尾 4 位 */
export interface Provider {
  name: string;
  endpoints: ProviderEndpoint[];
  /** 列表接口里为空，保存时留空 = 保留原来的值 */
  apiKey?: string | null;
  /** 密钥末尾 4 位 */
  tail?: string | null;
  models: string[];
  /** 每个模型支持的思考强度（从低到高）；没标的模型不能选强度 */
  efforts?: Record<string, string[]>;
  version?: string;
  updatedAt?: number;
  by?: string;
  source?: string;
}

/** 内置 agent 的说明（服务端 board-core 留一份）：认哪些接口、角色里能选哪几档强度 */
export interface AgentMeta {
  id: string;
  name: string;
  protocols: string[];
  efforts: string[];
  /** 读用户级 skill 的目录（第一个是装 board-skills 的地方）；旧版服务端没有 */
  skills?: string[];
  /** 支持 MCP 同步；旧版服务端没有 */
  mcp?: boolean;
  /** 界面上的主色；旧版服务端没有 */
  color?: string;
}

/** 列表接口：供应商（打码）、内置 agent 的说明 */
export interface ProviderView {
  providers: Provider[];
  meta?: AgentMeta[];
}

/** 一台机器上各 agent 的 skill 目录里、不是同步装的 skill（「从机器导入」用）；同名同内容在几个目录里的合成一项 */
export interface LocalSkill {
  name: string;
  /** 内容的版本号（和库里的算法一样）；读不了时为空 */
  version: string;
  files: number;
  size: number;
  agents: string[];
  /** 所在的 skill 目录（显示用），导入时用它找 */
  dirs: string[];
  /** 不能导入的原因 */
  why?: string | null;
}

export interface SkillImportPick {
  name: string;
  dir: string;
}

/** 从一台机器导入 skills（PRD §10.16「共用配置」） */
export interface SkillImport {
  /** 这台机器的 board host 会上报、会导入；旧版不会 */
  able: boolean;
  /** 会按请求重扫本机的 skill；旧版 board host 不会，只能看上次上报的缓存 */
  rescan: boolean;
  local: LocalSkill[];
  /** 等 board host 做（或正在做）的导入 */
  ask?: { id: number; picks: SkillImportPick[]; by: string; at: number } | null;
  /** 最近一次导入的结果 */
  last?: { id: number; at: number; by: string; ok: string[]; failed: { name: string; why: string }[] } | null;
}

/** skills 共用库里的一个 skill（PRD §10.16「共用配置」） */
export interface SkillInfo {
  name: string;
  /** 内容的哈希（12 位） */
  version: string;
  size: number;
  files: number;
  updatedAt: number;
  by: string;
  /** 从哪里上传：机器名、界面；board-skills 是「随服务端 x.y.z」 */
  source: string;
  /** board-skills：随服务端版本，不能删、不能改 */
  builtin?: boolean;
}

/** 自检（PRD §10.12「自检」）：agent 能不能干活、每个项目的目录能不能推送、board 版本是否落后 */
export interface SelfTest {
  /** 这台机器的 board host 会做自检；旧版不会（按钮置灰、要升级） */
  able?: boolean;
  /** 等 board host 做（或正在做）的自检，按请求的先后：几个 agent 的单查可以同时在，整台机器的请求和它们互斥 */
  asks: SelfTestAsk[];
  /** 最早的一个请求（给旧版 board host；界面看 asks） */
  ask?: SelfTestAsk | null;
  items: SelfTestItem[];
}

export interface SelfTestAsk {
  id: number;
  /** 只查这个 agent；空是整台机器 */
  agent?: string | null;
  by: string;
  at: number;
}

export interface SelfTestItem {
  kind: 'agent' | 'provider' | 'git' | 'version';
  /** agent 标识、供应商名称（连不连得上，整台自检才有）、白板路径；版本这一项是 board */
  key: string;
  ok: boolean;
  why?: string | null;
  /** agent 回答的前几行、git 输出的最后几行 */
  detail?: string | null;
  at: number;
}

/** 远程管理请求 */
export interface HostAsk {
  id: number;
  kind: 'recheck' | 'restart' | 'upgrade' | 'drain' | 'cancel' | 'agent-install' | 'agent-update' | 'rescan-skills';
  by: string;
  at: number;
  /** 升级：目标版本 */
  to?: string | null;
  /** 升级：不等在跑的派发结束 */
  now?: boolean;
  /** agent-install / agent-update：哪个 agent */
  agent?: string | null;
}

export interface HostOp {
  at: number;
  by: string;
  text: string;
}

export interface HostToken {
  id: number;
  name: string;
  /** 令牌开头几位 */
  hint: string;
  lastUsedAt: number | null;
  /** 同一个令牌还被这些在线的机器使用（这时不能随移除机器一起吊销） */
  shared?: string[];
}

export type DispatchResult = 'review' | 'stuck' | 'crashed' | 'stopped' | 'timeout' | 'canceled' | 'lost' | 'unclaimed' | 'split' | 'fine' | 'nosplit' | 'quota';

/** 一条领取规则：卡片的标签满足条件就由这个 agent 做 */
export interface ClaimRule {
  /** in 带其中任一个 · all 全都带 · not_in 一个都不带 · equal 标签正好是这几个 · only 只在这几个里 · none 没有标签 */
  op: 'in' | 'all' | 'not_in' | 'equal' | 'only' | 'none';
  labels: string[];
  /** 交给哪个角色（角色标识） */
  profile: string;
  /** 这个角色的 agent（服务端发出来时带着；旧服务端只有它） */
  agent?: string;
  /** 能跑这个角色（服务端算）：机器上的规则看这台机器，全局的规则看有没有任一台机器；都跑不了时这条规则不生效 */
  active?: boolean;
  /** 所有项目里都没有的标签（服务端算） */
  missing?: string[];
}

/** 调度的全局设置（随工作区数据下发） */
export interface Sched {
  /** 领取规则，有顺序：第一条命中的生效 */
  rules: ClaimRule[];
  /** 服务端的时区（没设时为空） */
  tz?: string;
  /** 取数据那一刻是服务端时区一天里的第几分钟 */
  minute: number;
  /** 自动升级：服务端的版本比机器新时，机器等手上的派发做完自己升级 */
  autoUpgrade?: boolean;
  /** 单次最长运行（分钟，0 = 不限），所有机器一样 */
  maxRun?: number;
  /** 没输出多久算卡住（分钟，0 = 不限），所有机器一样 */
  idleRun?: number;
  /** 角色（按排列顺序）：规则交给角色，派发选角色；旧服务端没有 */
  profiles?: Profile[];
}

/** 角色（PRD §9.1「分工」）：agent + 模型来源 + 强度，规则和派发按角色分 */
export interface Profile {
  id: string;
  name: string;
  agent: string;
  /** 共用配置里的供应商；空 = 本机配置（按这台机器上 agent 自己的配置跑）；@subscription = 官方订阅（#667） */
  provider: string;
  /** 模型；空 = agent 自己的默认（只有本机配置时可以空） */
  model: string;
  /** 思考强度；空 = 不设 */
  effort: string;
  /** 附加说明：派发时接在提示词后面 */
  note: string;
  /** 只在这些机器上跑（机器 id）；空 = 所有能跑的 */
  only: number[];
  sort?: number;
  createdAt?: number;
}

/** 一台机器能不能跑一个角色（服务端算）：没装这个 agent、不在角色的机器里……时 `ok` 是 false，`why` 写原因 */
export interface ProfileFit {
  profile: string;
  ok: boolean;
  why?: string;
}

/** 一次派发：在某台机器上用某种 agent 做某张卡 */
export interface Dispatch {
  id: number;
  board: string;
  card: number;
  title: string;
  /** 派给任意机器（服务端挑）：还没挑到机器时 host 是空的、hostId 是 0，没指定 agent 时 agent 也是空的 */
  any?: boolean;
  hostId: number;
  host: string;
  agent: string;
  agentLabel: string;
  /** 开始时生成的 agent 名字 <机器名>/<模型名>-<强度>（并发派发时末尾带 -<序号>） */
  name: string | null;
  /** 开始时用的启动环境标识与名称（Git Bash、WSL（Ubuntu）…）；旧版 board host、还没开始时为空 */
  env?: string | null;
  envName?: string | null;
  /** manual 手动 | cron 自动领取 | schedule 定时建卡 */
  source: 'manual' | 'cron' | 'schedule';
  by: Actor;
  state: 'queued' | 'running' | 'done';
  result: DispatchResult | null;
  createdAt: number;
  startedAt: number | null;
  endedAt: number | null;
  /** 有人请求停止（还在跑时）：谁 */
  stopBy: string | null;
  /** 持有者最近一次心跳 */
  beatAt: number | null;
  /** 同一个 agent 进程做的几张卡共用的会话标识：第一张的派发 ID。排队中、session 不是自己的，是开会话时排进去的（只由那个会话的 board next 开始）；旧版服务端没有 */
  session?: number;
  /** 这个会话的第几张、一共排了几张；不属于会话时 1、1；旧版服务端没有 */
  seq?: number;
  total?: number;
  /** 开始时传给 agent 的模型、思考强度（机器设置里选的）；没选、旧版时没有 */
  model?: string | null;
  effort?: string | null;
  /** '' = 做这张卡（领取）；'split' = 拆解「未整理」里的这条（不领取、不移动）。旧服务端没有 */
  kind?: string;
  /** 按哪个角色派的（角色标识）、派发时角色的名字；按 agent 派的、旧服务端没有 */
  profile?: string;
  profileName?: string;
  /** 角色用的供应商；本机配置时没有 */
  provider?: string;
}

/** 在线的 agent（PRD §3）：名字最近一次心跳在 50 秒以内 */
export interface AgentInfo {
  name: string;
  /** 这个会话跑在哪个 agent 里（claude、dsh、codex …）：名字里不再带标识，画图标用它 */
  agent?: string | null;
  /** 首次 check 的时间 */
  checkedAt: number;
  /** 最近一次心跳（只在上线、下线、卡片状态变化时推送，界面上的值不一定是最新的） */
  beatAt: number;
  /** 它持有的卡片（每块白板最多一张；可能没有） */
  cards: HeldCard[];
}

/** agent 持有的一张卡片：白板路径加上卡片本身 */
export type HeldCard = Card & { board: string };

/** agent 运行过程的一条结构化事件（会话「过程」，board-core runlog.rs） */
export interface RunEvent {
  id: number;
  /** 毫秒 */
  at?: number;
  kind: 'message' | 'tool' | 'error' | 'result' | 'text' | 'prompt';
  /** prompt：人在对话里发的消息（原样）；message：agent 说的话（Markdown）；error：出错信息；result：结束时的结果文字；text：解析不了的一行 */
  text?: string;
  /** tool：工具名、参数摘要、结果摘要、成功或失败（还没有结果时没有）、退出码、用时 */
  tool?: string;
  args?: string;
  out?: string;
  ok?: boolean;
  code?: number;
  ms?: number;
  /** result：轮数、输入 / 输出 tokens、费用（美元），agent 输出里有才有 */
  turns?: number;
  tokensIn?: number;
  tokensOut?: number;
  cost?: number;
  /** 子 agent 里的事件 */
  sub?: boolean;
  /** 超过 4 KB 截断过 */
  cut?: boolean;
  /** 对话里一轮的 result：人点了「停止这一轮」 */
  stopped?: boolean;
}

/** 机器对话（PRD §9.1「机器对话」）：在一台机器上和 agent 来回说话，不挂卡片、不占会话位；一直留着，随时能接着发 */
export interface Chat {
  id: number;
  hostId: number;
  host: string;
  agent: string;
  agentLabel: string;
  /** 按哪个角色开的、开的时候角色的名字；直接选 agent 时没有 */
  profile?: string;
  profileName?: string;
  provider?: string;
  model?: string | null;
  effort?: string | null;
  env?: string | null;
  envName?: string | null;
  /** 第一条消息 */
  title: string;
  by: string;
  createdAt: number;
  /** 最近一次有动静：发消息、一轮结束（列表按它排） */
  lastAt: number;
  /** busy 这一轮在跑（不能发下一条）| idle 等人发消息 */
  turn: 'busy' | 'idle';
  /** 在哪个项目的目录里跑（项目标识）；没有 = 主目录（#624） */
  project?: string;
  /** 实际路径（开始时按这台机器的工作空间算出，`~` 不展开） */
  cwd?: string | null;
}

/** 一个对话的全部内容：输出（最后 500 行）、事件（带 since 取时只有之后新增、改过的） */
export interface ChatView {
  chat: Chat;
  log: string[];
  fmt?: string | null;
  events: RunEvent[];
  eventsFrom: number;
  eventsDropped: number;
  runs?: number;
}

/** 一个会话（同一个 agent 进程用 board next 接着做的几张卡）：会话标识是第一张的派发 ID */
export interface SessionView {
  session: number;
  /** 这个会话的派发，按先后；每张带它的输出（最后 500 行）和结构化事件 */
  parts: (Dispatch & {
    log: string[];
    /** agent 的输出格式：claude、codex、dsh；'' = 只有原始输出；没有 = board host 没报（旧版，或还没开始） */
    fmt?: string | null;
    /** 事件（按先后）；带 since 取时只有之后新增、改过的 */
    events: RunEvent[];
    /** 留着的事件里最早的 id（0 = 一条都没有） */
    eventsFrom: number;
    /** 删掉了几条（超过 2000 条、结束 30 天后） */
    eventsDropped: number;
  })[];
  /** 事件的版本：下次带上 since 只取之后的 */
  runs?: number;
}
