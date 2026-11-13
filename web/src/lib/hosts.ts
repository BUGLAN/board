/* ==========================================================================
   任务下发（board host）：机器、派发的小工具（任务下发需求 §八）
   ========================================================================== */
import claudeIcon from '../assets/agents/claude.png';
import dshIcon from '../assets/agents/dsh.png';
import piIcon from '../assets/agents/pi.svg';
import { describeCron } from './cron';
import type { BoardInfo, Dispatch, DispatchResult, HostAgent, HostDir, HostInfo, ReadyCheck, SelfTestItem, Workspace } from './types';
import { agentColor } from './util';

/** agent 标记：已知的几种用它们自己的图标（取自各自的应用和官网）和主色；其他按标识取两个字母和颜色 */
export interface AgentMark {
  short: string;
  color: string;
  icon?: string;
  /** 图标本身是白底：加一圈细线，浅色主题下才看得出边 */
  light?: boolean;
}

const MARKS: Record<string, AgentMark> = {
  // Claude Code 的像素标志 Clawd（取自 Claude Code VS Code 扩展的 resources/clawd.svg，裁掉透明边后画成 64×64），不是 Claude 桌面应用的图标
  claude: { short: 'CC', color: '#D97757', icon: claudeIcon },
  // DeepSeek Harness 应用的图标
  dsh: { short: 'DS', color: '#4D6BFE', icon: dshIcon, light: true },
  // pi.dev 的 logo
  pi: { short: 'π', color: '#E5A93F', icon: piIcon },
  codex: { short: 'CX', color: '#10A37F' },
};

export function agentMark(id: string): AgentMark {
  return MARKS[id] ?? { short: id.slice(0, 2).toUpperCase(), color: agentColor(id) };
}

export const RESULT: Record<DispatchResult, { label: string; color: string }> = {
  review: { label: '已交审', color: 'var(--c-done)' },
  stuck: { label: '做不下去', color: 'var(--warn)' },
  crashed: { label: '异常退出', color: 'var(--danger)' },
  stopped: { label: '已停止', color: 'var(--text-3)' },
  timeout: { label: '超时', color: 'var(--danger)' },
  canceled: { label: '已取消', color: 'var(--text-3)' },
  lost: { label: '机器掉线', color: 'var(--danger)' },
  unclaimed: { label: '没领到', color: 'var(--text-3)' },
  split: { label: '已拆解', color: 'var(--c-done)' },
  nosplit: { label: '没拆出卡', color: 'var(--warn)' },
};

/** 拆解「未整理」里的条目的派发（不领取、不移动它，PRD §9.1） */
export const isSplit = (d: Pick<Dispatch, 'kind'>) => d.kind === 'split';

export const resultOf = (d: Dispatch) => (d.result ? RESULT[d.result] : { label: '已结束', color: 'var(--text-3)' });

export const SOURCE: Record<Dispatch['source'], string> = { manual: '手动', cron: '定时' };

export type HostState = 'full' | 'busy' | 'idle' | 'paused' | 'off';

/** 状态胶囊：满载 / 在跑 / 空闲 / 暂停接单 / 离线 */
export function hostState(h: HostInfo, running: number): { key: HostState; label: string; color: string } {
  if (!h.online) return { key: 'off', label: '离线', color: 'var(--text-3)' };
  if (h.paused) return { key: 'paused', label: '暂停接单', color: 'var(--warn)' };
  if (running >= h.max) return { key: 'full', label: '满载', color: 'var(--c-doing)' };
  if (running > 0) return { key: 'busy', label: '在跑', color: 'var(--c-done)' };
  return { key: 'idle', label: '空闲', color: 'var(--c-review)' };
}

export type OsKind = 'mac' | 'win' | 'linux';
export const osKind = (os: string): OsKind => (/mac|darwin|os x/i.test(os) ? 'mac' : /windows/i.test(os) ? 'win' : 'linux');

/* ---------- 环境检查（PRD §9.1）：没通过的机器不接这块白板的卡 ---------- */

/** 这台机器（按它的系统）要跑的检查命令；白板不检查时为空。和服务端 ReadyCheck::for_os 一致 */
export function checkFor(rc: ReadyCheck | undefined, os: string): string | null {
  if (!rc) return null;
  const pick = osKind(os) === 'win' && rc.win.trim() ? rc.win : rc.cmd;
  return pick.trim() || null;
}

export type Readiness = { key: 'none' } | { key: 'ok'; at: number } | { key: 'failed'; last: string; at: number } | { key: 'pending' };

