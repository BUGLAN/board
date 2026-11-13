/* ==========================================================================
   里程碑（PRD §10.15）：GET /api/milestones 的类型，和页面用到的纯函数（分页、状态归类、进度、提交号）
   - 版本 = 名称是 v 加数字开头的标签；版本记录（发版时间、这一版的提交）由 board versions sync 从 git 反推后写进服务端
   - 一页是一个版本，另外两页：未发版（还没有版本标签、已经开始做的）、没有版本（最近一次发版之前做完却没有版本标签的）
   ========================================================================== */
import type { Status } from './types';

export interface ReleaseCommit {
  /** 短提交号 */
  h: string;
  at: number;
  /** 提交说明的第一行 */
  s: string;
  /** 连到的卡片：「白板#ID」（白板写标识） */
  cards: string[];
}

export interface VersionInfo {
  /** 版本标签的 id */
  label: number;
  name: string;
  color: string;
  /** 发版时间；还没有版本记录（手动建的标签）时为空 */
  releasedAt: number | null;
  /** 这一版的提交，从新到旧 */
  commits: ReleaseCommit[];
}

export interface MilestoneCard {
  /** 白板路径：项目/白板 */
  board: string;
  id: number;
  title: string;
  column: string;
  labels: number[];
  holder: string | null;
  createdAt: number;
  /** 移入现在这个状态的时间：终点状态里的就是做完的时间 */
  movedAt: number | null;
  group: 'version' | 'next' | 'none';
}

export interface Milestones {
  /** 版本号从新到旧 */
  versions: VersionInfo[];
  lastRelease: number | null;
  cards: MilestoneCard[];
}

/** 一页：版本标签的 id，或者「未发版」「没有版本」 */
export type MsKey = number | 'next' | 'none';

/** 版本标签：名称是 v 加数字开头 */
export const isVersionName = (name: string) => /^v\d/i.test(name.trim());

/** 页的顺序：未发版（有发版记录或者有卡片时）、版本从新到旧、没有版本（有卡片时） */
export function pagesOf(m: Milestones): MsKey[] {
  const out: MsKey[] = [];
  if (m.lastRelease != null || m.cards.some(c => c.group === 'next')) out.push('next');
  out.push(...m.versions.map(v => v.label));
  if (m.cards.some(c => c.group === 'none')) out.push('none');
  return out;
}

/** 默认打开哪一页：未发版有卡片就是它，否则最新的版本 */
export function defaultPage(m: Milestones): MsKey | null {
  const pages = pagesOf(m);
  if (pages[0] === 'next' && !m.cards.some(c => c.group === 'next')) return pages[1] ?? pages[0];
  return pages[0] ?? null;
}

export function cardsOf(m: Milestones, key: MsKey): MilestoneCard[] {
  if (key === 'next') return m.cards.filter(c => c.group === 'next');
  if (key === 'none') return m.cards.filter(c => c.group === 'none');
  return m.cards.filter(c => c.labels.includes(key));
}

export const versionOf = (m: Milestones, key: MsKey) => (typeof key === 'number' ? m.versions.find(v => v.label === key) ?? null : null);

/** 进度图的起点：这个版本之前最近一个发了版的版本的时间；未发版是最近一次发版 */
export function startOf(m: Milestones, key: MsKey): number | null {
  if (key === 'next') return m.lastRelease;
  if (key === 'none') return null;
  const i = m.versions.findIndex(v => v.label === key);
  if (i < 0) return null;
  return m.versions.slice(i + 1).find(v => v.releasedAt != null)?.releasedAt ?? null;
}

/** 卡片在「白板#ID」里的写法 */
export const cardRef = (c: Pick<MilestoneCard, 'board' | 'id'>) => `${c.board.split('/').pop()}#${c.id}`;

/** 这一版里每张卡片的提交（从新到旧） */
export function commitsByCard(v: VersionInfo | null): Map<string, ReleaseCommit[]> {
  const map = new Map<string, ReleaseCommit[]>();
  for (const c of v?.commits ?? []) for (const r of c.cards) map.set(r, [...(map.get(r) ?? []), c]);
  return map;
}

/** 这一版里没写「卡片：#ID」的提交 */
export const looseCommits = (v: VersionInfo | null) => (v?.commits ?? []).filter(c => !c.cards.length);

/* ------------------------------ 状态 ------------------------------ */

/** 状态在里程碑里的归类：做完 / 等人看 / 需要人处理（错误节点）/ 在做（锁定）/ 没开始（可领取） */
export type Role = 'done' | 'review' | 'error' | 'doing' | 'todo';
/** 进度条从左到右、各状态张数的顺序 */
export const ROLE_ORDER: Role[] = ['done', 'review', 'error', 'doing', 'todo'];

export function roleOf(s: Pick<Status, 'final' | 'error' | 'lock' | 'claim'> | undefined): Role {
  if (!s) return 'todo';
  if (s.final) return 'done';
  if (s.error) return 'error';
  if (s.lock) return 'doing';
  if (s.claim) return 'todo';
  return 'review';
}

export interface StatusCount {
  name: string;
  color: string;
  role: Role;
  n: number;
}

export interface Summary {
  total: number;
  done: number;
  pct: number;
  /** 各状态的张数（不同白板上同名的状态合在一起），按归类排序 */
  byStatus: StatusCount[];
}

/** statusOf：卡片所在的状态（按白板找）；找不到时归为「没开始」 */
export function summarize(cards: MilestoneCard[], statusOf: (c: MilestoneCard) => Status | undefined): Summary {
  const by = new Map<string, StatusCount>();
  let done = 0;
  for (const c of cards) {
    const s = statusOf(c);
    const role = roleOf(s);
    if (role === 'done') done++;
    const name = s?.name ?? c.column;
    const e = by.get(name) ?? { name, color: s?.color ?? 'todo', role, n: 0 };
    e.n++;
    by.set(name, e);
  }
  const byStatus = [...by.values()].sort((a, b) => ROLE_ORDER.indexOf(a.role) - ROLE_ORDER.indexOf(b.role));
  return { total: cards.length, done, pct: cards.length ? Math.round((done / cards.length) * 100) : 0, byStatus };
}

/** 一个版本里卡片的顺序：没做完的按归类（等人看、需要人处理、在做、没开始），同类里新的在前；做完的按做完的时间从新到旧 */
export function sortCards(cards: MilestoneCard[], statusOf: (c: MilestoneCard) => Status | undefined) {
  const rank = (c: MilestoneCard) => ROLE_ORDER.indexOf(roleOf(statusOf(c)));
  return [...cards].sort((a, b) => rank(a) - rank(b) || (b.movedAt ?? b.createdAt) - (a.movedAt ?? a.createdAt) || b.id - a.id);
}

/** 按标题、#编号筛 */
export function matchCard(q: string) {
  const s = q.trim().toLowerCase();
  if (!s) return () => true;
  const id = s.match(/^#?(\d+)$/)?.[1];
  return (c: MilestoneCard) => (id ? String(c.id).startsWith(id) : c.title.toLowerCase().includes(s));
}

const H = 3600e3;
const DAY = 24 * H;
/** 两次发版之间多久：6 小时 / 2 天 */
export const gap = (ms: number) => (ms < DAY ? `${Math.max(1, Math.round(ms / H))} 小时` : `${Math.round(ms / DAY)} 天`);
