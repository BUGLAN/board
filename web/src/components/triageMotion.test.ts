import { afterEach, describe, expect, it, vi } from 'vitest';
import { ancestors, placeSlots, triageToastDelay, TRIAGE_TOAST_DELAY } from './triageMotion';

/* 「未整理 → 待办」的离场（default/main#361）：整理走的行原地放回一个槽，列表里的位置不变，下面的行跟着槽收起往上走 */

let root: HTMLElement;
afterEach(() => {
  root?.remove();
  delete document.documentElement.dataset.motion;
});

/** 搭一个和未整理页一样层级的列表：.ib-wrap > .ib-grp > .ib-rows > .ib-row[data-ib] */
function page(groups: number[][], tail = '') {
  root = document.createElement('div');
  root.className = 'ib-wrap';
  root.innerHTML =
    '<header class="ib-head"></header>' +
    groups.map((ids, g) => `<section class="ib-grp" id="g${g}"><div class="ib-gh"></div><div class="ib-rows">${ids.map(id => `<div class="ib-row" data-ib="${id}"><div class="ib-line"></div></div>`).join('')}</div></section>`).join('') +
    tail;
  document.body.appendChild(root);
  // 和 useRowMotion 一样，先记下各行和它往上的容器
  return [...root.querySelectorAll<HTMLElement>('[data-ib]')].map(el => [Number(el.dataset.ib), { el, up: ancestors(el, root) }] as [number, { el: HTMLElement; up: HTMLElement[] }]);
}
const row = (id: number) => root.querySelector<HTMLElement>(`[data-ib="${id}"]`)!;
/** 列表里一层一层的样子：行写编号，槽写 slot(里面的东西) */
const shape = (el: Element): string =>
  [...el.children]
    .map(c =>
      c.classList.contains('ib-leave')
        ? `slot(${shape(c)})`
        : c.classList.contains('ib-row')
          ? (c as HTMLElement).dataset.ib ?? `gone:${c.querySelector('.ib-line') ? 'row' : '?'}`
          : c.classList.contains('ib-grp')
            ? `${c.id}[${shape(c.querySelector('.ib-rows')!)}]`
            : '',
    )
    .filter(Boolean)
    .join(' ');

describe('整理走的行原地放回槽里（default/main#361）', () => {
  it('中间一行走了：槽放回原来的位置，行在槽里、不再算列表里的行', () => {
    const prev = page([[1, 2, 3]]);
    const el = row(2);
    el.remove(); // React 摘掉这一行
    const slots = placeSlots(root, prev, id => (id === 2 ? 'row' : false));
    expect(shape(root)).toBe('g0[1 slot(gone:row) 3]');
    expect(slots).toHaveLength(1);
    expect(slots[0].rows).toEqual([{ el, id: 2, how: 'row' }]);
    expect(el.hasAttribute('data-ib')).toBe(false);
    expect(slots[0].slot.getAttribute('aria-hidden')).toBe('true');
  });

  it('最后一行走了：槽放在容器最后', () => {
    const prev = page([[1, 2, 3]]);
    row(3).remove();
    placeSlots(root, prev, () => undefined);
    expect(shape(root)).toBe('g0[1 2 slot(gone:row)]');
  });

  it('相邻的几行一起走（批量）：各一个槽，顺序不变', () => {
    const prev = page([[1, 2, 3, 4]]);
    [2, 3].forEach(id => row(id).remove());
    const slots = placeSlots(root, prev, () => undefined);
    expect(slots.map(s => s.rows.map(r => r.id))).toEqual([[2], [3]]);
    expect(shape(root)).toBe('g0[1 slot(gone:row) slot(gone:row) 4]');
  });

  it('一组都走了（React 摘掉的是整组）：整组放进一个槽，放回下一组前面', () => {
    const prev = page([[1], [2, 3], [4]]);
    root.querySelector('#g1')!.remove();
    const slots = placeSlots(root, prev, () => 'edit');
    expect(slots).toHaveLength(1);
    expect(slots[0].rows.map(r => r.id)).toEqual([2, 3]);
    expect(slots[0].unit.id).toBe('g1');
    expect([...root.children].map(c => c.id || c.className)).toEqual(['ib-head', 'g0', 'ib-leave', 'g2']);
  });

  it('整块列表都空了：槽放在空状态前面，不跑到「全部整理完了」下面', () => {
    const prev = page([[1]], '<div class="ib-empty"></div>');
    root.querySelector('#g0')!.remove();
    placeSlots(root, prev, () => undefined);
    expect([...root.children].map(c => c.className)).toEqual(['ib-head', 'ib-leave', 'ib-empty']);
  });

  it('不认领的（删除、筛选掉的）不动；还在列表里的不问', () => {
    const prev = page([[1, 2, 3]]);
    row(2).remove();
    const claim = vi.fn(() => false as const);
    expect(placeSlots(root, prev, claim)).toEqual([]);
    expect(claim).toHaveBeenCalledTimes(1);
    expect(claim).toHaveBeenCalledWith(2);
    expect(shape(root)).toBe('g0[1 3]');
  });

  it('原来的容器也没了（放不回去）：不认领，留给调用方', () => {
    const prev = page([[1, 2]]);
    const grp = root.querySelector('#g0')!;
    row(2).remove();
    grp.remove(); // 行先被摘，组后来也没了：行原来挂的 .ib-rows 已经不在页面上
    const claim = vi.fn((id: number) => (id === 2 ? undefined : (false as const)));
    expect(placeSlots(root, prev, claim)).toEqual([]);
    expect(claim).not.toHaveBeenCalledWith(2);
    expect(row(2)).toBeNull();
  });
});

describe('提示等离场做完再出来', () => {
  it('平时晚一点出来，「减弱动效」时立刻出来', () => {
    expect(triageToastDelay()).toBe(TRIAGE_TOAST_DELAY);
    expect(TRIAGE_TOAST_DELAY).toBeGreaterThan(0);
    document.documentElement.dataset.motion = 'reduced';
    expect(triageToastDelay()).toBe(0);
  });
});
