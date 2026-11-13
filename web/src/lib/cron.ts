/* 定时领取的 cron 表达式（与 board-core 的 cron.rs 一致）：5 段「分 时 日 月 周」，支持 *、*\/n、a、a-b、a-b/n 和逗号列表；
   星期 0 和 7 都是周日。界面里不让人手写表达式，全部由 buildCron 生成，校验由服务端做 */

/** 旧的 HH:MM 换成「M H * * *」，表达式合并多余空白 */
export function normalizeCron(raw: string): string {
  const m = raw.trim().match(/^(\d{1,2}):(\d{2})$/);
  if (m && Number(m[1]) < 24 && Number(m[2]) < 60) return `${Number(m[2])} ${Number(m[1])} * * *`;
  return raw.trim().split(/\s+/).join(' ');
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
  if (s === '* * * * *') return '每分钟';
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
      return s.every === 1 ? '* * * * *' : `*/${s.every} * * * *`;
    case 'hours':
      return `0 */${s.every} * * *`;
  }
}

/** cron 表达式 → 界面上选出来的定时（buildCron 的反过来）；不是选择组件能生成的（手写的）返回 null */
export function parseSchedule(raw: string): Schedule | null {
  const s = normalizeCron(raw);
  let m = s.match(/^(\d+) (\d+) \* \* \*$/);
  if (m) return { kind: 'daily', h: +m[2], m: +m[1] };
  m = s.match(/^(\d+) (\d+) \* \* 1-5$/);
  if (m) return { kind: 'weekdays', h: +m[2], m: +m[1] };
  m = s.match(/^(\d+) (\d+) \* \* ([0-7](?:,[0-7])*)$/);
  if (m) return { kind: 'weekly', days: [...new Set(m[3].split(',').map(d => +d % 7))], h: +m[2], m: +m[1] };
  m = s.match(/^(\d+) (\d+) (\d+) \* \*$/);
  if (m) return { kind: 'monthly', day: +m[3], h: +m[2], m: +m[1] };
  if (s === '* * * * *') return { kind: 'minutes', every: 1 };
  m = s.match(/^\*\/(\d+) \* \* \* \*$/);
  if (m) return { kind: 'minutes', every: +m[1] };
  m = s.match(/^0 \*\/(\d+) \* \* \*$/);
  if (m) return { kind: 'hours', every: +m[1] };
  return null;
}
