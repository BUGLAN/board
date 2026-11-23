import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { card, snapshot } from '../../test/fixtures';
import { unitsOf } from '../lib/split';
import type { Card, Label } from '../lib/types';
import { useData } from '../store/data';
import { useUI } from '../store/ui';
import { Check, groupsOf, Row, useInbox } from './Inbox';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const box = () => host.querySelector('.ib-check') as HTMLElement;

describe('未整理的勾选框（default/main#315）', () => {
  it('是一个带名称、带勾选状态的勾选框，不再是 `.ib-check` 的空盒子里塞一个没有名字的 input', () => {
    act(() => root.render(<Check on={false} label="勾选 #9 调研：一个仓库是否需要多块白板" onToggle={() => {}} />));
    const el = box();
    expect(el.getAttribute('role')).toBe('checkbox');
    expect(el.tagName).toBe('BUTTON');
    expect(el.getAttribute('aria-checked')).toBe('false');
    expect(el.getAttribute('aria-label')).toBe('勾选 #9 调研：一个仓库是否需要多块白板');
    expect(el.querySelector('input')).toBeNull();
  });

  it('勾上、部分选中各自报出来', () => {
    act(() => root.render(<Check on label="取消勾选 #9" onToggle={() => {}} />));
    expect(box().getAttribute('aria-checked')).toBe('true');
    act(() => root.render(<Check on={false} some label="取消勾选「这一周」" onToggle={() => {}} />));
    expect(box().getAttribute('aria-checked')).toBe('mixed');
    act(() => root.render(<Check on some label="取消勾选「这一周」" onToggle={() => {}} />));
    expect(box().getAttribute('aria-checked')).toBe('mixed');
  });

  it('点一下切换，按住 Shift 时把 shiftKey 带出去', () => {
    const onToggle = vi.fn();
    act(() => root.render(<Check on={false} label="勾选 #9" onToggle={onToggle} />));
    act(() => box().click());
    expect(onToggle).toHaveBeenLastCalledWith(false);
    act(() => box().dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true })));
    expect(onToggle).toHaveBeenLastCalledWith(true);
  });

  it('空格、回车都能切换（空格不滚页面）', () => {
    const onToggle = vi.fn();
    act(() => root.render(<Check on={false} label="勾选 #9" onToggle={onToggle} />));
    const space = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true });
    act(() => {
      box().dispatchEvent(space);
    });
    expect(space.defaultPrevented).toBe(true);
    expect(onToggle).toHaveBeenCalledTimes(1);
    act(() => {
      box().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
      box().click();
    });
    expect(onToggle).toHaveBeenCalledTimes(2);
  });

  it('点勾选框不会顺便展开这一行', () => {
    const onToggle = vi.fn();
    const onRow = vi.fn();
    act(() =>
      root.render(
        <div onClick={onRow}>
          <Check on={false} label="勾选 #9" onToggle={onToggle} />
        </div>,
      ),
    );
    act(() => box().click());
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(onRow).not.toHaveBeenCalled();
  });
});

describe('未整理的倒序（default/main#497）', () => {
  const DAY = 86400000;
  const now = new Date(2026, 10, 19, 12).getTime();
  const lb = (id: number, name: string): Label => ({ id, name, color: '#888', uses: 0 });
  const labels = [lb(1, 'bug'), lb(2, 'UI')];
  // 已经按新到旧排好（useInboxCards 的顺序）：今天两条、这一周一条、放了两周以上两条；8、7 拆自隐藏的 6，算一个条目
  const list = [
    card(9, { createdAt: now - 1000, labels: [2] }),
    card(8, { createdAt: now - 2000, splitFrom: 6 }),
    card(7, { createdAt: now - 3000, splitFrom: 6 }),
    card(5, { createdAt: now - 3 * DAY, labels: [1], createdBy: { kind: 'agent', name: 'a/x' } }),
    card(4, { createdAt: now - 20 * DAY, labels: [2] }),
    card(3, { createdAt: now - 30 * DAY, labels: [1] }),
  ];
  const units = () => unitsOf(list, { 6: card(6, { createdAt: now - 4000 }) });
  const ids = (gs: ReturnType<typeof groupsOf>) => gs.map(g => [g.key, g.rows.map(c => c.id)]);

  it('默认（正序）不变：从新到旧', () => {
    expect(ids(groupsOf(units(), 'time', now, labels))).toEqual([
      ['today', [9, 8, 7]],
      ['week', [5]],
      ['stale', [4, 3]],
    ]);
  });

  it('按时间倒序：组的顺序和组里的条目都倒过来，拆解组整组挪、组里的卡照旧', () => {
    expect(ids(groupsOf(units(), 'time', now, labels, true))).toEqual([
      ['stale', [3, 4]],
      ['week', [5]],
      ['today', [8, 7, 9]],
    ]);
  });

  it('按标签 / 按来源倒序：组的顺序按原规则，只倒组里的条目', () => {
    expect(ids(groupsOf(units(), 'label', now, labels, true))).toEqual([
      ['l1', [3, 5]],
      ['l2', [4, 9]],
      ['none', [8, 7]],
    ]);
    const src = groupsOf(units(), 'source', now, labels, true);
    expect(src.map(g => g.key)).toEqual(groupsOf(units(), 'source', now, labels).map(g => g.key));
    expect(src.find(g => g.key.startsWith('human'))?.rows.map(c => c.id)).toEqual([3, 4, 8, 7, 9]);
  });

  it('重复算结果相同，不改动传进来的条目', () => {
    const us = units();
    const before = us.map(u => (u.kind === 'one' ? u.card.id : u.cards.map(c => c.id)));
    expect(ids(groupsOf(us, 'time', now, labels, true))).toEqual(ids(groupsOf(us, 'time', now, labels, true)));
    expect(us.map(u => (u.kind === 'one' ? u.card.id : u.cards.map(c => c.id)))).toEqual(before);
  });
});

