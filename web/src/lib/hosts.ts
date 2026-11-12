/* ==========================================================================
   任务下发（board host）：机器、派发的小工具（任务下发需求 §八）
   ========================================================================== */
import claudeIcon from '../assets/agents/claude.png';
import dshIcon from '../assets/agents/dsh.png';
import piIcon from '../assets/agents/pi.svg';
import { describeCron } from './cron';
import type { BoardInfo, Dispatch, DispatchResult, HostAgent, HostDir, HostInfo, ReadyCheck, Workspace } from './types';
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
  canceled: { label: '已取消', color: 'var(--text-3)' },
  lost: { label: '机器掉线', color: 'var(--danger)' },
  unclaimed: { label: '没领到', color: 'var(--text-3)' },
};

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

/** 定时领取的摘要：「每天 23:00、03:00」或「只接手动」 */
/** 定时领取的摘要（「每天 23:00、工作日 09:00」，和机器详情里的胶囊一致）；没开或没设时为空，不显示占位 */
export const cronSummary = (h: HostInfo) => (h.cronOn && h.cron.length ? h.cron.map(describeCron).join('、') : '');

/** 机器设置里实际会用到的 agent：默认 agent、领取规则和兜底里的 agent（去重、按这个顺序）；不是检测到的全部 */
export const usedAgents = (h: HostInfo) => {
  const ids = [h.agent, ...(h.rules ?? []).map(r => r.agent), h.fallback].filter((x): x is string => !!x);
  return [...new Set(ids)].map(id => h.agents.find(a => a.id === id)).filter((a): a is HostAgent => !!a);
};

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

/** 多久之前：3 秒前 / 5 分钟前 / 2 小时前 / 3 天前 */
export function since(t: number, now: number) {
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return `${s} 秒前`;
  if (s < 3600) return `${Math.round(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.round(s / 3600)} 小时前`;
  return `${Math.round(s / 86400)} 天前`;
}

/** 已用分钟 */
export const mins = (t: number | null, now: number) => (t == null ? 0 : Math.max(0, Math.floor((now - t) / 60000)));

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
