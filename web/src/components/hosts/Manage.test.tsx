import { describe, expect, it } from 'vitest';
import type { HostInfo } from '../../lib/types';
import { askState, versionLt } from './Manage';

const h = (x: Partial<HostInfo> = {}) => ({ name: 'win2', online: true, paused: false, ops: [], ...x }) as HostInfo;

describe('维护（远程管理）', () => {
  it('versionLt 和服务端一致：逐段按数字比', () => {
    expect(versionLt('0.1.9', '0.1.19')).toBe(true);
    expect(versionLt('v0.1.19', '0.2.0')).toBe(true);
    expect(versionLt('0.1.19', '0.1.19')).toBe(false);
    expect(versionLt('1.0.0', '0.9.9')).toBe(false);
  });

  it('请求的进展：等几个派发结束、正在做、已排空；没有请求时为空', () => {
    expect(askState(h(), 0)).toBeNull();
    expect(askState(h({ ask: { id: 1, kind: 'upgrade', by: 'me', at: 0, to: '0.2.0' } }), 2)?.text).toBe('等 2 个派发结束后升级到 0.2.0');
    expect(askState(h({ ask: { id: 1, kind: 'upgrade', by: 'me', at: 0, to: '0.2.0' } }), 0)?.text).toBe('正在升级到 0.2.0');
    expect(askState(h({ ask: { id: 1, kind: 'restart', by: 'me', at: 0 } }), 1)?.text).toBe('等 1 个在跑的派发停下后重启 board host');
    expect(askState(h({ drain: true, paused: true }), 1)).toEqual({ text: '排空中：等 1 个在跑的派发结束', tone: 'warn' });
    expect(askState(h({ drain: true, paused: true }), 0)).toEqual({ text: '已排空，可以关机或维护', tone: 'ok' });
  });
});
