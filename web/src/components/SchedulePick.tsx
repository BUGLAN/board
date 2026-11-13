/* ==========================================================================
   选一个定时：「每天 / 工作日 / 每周几 / 每月几号」和时间，或「每隔几分钟 / 几小时」，由 lib/cron.ts 的 buildCron 生成表达式，
   不让人手写（手写容易错）。机器设置的「定时领取」（hosts/HostModal 的 CronTimes）和白板设置的「定时建卡」（CardCrons）共用
   ========================================================================== */
import type { Schedule } from '../lib/cron';
import { cx } from '../lib/util';
import { Select } from './bits';

const KINDS: [Schedule['kind'], string][] = [
  ['daily', '每天'],
  ['weekdays', '工作日'],
  ['weekly', '每周'],
  ['monthly', '每月'],
  ['minutes', '每隔几分钟'],
  ['hours', '每隔几小时'],
];
const WEEKDAYS: [number, string][] = [[1, '一'], [2, '二'], [3, '三'], [4, '四'], [5, '五'], [6, '六'], [0, '日']];
/** 「每隔几分钟」的默认档位；定时建卡多一个「1 分钟」（每分钟） */
export const EVERY_MINUTES = [5, 10, 15, 20, 30];
const EVERY_HOURS = [1, 2, 3, 4, 6, 8, 12];
const two = (n: number) => String(n).padStart(2, '0');
const KIND_OPTIONS = KINDS.map(([value, label]) => ({ value, label }));
const DAY_OPTIONS = Array.from({ length: 31 }, (_, i) => ({ value: String(i + 1), label: `${i + 1} 号` }));
const HOUR_OPTIONS = Array.from({ length: 24 }, (_, i) => ({ value: String(i), label: two(i) }));
const MIN_OPTIONS = Array.from({ length: 12 }, (_, i) => ({ value: String(i * 5), label: two(i * 5) }));

export const DEFAULT_SCHEDULE: Schedule = { kind: 'daily', h: 9, m: 0 };

/** 换「多久一次」：时间、每隔几分钟（小时）尽量沿用原来选的 */
export function switchKind(s: Schedule, kind: Schedule['kind'], minutes = EVERY_MINUTES): Schedule {
  const h = 'h' in s ? s.h : 9;
  const m = 'm' in s ? s.m : 0;
  switch (kind) {
    case 'minutes':
      return { kind, every: s.kind === 'minutes' ? s.every : minutes.includes(15) ? 15 : minutes[0] };
    case 'hours':
      return { kind, every: s.kind === 'hours' ? s.every : 1 };
    case 'weekly':
      return { kind, days: s.kind === 'weekly' ? s.days : [1], h, m };
    case 'monthly':
      return { kind, day: s.kind === 'monthly' ? s.day : 1, h, m };
    default:
      return { kind, h, m };
  }
}

/** 一排选择框：多久一次、星期几 / 几号、时:分（或每隔几分钟 / 几小时）。分钟按 5 分钟一档；原来的值不在档位里时也列出来 */
export function SchedulePick({ spec, onChange, minutes = EVERY_MINUTES, disabled }: { spec: Schedule; onChange: (s: Schedule) => void; minutes?: number[]; disabled?: boolean }) {
  const every = spec.kind === 'minutes' ? [...new Set([...minutes, spec.every])].sort((a, b) => a - b) : spec.kind === 'hours' ? [...new Set([...EVERY_HOURS, spec.every])].sort((a, b) => a - b) : [];
  const mins = 'm' in spec && spec.m % 5 ? [...MIN_OPTIONS, { value: String(spec.m), label: two(spec.m) }].sort((a, b) => +a.value - +b.value) : MIN_OPTIONS;
  return (
    <span className="hp-sched" role="group" aria-label="定时">
      <Select value={spec.kind} options={KIND_OPTIONS} aria-label="多久一次" disabled={disabled} onChange={v => onChange(switchKind(spec, v as Schedule['kind'], minutes))} />
      {spec.kind === 'weekly' && (
        <span className="hp-wdays">
          {WEEKDAYS.map(([d, n]) => {
            const on = spec.days.includes(d);
            return (
              <button
                key={d}
                type="button"
                className={cx('hp-wday', on && 'on')}
                aria-pressed={on}
                aria-label={`周${n}`}
                disabled={disabled}
                onClick={() => onChange({ ...spec, days: on ? spec.days.filter(x => x !== d) : [...spec.days, d] })}
              >
                {n}
              </button>
            );
          })}
        </span>
      )}
      {spec.kind === 'monthly' && (
        <Select value={String(spec.day)} options={DAY_OPTIONS} aria-label="几号" disabled={disabled} onChange={v => onChange({ ...spec, day: +v })} data-tip="没有这一天的月份（例如 2 月 30 号）不到点" />
      )}
      {'h' in spec ? (
        <span className="hp-hm">
          <Select value={String(spec.h)} options={HOUR_OPTIONS} aria-label="时" disabled={disabled} onChange={v => onChange({ ...spec, h: +v })} />
          :
          <Select value={String(spec.m)} options={mins} aria-label="分" disabled={disabled} onChange={v => onChange({ ...spec, m: +v })} />
        </span>
      ) : (
        <Select
          value={String(spec.every)}
          options={every.map(n => ({ value: String(n), label: spec.kind === 'minutes' && n === 1 ? '1 分钟（每分钟）' : `${n} ${spec.kind === 'minutes' ? '分钟' : '小时'}` }))}
          aria-label={spec.kind === 'minutes' ? '每隔几分钟' : '每隔几小时'}
          disabled={disabled}
          onChange={v => onChange({ ...spec, every: +v })}
        />
      )}
    </span>
  );
}
