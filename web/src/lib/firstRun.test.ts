import { describe, expect, it } from 'vitest';
import { checklistOf, goOf, missing } from './firstRun';
import { normWorkspace } from './normalize';
import type { Checklist, Workspace } from './types';

const list = (x: Partial<Checklist> = {}): Checklist => ({
  board: 'p/main',
  done: false,
  items: [
    { key: 'host', state: 'ok', why: 'mbp 在线' },
    { key: 'agent', state: 'unknown', why: '还没自检', go: 'selftest:mbp' },
    { key: 'repo', state: 'fail', why: '还没设仓库地址', go: 'project-repo' },
    { key: 'push', state: 'wait', why: '设了仓库地址之后再看' },
    { key: 'auto', state: 'fail', why: '还没有领取规则', go: 'rules', auto: true },
  ],
  ...x,
});

describe('跑第一张卡清单', () => {
  it('白板页的提示：没通过、还没确认的项；不算自动领取那一项，交审过之后不再提示', () => {
    expect(missing(list()).map(i => i.key)).toEqual(['agent', 'repo']);
    expect(missing(list({ done: true }))).toEqual([]);
    expect(missing(list({ items: [{ key: 'auto', state: 'fail', why: 'x', auto: true }] }))).toEqual([]);
    expect(missing(null)).toEqual([]);
  });

  it('去处：机器名里带冒号也拆得对；认不出的不给按钮', () => {
    expect(goOf('selftest:mbp')).toEqual({ kind: 'selftest', label: '自检', host: 'mbp' });
    expect(goOf('host-agents:a:b')).toEqual({ kind: 'host', label: 'a:b 的 agent', host: 'a:b', tab: 'agents' });
    expect(goOf('host-settings:win')).toMatchObject({ kind: 'host', tab: 'settings', host: 'win' });
    expect(goOf('project-check')).toEqual({ kind: 'project', label: '环境检查' });
    expect(goOf('add-host')?.label).toBe('添加机器');
    expect(goOf('accept')?.label).toBe('接单');
    expect(goOf('nope')).toBeNull();
    expect(goOf(null)).toBeNull();
  });

  it('旧服务端（工作区数据里没有清单）：不显示', () => {
    const ws = normWorkspace({ rev: 1, now: 0, projects: [] } as Workspace);
    expect(checklistOf(ws, 'p/main')).toBeNull();
    const ws2 = normWorkspace({ rev: 1, now: 0, projects: [], checklist: [{ board: 'p/main', done: false, items: null }] } as unknown as Workspace);
    expect(checklistOf(ws2, 'p/main')?.items).toEqual([]);
  });
});