describe('未整理列表的序号（default/main#625）', () => {
  const now = new Date(2026, 10, 23, 12).getTime();
  const writeText = vi.fn(async (_text: string) => {});
  const execCommand = vi.fn(() => false);

  beforeEach(() => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    Object.defineProperty(document, 'execCommand', { configurable: true, value: execCommand });
    writeText.mockClear();
    execCommand.mockClear();
    useData.setState({ path: 'default/main', board: snapshot('default/main', { inbox: [40, 41] }) });
    useInbox.setState({ checked: [], open: null, view: 'list', page: 'inbox' });
    useUI.setState({ toasts: [] });
  });

  const row = (id: number, over: Partial<Card> = {}) => {
    const c = card(id, { column: 'inbox', createdBy: { kind: 'agent', name: 'a/x' }, createdAt: now - 1000, ...over });
    act(() =>
      root.render(<Row card={c} now={now} labels={[]} all={[c]} home="待办" checked={false} open={false} onCheck={() => {}} />),
    );
    return host.querySelector('.ib-line') as HTMLElement;
  };

  it('有没有新的点，序号前都留着同样一格；点只出现在新条目上', () => {
    const line = row(40);
    const slot = line.querySelector('.ib-idwrap > .ib-dotslot') as HTMLElement;
    expect(slot).not.toBeNull();
    expect(slot.querySelector('.ib-newdot')?.getAttribute('aria-label')).toBe('新的');
    const id = line.querySelector('.ib-idwrap > .ib-id') as HTMLButtonElement;
    expect(id.tagName).toBe('BUTTON');
    expect(id.textContent).toBe('#40');
    expect(id.getAttribute('title')).toBe('点击复制卡片引用');
    expect(id.getAttribute('aria-label')).toBe('点击复制卡片引用 #40');

    const read = row(41, { seenAt: now - 500 });
    const readSlot = read.querySelector('.ib-idwrap > .ib-dotslot') as HTMLElement;
    expect(readSlot).not.toBeNull();
    expect(readSlot.querySelector('.ib-newdot')).toBeNull();
    expect(read.querySelector('.ib-idwrap > .ib-id')?.textContent).toBe('#41');
  });

  it('点序号复制项目/白板#ID 并提示，不展开、不勾选', async () => {
    const line = row(41, { seenAt: now - 500 });
    act(() => (line.querySelector('.ib-id') as HTMLElement).click());
    expect(writeText).toHaveBeenCalledWith('default/main#41');
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(useUI.getState().toasts.at(-1)?.title).toBe('已复制 default/main#41');
    expect(useInbox.getState().open).toBeNull();
    expect(useInbox.getState().checked).toEqual([]);
  });

  it('复制失败时提示失败，内容仍是这张卡片的引用', async () => {
    writeText.mockRejectedValueOnce(new Error('denied'));
    const line = row(41, { seenAt: now - 500 });
    act(() => (line.querySelector('.ib-id') as HTMLElement).click());
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(execCommand).toHaveBeenCalledWith('copy');
    expect(useUI.getState().toasts.at(-1)).toMatchObject({ type: 'error', title: '复制失败', text: 'default/main#41' });
  });
});