/** 一台机器上一块白板的环境检查：不检查 / 通过 / 没通过（输出最后一行） / 还没检查（命令改了之后还没跑完、旧版 board host） */
export function dirReadiness(rc: ReadyCheck | undefined, h: HostInfo, d: HostDir | undefined): Readiness {
  const cmd = checkFor(rc, h.os);
  if (!cmd) return { key: 'none' };
  const r = d?.ready;
  if (!r || r.cmd !== cmd) return { key: 'pending' };
  if (r.ok) return { key: 'ok', at: r.at };
  const last = [...r.tail].reverse().find(l => l.trim())?.trim() ?? (r.code != null ? `退出码 ${r.code}` : '没有输出');
  return { key: 'failed', last, at: r.at };
}

/** 接这块白板的卡 */
export const readyOk = (r: Readiness) => r.key === 'none' || r.key === 'ok';

/** 环境检查没通过时的说明（派发对话框里写在那台机器旁边） */
export function readyWhy(r: Readiness): string {
  if (r.key === 'failed') return `环境检查没通过：${r.last}`;
  if (r.key === 'pending') return '环境检查还没跑完';
  return '';
}

/** 一块白板的设置（工作区里找） */
export const boardOf = (ws: Workspace | null, path: string): BoardInfo | undefined =>
  ws?.projects.flatMap(p => p.boards).find(b => b.path === path);

/** 能接这块白板的卡的机器：有这块白板目录的每台机器，带它的目录和环境检查结果 */
export function boardHosts(ws: Workspace | null, path: string) {
  const rc = boardOf(ws, path)?.readyCheck;
  return hostsOf(ws).flatMap(h => {
    const d = h.dirs.find(x => x.board === path);
    return d ? [{ h, d, ready: dirReadiness(rc, h, d) }] : [];
  });
}

/** 启动环境的名称：机器上报的列表里找，找不到时用标识 */
export function envName(h: HostInfo | undefined, id: string | null | undefined): string | null {
  if (!id) return null;
  return h?.envs?.find(e => e.id === id)?.name ?? id;
}

/** 开会话时排进去、等这个会话的 board next 开始的派发：不占空位，不在「排队」里列 */
export const inSession = (d: Dispatch) => d.state === 'queued' && d.session != null && d.session !== d.id;

/** 排队的原因；会话里排着的写「会话第 N/M 张」 */
export function queueReason(h: HostInfo | undefined, d?: Dispatch): string {
  if (d && inSession(d)) return `会话第 ${d.seq ?? '?'}/${d.total ?? '?'} 张`;
  if (!h) return '机器已移除';
  if (!h.online) return '等机器上线';
  if (h.paused) return '暂停接单';
  return '等空位';
}

/** 定时领取的摘要（「每天 23:00、工作日 09:00」，和机器详情里的胶囊一致）；没开或没设时为空，不显示占位 */
export const cronSummary = (h: HostInfo) => (h.cronOn && h.cron.length ? h.cron.map(describeCron).join('、') : '');

const NO_HOSTS: HostInfo[] = [];
const NO_DISPATCHES: Dispatch[] = [];
export const hostsOf = (ws: Workspace | null) => ws?.hosts ?? NO_HOSTS;
export const dispatchesOf = (ws: Workspace | null) => ws?.dispatches ?? NO_DISPATCHES;

/** 一张卡片排队或正在跑的派发 */
export const cardDispatch = (ws: Workspace | null, board: string, card: number) =>
  dispatchesOf(ws).find(d => d.board === board && d.card === card);

/** 一台机器在跑、排队的派发（在跑按机器名，排队按机器 ID）；排队不含会话里排着的（它们挂在会话正在跑的那一行下面） */
export function hostLoad(ws: Workspace | null, h: HostInfo) {
  const all = dispatchesOf(ws);
  return {
    running: all.filter(d => d.state === 'running' && d.host === h.name),
    queued: all.filter(d => d.state === 'queued' && d.hostId === h.id && !inSession(d)),
  };
}

/** 一个会话还排着的派发，按排好的顺序 */
export const sessionQueue = (ws: Workspace | null, session: number | undefined) =>
  session == null ? NO_DISPATCHES : dispatchesOf(ws).filter(d => inSession(d) && d.session === session);

/** 会话里排着的派发，它的会话正在跑的那一张（名字、卡片）；会话没在跑时为空 */
export const sessionRunning = (ws: Workspace | null, d: Dispatch) =>
  dispatchesOf(ws).find(x => x.state === 'running' && x.session === d.session);

/** 会话弹窗最多先画几张（排得再多也只占这么多行骨架） */
const SLOTS_MAX = 20;

