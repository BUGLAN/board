import { act, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useData } from '../store/data';
import { useRangeSelect, useSelectKeys } from './Selection';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({ detailId: null });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
});

function Harness({ order, canSelect }: { order: number[]; canSelect?: (id: number) => boolean }) {
  const [picked, setPicked] = useState<number[]>([]);
  const toggle = useRangeSelect(order, setPicked, { canSelect });
  useSelectKeys({
    ids: order.filter(id => canSelect?.(id) ?? true),
    onSelectAll: setPicked,
    onEscape: () => {
      if (!picked.length) return false;
      setPicked([]);
      return true;
    },
  });
  return (
    <>
      {order.map(id => (
        <button key={id} type="button" data-id={id} aria-pressed={picked.includes(id)} onClick={e => toggle(id, e.shiftKey)} />
      ))}
      <output>{picked.join(',')}</output>
    </>
  );
}

const button = (id: number) => host.querySelector<HTMLButtonElement>(`[data-id="${id}"]`)!;
const picked = () => host.querySelector('output')!.textContent;
const shiftClick = (id: number) =>
  act(() => button(id).dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true })));

describe('useRangeSelect（Shift 连选）', () => {
  it('从上一张连到这一张；临时卡不选进来', () => {
    act(() => root.render(<Harness order={[1, -1, 2, 3]} canSelect={id => id > 0} />));
    act(() => button(1).click());
    shiftClick(3);
    expect(picked()).toBe('1,2,3');
    expect(button(-1).getAttribute('aria-pressed')).toBe('false');
  });
});

describe('useSelectKeys（Esc / ⌘A）', () => {
  it('⌘A 只全选能选的，Esc 清空', () => {
    act(() => root.render(<Harness order={[1, -1, 2]} canSelect={id => id > 0} />));
    act(() => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', metaKey: true, bubbles: true })));
    expect(picked()).toBe('1,2');

    act(() => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(picked()).toBe('');
  });
});
