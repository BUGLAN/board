/* 大盘的格式化和口径说明（接口的时长都是毫秒） */
import type { StatsBucket } from '../../lib/types';

const H = 3600_000;
const DAY = 24 * H;

/** 时长：不到 1 小时写分钟，不到 2 天写小时，再长写天 */
export function fmtDur(ms: number) {
  const m = Math.round(ms / 60_000);
  if (m < 60) return `${m} 分钟`;
  if (ms < 2 * DAY) {
    const h = ms / H;
    return `${h >= 10 ? Math.round(h) : h.toFixed(1).replace(/\.0$/, '')} 小时`;
  }
  const d = ms / DAY;
  return `${d >= 10 ? Math.round(d) : d.toFixed(1).replace(/\.0$/, '')} 天`;
}

/** 拆成数值和单位（大数字下面跟小单位） */
export function durParts(ms: number) {
  const [v, u] = fmtDur(ms).split(' ');
  return { v, u };
}

export const pct = (v: number | null | undefined) => (v == null ? '—' : `${Math.round(v * 100)}%`);
export const num = (v: number) => v.toLocaleString('zh-CN');

export type Metric = 'done' | 'created' | 'claims' | 'rework' | 'comments' | 'beats' | 'spent';
export const METRICS: { key: Metric; label: string; hint: string }[] = [
  { key: 'done', label: '完成', hint: '移入终点状态的卡片数（去重）' },
  { key: 'created', label: '新建', hint: '新建的卡片数' },
  { key: 'claims', label: '领取', hint: '开始的领取次数' },
  { key: 'rework', label: '返工', hint: '领取过又回到可领取状态的次数' },
  { key: 'comments', label: '评论', hint: '评论条数' },
  { key: 'beats', label: '心跳', hint: '这段时间开始的领取发出的心跳次数' },
  { key: 'spent', label: '累计耗时', hint: '这段时间开始且已结束的领取的耗时之和' },
];

/** 趋势图上的值：累计耗时换成小时（保留一位） */
export const metricValue = (b: StatsBucket, k: Metric) => (k === 'spent' ? Math.round(b.spent / 360_000) / 10 : b[k]);
export const metricFmt = (k: Metric) => (v: number) => (k === 'spent' ? `${v} 小时` : num(v));

/** 交付周期分布的 7 档（与服务端的档位一致） */
export const LEAD_BINS = [
  { label: '1 小时内', max: H },
  { label: '1–2 小时', max: 2 * H },
  { label: '2–4 小时', max: 4 * H },
  { label: '4–8 小时', max: 8 * H },
  { label: '8–24 小时', max: DAY },
  { label: '1–3 天', max: 3 * DAY },
  { label: '3 天以上', max: Infinity },
];
export const binOf = (ms: number) => LEAD_BINS.findIndex(b => ms < b.max);

/** 口径（鼠标停在块上显示） */
export const HINT = {
  done: '这段时间移入终点状态的卡片数（同一张卡只算一次）',
  lead: '这段时间完成的卡片，从新建到移入终点状态用了多久。P85：85% 的卡片在这个时间内完成',
  firstPass: '这段时间完成的卡片里，从新建到完成之间一次都没有返工的比例',
  rework: '领取过又回到可领取状态的次数（人把卡片打回）',
  claims: '这段时间开始的领取次数',
  idle: '可领取状态里有卡、而白板上没有任何进行中的领取的时间，单段满 15 分钟才计入',
};
