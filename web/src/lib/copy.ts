/* ==========================================================================
   复制为新卡片（PRD §10.8）：复制面板上列的原卡经历、默认带过去的标签、附在描述后面的人的评论、会复制的附件。
   会复制哪些附件和服务端（board-core Cmd::Copy）同一条规则：新描述里引用的、原卡的附件
   ========================================================================== */
import type { Activity, Attachment, CardDetail, Label } from './types';

/** 版本标签（发版时打的 v 开头标签，同里程碑）：新卡还没发版，默认不带 */
export const isVersionLabel = (name: string) => /^v\d/i.test(name.trim());

/** 原卡的经历：复制面板上列出来，都不带到新卡 */
export interface Past {
  runs: number;
  lost: number;
  /** agent、board host 写的报错（异常退出、没有输出被停、掉线） */
  errors: number;
  /** 评论里留下的未完成的改动（`未完成的改动：wip/…`，board base 会接着做的分支） */
  wips: string[];
  events: number;
  /** 人的评论，按时间 */
  human: Activity[];
}

const ERROR = /异常退出|没有任何输出|掉线|断开|SYSTEM_ERROR/;
const WIP = /未完成的改动[：:]\s*`?(wip\/[^\s`]+)/g;

export function pastOf(d: CardDetail): Past {
  const comments = d.activity.filter(a => a.kind === 'comment');
  const wips = new Set<string>();
  for (const a of comments) for (const m of a.text.matchAll(WIP)) wips.add(m[1]);
  return {
    runs: d.runs,
    lost: d.runsLost,
    errors: comments.filter(a => a.actor.kind !== 'human' && ERROR.test(a.text)).length,
    wips: [...wips],
    events: d.activity.length,
    human: comments.filter(a => a.actor.kind === 'human'),
  };
}

/** 默认带过去的标签：原卡的标签去掉版本标签，按项目里标签的顺序 */
export function carryLabels(ids: readonly number[], labels: readonly Label[]): number[] {
  return labels.filter(l => ids.includes(l.id) && !isVersionLabel(l.name)).map(l => l.id);
}

/** 人的评论附在描述最后一节，每条引用一段 */
export function withNotes(body: string, src: number, notes: readonly Activity[]): string {
  if (!notes.length) return body;
  const quoted = notes.map(a => a.text.trim().split('\n').map(l => (l ? `> ${l}` : '>')).join('\n')).join('\n\n');
  return `${body.trim()}\n\n## 原卡 #${src} 里人的意见\n\n${quoted}`;
}

/** 会复制到新卡的附件：新描述里引用的、原卡的附件（只在评论里引用的不复制） */
export function filesToCopy(d: CardDetail, body: string): Attachment[] {
  const used = new Set([...body.matchAll(/\/api\/files\/(\d+)/g)].map(m => Number(m[1])));
  return (d.attachments ?? []).filter(a => used.has(a.id));
}
