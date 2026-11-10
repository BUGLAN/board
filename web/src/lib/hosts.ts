/* ==========================================================================
   任务下发（board host）：机器、派发的小工具（任务下发需求 §八）
   ========================================================================== */
import claudeIcon from '../assets/agents/claude.png';
import dshIcon from '../assets/agents/dsh.png';
import piIcon from '../assets/agents/pi.svg';
import type { Dispatch, DispatchResult, HostAgent, HostInfo, Workspace } from './types';
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
  // Claude 应用的图标
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

/** 启动环境的名称：机器上报的列表里找，找不到时用标识 */
export function envName(h: HostInfo | undefined, id: string | null | undefined): string | null {
  if (!id) return null;
  return h?.envs?.find(e => e.id === id)?.name ?? id;
}

/** 排队的原因 */
export function queueReason(h: HostInfo | undefined): string {
  if (!h) return '机器已移除';
  if (!h.online) return '等机器上线';
  if (h.paused) return '暂停接单';
  return '等空位';
}

/** 定时领取的摘要：「每天 23:00、03:00」或「只接手动」 */
/** 定时领取的摘要（「每天 23:00、03:00」）；没开或没设时间点时为空，不显示占位 */
export const cronSummary = (h: HostInfo) => (h.cronOn && h.cron.length ? `每天 ${h.cron.join('、')}` : '');

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

/** 一台机器在跑、排队的派发（在跑按机器名，排队按机器 ID） */
export function hostLoad(ws: Workspace | null, h: HostInfo) {
  const all = dispatchesOf(ws);
  return {
    running: all.filter(d => d.state === 'running' && d.host === h.name),
    queued: all.filter(d => d.state === 'queued' && d.hostId === h.id),
  };
}

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
