/* 自动检查的提示决策（default/main#327）：只有 available 要弹，up-to-date / failed / idle 都静默 */
import { describe, expect, it } from 'vitest';
import { autoNotice } from './update-notice';
import type { CheckState } from './update';

const latest = { current: '0.1.0', version: null, notes: null, date: null };

describe('autoNotice', () => {
  it('发现新版本：弹确认框，带版本号', () => {
    const state: CheckState = { kind: 'available', result: { ...latest, version: '0.2.0' }, version: '0.2.0' };
    expect(autoNotice(state)).toEqual({ action: 'confirm', version: '0.2.0' });
  });

  it('已是最新、检查失败、空闲、检查中都不弹', () => {
    const states: CheckState[] = [
      { kind: 'up-to-date', result: latest },
      { kind: 'failed', error: 'unsupported。当前版本不受影响，可以重新检查' },
      { kind: 'idle' },
      { kind: 'checking' },
    ];
    for (const s of states) expect(autoNotice(s)).toEqual({ action: 'none' });
  });
});
