/* 定时领取的 cron 表达式（与 board-core 的 cron.rs 一致）：5 段「分 时 日 月 周」，支持 *、*\/n、a、a-b、a-b/n 和逗号列表；
   星期 0 和 7 都是周日。界面里就地校验，服务端再校验一次 */

const FIELDS: [string, number, number][] = [
  ['分', 0, 59],
  ['时', 0, 23],
  ['日', 1, 31],
  ['月', 1, 12],
  ['周', 0, 7],
];

const num = (s: string) => (/^\d+$/.test(s) ? Number(s) : NaN);

function fieldOk(raw: string, lo: number, hi: number): boolean {
  return raw.split(',').every(item => {
    const [base, step] = item.split('/');
    if (item.split('/').length > 2) return false;
    if (step !== undefined && !(num(step) > 0)) return false;
    let a: number;
    let b: number;
    if (base === '*') [a, b] = [lo, hi];
    else if (base.includes('-')) [a, b] = base.split('-').map(num) as [number, number];
    else [a, b] = [num(base), step !== undefined ? hi : num(base)];
    return Number.isInteger(a) && Number.isInteger(b) && a >= lo && b <= hi && a <= b;
  });
}

/** 旧的 HH:MM 换成「M H * * *」，表达式合并多余空白 */
export function normalizeCron(raw: string): string {
  const m = raw.trim().match(/^(\d{1,2}):(\d{2})$/);
  if (m && Number(m[1]) < 24 && Number(m[2]) < 60) return `${Number(m[2])} ${Number(m[1])} * * *`;
  return raw.trim().split(/\s+/).join(' ');
}

/** 不对时返回原因（写明哪一段不对），对时返回 null */
export function cronError(raw: string): string | null {
  const s = normalizeCron(raw);
  const parts = s ? s.split(' ') : [];
  if (parts.length !== 5) return `cron 表达式要有 5 段（分 时 日 月 周），现在是 ${parts.length} 段`;
  for (let i = 0; i < 5; i++) {
    const [name, lo, hi] = FIELDS[i];
    if (!fieldOk(parts[i], lo, hi)) return `第 ${i + 1} 段（${name}）不对：「${parts[i]}」，要在 ${lo}–${hi} 之间`;
  }
  return null;
}

const pad = (n: number) => String(n).padStart(2, '0');

const WEEK = ['日', '一', '二', '三', '四', '五', '六', '日'];

/** 给人看的说明：每天 / 工作日 / 每周几 / 每月几号 HH:MM、每 N 分钟、每 N 小时；界面组件生成的都能说出来，其他（旧的手写表达式）写原样 */
export function describeCron(raw: string): string {
  const s = normalizeCron(raw);
  let m = s.match(/^(\d+) (\d+) \* \* \*$/);
  if (m) return `每天 ${pad(+m[2])}:${pad(+m[1])}`;
  m = s.match(/^(\d+) (\d+) \* \* 1-5$/);
  if (m) return `工作日 ${pad(+m[2])}:${pad(+m[1])}`;
  m = s.match(/^(\d+) (\d+) \* \* ([0-7](?:,[0-7])*)$/);
  if (m) return `每周${[...new Set(m[3].split(',').map(Number).sort((a, b) => (a || 7) - (b || 7)).map(d => WEEK[d]))].join('、')} ${pad(+m[2])}:${pad(+m[1])}`;
  m = s.match(/^(\d+) (\d+) (\d+) \* \*$/);
  if (m) return `每月 ${+m[3]} 号 ${pad(+m[2])}:${pad(+m[1])}`;
  m = s.match(/^\*\/(\d+) \* \* \* \*$/);
  if (m) return `每 ${m[1]} 分钟`;
  m = s.match(/^0 \*\/(\d+) \* \* \*$/);
  if (m) return `每 ${m[1]} 小时`;
  return s;
}

/** 界面上选出来的定时：不让人手写 cron，由它生成表达式 */
export type Schedule =
  | { kind: 'daily' | 'weekdays'; h: number; m: number }
  /** days：0 = 周日 … 6 = 周六 */
  | { kind: 'weekly'; days: number[]; h: number; m: number }
  | { kind: 'monthly'; day: number; h: number; m: number }
  | { kind: 'minutes' | 'hours'; every: number };

/** 选出来的定时 → cron 表达式；选得不完整（每周没选哪天）时返回 null */
export function buildCron(s: Schedule): string | null {
  switch (s.kind) {
    case 'daily':
      return `${s.m} ${s.h} * * *`;
    case 'weekdays':
      return `${s.m} ${s.h} * * 1-5`;
    case 'weekly': {
      // 周一在前、周日在最后（和人读的顺序一致）
      const days = [...new Set(s.days)].sort((a, b) => (a || 7) - (b || 7));
      return days.length ? `${s.m} ${s.h} * * ${days.join(',')}` : null;
    }
    case 'monthly':
      return `${s.m} ${s.h} ${s.day} * *`;
    case 'minutes':
      return `*/${s.every} * * * *`;
    case 'hours':
      return `0 */${s.every} * * *`;
  }
}
