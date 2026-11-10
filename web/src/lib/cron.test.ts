import { describe, expect, it } from 'vitest';
import { cronError, describeCron, normalizeCron } from './cron';

describe('cron 表达式（与服务端一致）', () => {
  it('合法的通过，不对的写明哪一段', () => {
    for (const ok of ['*/5 * * * *', '0 9 * * 1-5', '0,30 8-18/2 1,15 */3 0,7', '03:00']) expect(cronError(ok)).toBeNull();
    expect(cronError('61 * * * *')).toContain('第 1 段（分）');
    expect(cronError('* * *')).toContain('5 段');
    for (const bad of ['* 24 * * *', '* * 0 * *', '* * * 13 *', '* * * * 8', '*/0 * * * *', '5-1 * * * *', 'a * * * *']) expect(cronError(bad)).not.toBeNull();
  });
  it('旧的 HH:MM 换成每天这个时间；说明给人看', () => {
    expect(normalizeCron('03:00')).toBe('0 3 * * *');
    expect(describeCron('03:00')).toBe('每天 03:00');
    expect(describeCron('0 9 * * 1-5')).toBe('工作日 09:00');
    expect(describeCron('*/5 * * * *')).toBe('每 5 分钟');
    expect(describeCron('0 0 1 * *')).toBe('0 0 1 * *');
  });
});
