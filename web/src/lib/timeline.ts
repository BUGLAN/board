/* ==========================================================================
   项目 Tab 的提交时间轴（PRD §10.4）用到的纯函数：分段、版本颜色、日期分隔、间隔文字
   数据和里程碑页同一份：GET /api/milestones（lib/milestones.ts 的类型）
   ========================================================================== */
import { sortCards, type MilestoneCard, type Milestones, type ReleaseCommit } from './milestones';
import type { Status } from './types';

/** 版本的颜色：按发版先后轮流取，相邻两版色相隔得远；新发一版不改老版本的颜色。
    不用橙色（「进行中」和未发版用它）和灰色 */
export const PALETTE = ['#3B82F6', '#EAB308', '#8B5CF6', '#22C55E', '#F43F5E', '#14B8A6', '#EC4899'];

/** 第 i 个发了版的版本（从最早的数起，0 开始）的颜色 */
export const verColor = (i: number) => PALETTE[((i % PALETTE.length) + PALETTE.length) % PALETTE.length];

/** 时间轴的一段：最上面的「未发版」，下面每个发了版的版本一段 */
export interface Seg {
  kind: 'next' | 'version';
  /** 未发版是 'next'，版本是版本标签的 id */
  key: string;
  name: string;
  color: string;
  /** 发版时间；未发版为空 */
  releasedAt: number | null;
  /** 上一个版本发版的时间（算间隔）；没有上一个版本为空 */
  prevAt: number | null;
  /** 这一版的提交（从新到旧）；未发版为空 */
  commits: ReleaseCommit[];
  /** 未发版：上次发版之后开始做的卡片（按里程碑页的顺序）；版本为空 */
  cards: MilestoneCard[];
}

/** 分段：board 是只看的白板路径（null 看整个项目）。
    - 只算有发版记录的版本（手动建的版本标签没有提交，不进时间轴）
    - 只看一块白板时，只留连到这块白板卡片的提交；一个提交都不剩的版本整段不显示 */
export function segmentsOf(data: Milestones, board: string | null, statusOf: (c: MilestoneCard) => Status | undefined): Seg[] {
  const bk = board?.split('/')[1];
  const hit = (c: ReleaseCommit) => !bk || c.cards.some(r => r.startsWith(`${bk}#`));
  const released = data.versions.filter((v): v is typeof v & { releasedAt: number } => v.releasedAt != null);
  const segs: Seg[] = [];
  const next = sortCards(
    data.cards.filter(c => c.group === 'next' && (!board || c.board === board)),
    statusOf,
  );
  if (next.length)
    segs.push({ kind: 'next', key: 'next', name: '未发版', color: 'var(--c-doing)', releasedAt: null, prevAt: released[0]?.releasedAt ?? null, commits: [], cards: next });
  released.forEach((v, i) => {
    const commits = v.commits.filter(hit);
    if (!commits.length) return;
    segs.push({
      kind: 'version',
      key: String(v.label),
      name: v.name,
      color: verColor(released.length - 1 - i),
      releasedAt: v.releasedAt,
      prevAt: released[i + 1]?.releasedAt ?? null,
      commits,
      cards: [],
    });
  });
  return segs;
}

/** 一段占几行（无限滚动按行数截断） */
export const rowsOf = (s: Seg) => s.commits.length + s.cards.length;

/** 按已画的行数截断：前面的段先画满，limit 用完后面的段不画 */
export function takeRows(segs: Seg[], limit: number): { seg: Seg; take: number }[] {
  let left = limit;
  return segs.map(seg => {
    const take = Math.max(0, Math.min(rowsOf(seg), left));
    left -= take;
    return { seg, take };
  });
}

/** 跳到一段要画到第几行（含这一段整段） */
export function rowsThrough(segs: Seg[], key: string) {
  let n = 0;
  for (const s of segs) {
    n += rowsOf(s);
    if (s.key === key) return n;
  }
  return n;
}

const WEEK = '日一二三四五六';
/** 11月28日；week 时带星期：11月28日 周六 */
export function dayLabel(t: number, week = false) {
  const d = new Date(t);
  return `${d.getMonth() + 1}月${d.getDate()}日${week ? ` 周${WEEK[d.getDay()]}` : ''}`;
}

/** 同一天的提交只在第一条前面写日期 */
export function withDays(commits: ReleaseCommit[]): { c: ReleaseCommit; day: string | null }[] {
  let last = '';
  return commits.map(c => {
    const d = dayLabel(c.at, true);
    const day = d !== last ? d : null;
    last = d;
    return { c, day };
  });
}

/** 两次发版之间多久：25 分钟 / 8 小时 / 2 天 */
export function gapText(ms: number) {
  const h = ms / 3600e3;
  if (h < 1) return `${Math.max(1, Math.round(ms / 60e3))} 分钟`;
  if (h < 24) return `${Math.round(h)} 小时`;
  return `${Math.round(h / 24)} 天`;
}

/** 页头右边的总数：19 个版本 · 193 个提交（只算版本，不算未发版）；一个版本都没有时为空 */
export function summary(segs: Seg[]) {
  const vs = segs.filter(s => s.kind === 'version');
  if (!vs.length) return null;
  return `${vs.length} 个版本 · ${vs.reduce((a, s) => a + s.commits.length, 0)} 个提交`;
}