/** 会话弹窗取回来之前已经知道的派发：打开它的那一行带来的（known），加上推送里这个会话在跑、排着的（live，较新，盖掉前者）。
 *  按会话里的先后排成第 1…N 张，N 取 seq、total 里最大的：知道的放在自己的位置，不知道的是 null（那一行、那一段画骨架）。
 *  一张都不知道时为空（画通用骨架）；旧版服务端没有 seq 时只按先后列出知道的 */
export function sessionSlots(id: number, known: readonly Dispatch[] | undefined, live: readonly Dispatch[]): (Dispatch | null)[] {
  const byId = new Map<number, Dispatch>();
  for (const d of [...(known ?? []), ...live]) if ((d.session || d.id) === id) byId.set(d.id, d);
  const list = [...byId.values()].sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0) || a.id - b.id);
  if (!list.length || list.some(d => d.seq == null || d.seq < 1)) return list;
  const n = Math.min(SLOTS_MAX, Math.max(list.length, ...list.map(d => Math.max(d.seq ?? 1, d.total ?? 1))));
  const slots: (Dispatch | null)[] = Array.from({ length: n }, () => null);
  for (const d of list) if (d.seq! <= n && !slots[d.seq! - 1]) slots[d.seq! - 1] = d;
  return slots;
}

/** 「最近」里同一个会话（同一个 agent 进程）的派发合成一组：按最近一张结束的时间排，组里新的在前 */
export function sessionGroups(list: Dispatch[]) {
  const groups: { session: number; t: number; list: Dispatch[] }[] = [];
  for (const d of list) {
    const key = d.session || d.id;
    const g = groups.find(x => x.session === key);
    if (g) g.list.push(d);
    else groups.push({ session: key, t: d.endedAt ?? d.createdAt, list: [d] });
  }
  return groups;
}

/** 多久之前：3 秒前 / 5 分钟前 / 2 小时前 / 3 天前 */
export function since(t: number, now: number) {
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return `${s} 秒前`;
  if (s < 3600) return `${Math.round(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.round(s / 3600)} 小时前`;
  return `${Math.round(s / 86400)} 天前`;
}

/** agent 在实际用的启动环境里看不到 board-skills 时的提醒（和 board host 终端里那行一致）；看得到、没找到命令、旧版 board host 返回 null */
export function skillMiss(h: HostInfo, a: HostAgent): string | null {
  if (!a.found || !a.no_skill) return null;
  const env = envName(h, a.env);
  return `${a.name}${env ? ` 在${env}里` : ''}看不到 board-skills：派给它的卡片会做不成，先装到 ${a.no_skill}（见 docs/agents.md）`;
}

/** 桌面端能一键安装 board-skills 的 agent → 它读用户级 skill 的目录（和 board host 内置模板、src-tauri/src/skill.rs 一致） */
const SKILL_HOME: Record<string, string> = {
  dsh: '~/.agents/skills',
  pi: '~/.agents/skills',
  claude: '~/.claude/skills',
  codex: '$CODEX_HOME/skills',
};

/** 看不到 board-skills 的 agent 分两组：桌面端能一键装的（在本机、不在 WSL 里、是认识的 agent），和只能手动装的 */
export function skillGaps(h: HostInfo) {
  const missing = h.agents.filter(a => skillMiss(h, a));
  const auto = missing.filter(a => SKILL_HOME[a.id] && !a.env?.startsWith('wsl:'));
  const dirs = [...new Set(auto.map(a => SKILL_HOME[a.id]))];
  const manual = missing.filter(a => !auto.includes(a));
  return { missing, auto, manual, dirs, places: places(missing), manualPlaces: places(manual) };
}

/** 该装到哪里，每个 agent 取第一个位置（「~/.claude/skills/board-skills 或 仓库的 …」→ 前半句），去重 */
const places = (as: HostAgent[]) => [...new Set(as.map(a => (a.no_skill ?? '').split(' 或 ')[0]).filter(Boolean))];

/** 本机桌面端跑的 board host 就是这台机器：配置里的机器名（空就是默认的主机名）和机器名一致，并且正在跑 */
export const isLocalHost = (h: HostInfo, local: { running: boolean; name: string; defaultName: string } | null) =>
  !!local?.running && (local.name.trim() || local.defaultName) === h.name;

/** 目录太长时中间省略：开头和最后一级对齐（两块并排看时左端一致），悬停看全路径 */
export type PathParts = { head: string; tail: string };
export const pathParts = (path: string): PathParts => {
  const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return cut > 0 ? { head: path.slice(0, cut + 1), tail: path.slice(cut + 1) } : { head: '', tail: path };
};

