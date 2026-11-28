import { act, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { card, snapshot } from '../../test/fixtures';
import type { Cmd } from '../lib/types';
import { useData } from '../store/data';
import { TaskBody } from './TaskBody';

const { execMock } = vi.hoisted(() => ({ execMock: vi.fn() }));

vi.mock('../store/data', async importOriginal => {
  const actual = await importOriginal<typeof import('../store/data')>();
  return { ...actual, exec: (cmd: Cmd, patch?: unknown) => execMock(cmd, patch) };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
const body = '- [ ] 待做\n- [x] 做完';

beforeEach(() => {
  execMock.mockReset();
  execMock.mockResolvedValue(undefined);
  useData.setState({
    ws: null,
    path: 'p/a',
    board: snapshot('p/a', { todo: [1] }, [card(1, { body })]),
    saving: {},
    pending: {},
  } as never);
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
  useData.setState({ ws: null, board: null } as never);
});

describe('TaskBody 的任务勾选框', () => {
  it('Space / Enter 能勾选、取消，仍然走乐观更新', () => {
    act(() => root.render(<TaskBody card={card(1, { body })} />));
    const boxes = [...host.querySelectorAll<HTMLElement>('li.task > [role="checkbox"]')];
    expect(boxes.map(b => b.getAttribute('aria-checked'))).toEqual(['false', 'true']);

    act(() => boxes[0].dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true })));
    expect(execMock).toHaveBeenLastCalledWith({ op: 'edit', id: 1, body: '- [x] 待做\n- [x] 做完' }, expect.anything());

    act(() => boxes[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })));
    expect(execMock).toHaveBeenLastCalledWith({ op: 'edit', id: 1, body: '- [ ] 待做\n- [ ] 做完' }, expect.anything());
  });

  it('只读时不响应键盘', () => {
    act(() => root.render(<TaskBody card={card(1, { body })} locked />));
    const box = host.querySelector<HTMLElement>('li.task > [role="checkbox"]')!;
    act(() => box.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true })));
    expect(execMock).not.toHaveBeenCalled();
  });

  it('重画后焦点留在同一项，读屏能读到新状态', () => {
    const Live = () => {
      const [value, setValue] = useState(body);
      execMock.mockImplementation((cmd: Cmd) => {
        if (cmd.op === 'edit') setValue(cmd.body ?? value);
        return Promise.resolve();
      });
      return <TaskBody card={card(1, { body: value })} />;
    };
    act(() => root.render(<Live />));
    const box = host.querySelector<HTMLElement>('li.task > [role="checkbox"]')!;
    box.focus();
    act(() => box.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true })));
    expect(document.activeElement).toBe(host.querySelectorAll('li.task > [role="checkbox"]')[0]);
    expect(document.activeElement?.getAttribute('aria-checked')).toBe('true');
    expect(document.activeElement?.closest('li.task')?.textContent?.trim()).toBe('待做');
  });
});
