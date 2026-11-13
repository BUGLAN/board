import { describe, expect, it } from 'vitest';
import { buildCron, describeCron, normalizeCron, parseSchedule } from './cron';

describe('cron 表达式（与服务端一致）', () => {
  it('旧的 HH:MM 换成每天这个时间；说明给人看', () => {
    expect(normalizeCron('03:00')).toBe('0 3 * * *');
    expect(describeCron('03:00')).toBe('每天 03:00');
    expect(describeCron('0 9 * * 1-5')).toBe('工作日 09:00');
    expect(describeCron('*/5 * * * *')).toBe('每 5 分钟');
    expect(describeCron('0 0 1 * *')).toBe('每月 1 号 00:00');
    expect(describeCron('30 8 * * 1,3,5')).toBe('每周一、三、五 08:30');
    expect(describeCron('0 9 * * 0,7')).toBe('每周日 09:00');
    expect(describeCron('0 */2 * * *')).toBe('每 2 小时');
    expect(describeCron('0,30 8-18/2 * * *')).toBe('0,30 8-18/2 * * *');
  });
  it('界面选出来的定时生成合法的表达式，说明和选的一致', () => {
    const cases: [Parameters<typeof buildCron>[0], string, string][] = [
      [{ kind: 'daily', h: 3, m: 0 }, '0 3 * * *', '每天 03:00'],
      [{ kind: 'weekdays', h: 9, m: 30 }, '30 9 * * 1-5', '工作日 09:30'],
      [{ kind: 'weekly', days: [5, 1, 1], h: 18, m: 5 }, '5 18 * * 1,5', '每周一、五 18:05'],
      [{ kind: 'weekly', days: [0, 6], h: 10, m: 0 }, '0 10 * * 6,0', '每周六、日 10:00'],
      [{ kind: 'monthly', day: 15, h: 23, m: 55 }, '55 23 15 * *', '每月 15 号 23:55'],
      [{ kind: 'minutes', every: 15 }, '*/15 * * * *', '每 15 分钟'],
      [{ kind: 'minutes', every: 1 }, '* * * * *', '每分钟'],
      [{ kind: 'hours', every: 6 }, '0 */6 * * *', '每 6 小时'],
    ];
    for (const [s, cron, text] of cases) {
      expect(buildCron(s)).toBe(cron);
      expect(describeCron(cron)).toBe(text);
    }
    expect(buildCron({ kind: 'weekly', days: [], h: 9, m: 0 })).toBeNull();
  });
  it('表达式反解析成选择组件的定时（定时建卡编辑时回填），再生成回同一个表达式；手写的返回 null', () => {
    for (const cron of ['0 3 * * *', '30 9 * * 1-5', '5 18 * * 1,5', '0 10 * * 6,0', '55 23 15 * *', '*/15 * * * *', '* * * * *', '0 */6 * * *']) {
      const s = parseSchedule(cron);
      expect(s, cron).not.toBeNull();
      expect(buildCron(s!)).toBe(cron);
    }
    expect(parseSchedule('0 9 * * 7')).toEqual({ kind: 'weekly', days: [0], h: 9, m: 0 });
    expect(parseSchedule('0,30 8-18/2 * * *')).toBeNull();
  });
});