/** 只在本机（桌面端运行 board host 的那台）的机器详情里能选文件夹：
    别的机器上选的目录是那台机器上的，选了也没用；`--dir` 启动的行由启动参数固定。
    `isLocal` 是这台电脑正在跑的 board host 就是它（见 isLocalHost），`desktop` 是桌面端（只有它能弹系统的文件夹选择框） */
export const canPickDir = (d: { fixed?: boolean }, env: { isLocal: boolean; desktop: boolean }) => !d.fixed && env.isLocal && env.desktop;

/* ------------------------------ 添加机器：一行安装命令 ------------------------------ */

/** 机器名的规则（和 board-core 的 check_host_name 一样）：不对时返回原因 */
export function hostNameError(name: string): string | null {
  if (!name) return '填一个机器名';
  if (name.length > 24 || !/^[a-z0-9][a-z0-9.-]*$/.test(name)) return '机器名只能用小写字母、数字、. 和 -，以字母或数字开头，最长 24 个字符';
  return null;
}

const shQuote = (s: string) => (/^[A-Za-z0-9_./:=@%+~-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);
const psQuote = (s: string) => `'${s.replace(/'/g, "''")}'`;

/**
 * 「添加机器」给命令行机器的那一行（PRD §10.12）：下载服务端的安装脚本 → 安装 board → board login → board host --install。
 * macOS / Linux 是 curl | sh，Windows 是一行 PowerShell；值里有空格、引号时加引号
 */
export function installCommand(o: { origin: string; token: string; name: string; dirs: { board: string; path: string }[]; os: 'unix' | 'windows' }): string {
  const dirs = o.dirs.filter(d => d.board && d.path.trim()).map(d => `${d.board}=${d.path.trim()}`);
  if (o.os === 'windows') {
    const dir = dirs.length ? ` -Dir ${dirs.map(psQuote).join(',')}` : '';
    return `& ([scriptblock]::Create((irm ${psQuote(`${o.origin}/install.ps1`)}))) -Token ${psQuote(o.token)} -Name ${psQuote(o.name)}${dir} -Service`;
  }
  const dir = dirs.map(d => ` --dir ${shQuote(d)}`).join('');
  return `curl -fsSL ${shQuote(`${o.origin}/install.sh`)} | sh -s -- --token ${shQuote(o.token)} --name ${shQuote(o.name)}${dir} --service`;
}

/* ------------------------------ 自检（PRD §10.12「自检」） ------------------------------ */

/** 自检请求多久没做完算作废（和服务端 SELFTEST_STALE_MS 一致）：之后可以重新自检 */
export const SELFTEST_STALE_MS = 15 * 60 * 1000;

/** 自检现在的状态：正在做（谁、什么时候请求的、只查哪个 agent）、等机器上线、过期没做完；没有请求时为 null */
export function selftestState(h: HostInfo, now: number): { tone: 'busy' | 'warn'; text: string; agent: string | null } | null {
  const a = h.selftest?.ask;
  if (!a) return null;
  const who = a.agent ? (h.agents.find(x => x.id === a.agent)?.name ?? a.agent) : null;
  const what = who ? `自检 ${who}` : '自检';
  if (now - a.at >= SELFTEST_STALE_MS) return { tone: 'warn', text: `${what}没做完（${since(a.at, now)}请求的）：可以重新自检`, agent: a.agent ?? null };
  if (!h.online) return { tone: 'warn', text: `等机器上线后${what}`, agent: a.agent ?? null };
  return { tone: 'busy', text: `正在${what}：${who ? '问它一句' : '问每个 agent 一句、检查每块白板能不能推送'}，最长 3 分钟`, agent: a.agent ?? null };
}

/** 正在做自检（没过期）：这时不能再请求 */
export const selftestBusy = (h: HostInfo, now: number) => !!h.selftest?.ask && now - h.selftest.ask.at < SELFTEST_STALE_MS;

/** 自检结果一行一行：名称（agent 名、「<白板> 的仓库」、「board 版本」），按 agent、git、版本排 */
export function selftestRows(h: HostInfo): (SelfTestItem & { label: string })[] {
  const order = { agent: 0, git: 1, version: 2 } as const;
  return [...(h.selftest?.items ?? [])]
    .sort((a, b) => order[a.kind] - order[b.kind])
    .map(i => ({
      ...i,
      label: i.kind === 'agent' ? (h.agents.find(a => a.id === i.key)?.name ?? i.key) : i.kind === 'git' ? `${i.key} 的仓库` : 'board 版本',
    }));
}
