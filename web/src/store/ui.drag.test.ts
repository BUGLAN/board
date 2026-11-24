/* 拖拽预览的切换：上一张还在落位时开始拖下一张（default/main#657） */
import { afterEach, describe, expect, it } from 'vitest';
import { endDrop, setDrag, startDrag, useUI, type DragUI } from './ui';

const drag = (over: Partial<DragUI>): DragUI => ({ id: 1, fromCol: 'review', col: 'review', index: 0, over: 'review', valid: true, veto: null, ...over });

afterEach(() => useUI.setState({ drag: null, landing: [] }));

describe('startDrag / endDrop', () => {
  it('上一张已经松手、幽灵还在飞：它挪进 landing，和新的 drag 是同一次更新（中间没有一帧两边都没有它）', () => {
    const prev = drag({ id: 1, col: 'done', index: 3, dropping: true });
    setDrag(prev);
    const seen: { drag: number | undefined; landing: number[] }[] = [];
    const un = useUI.subscribe(s => seen.push({ drag: s.drag?.id, landing: s.landing.map(l => l.id) }));
    startDrag(drag({ id: 2 }));
    un();
    expect(seen).toEqual([{ drag: 2, landing: [1] }]);
    expect(useUI.getState().landing[0]).toBe(prev);
  });

  it('上一张没在落位（没有、或者还没松手）：landing 不变', () => {
    startDrag(drag({ id: 2 }));
    expect(useUI.getState().landing).toEqual([]);
    startDrag(drag({ id: 3 }));
    expect(useUI.getState()).toMatchObject({ drag: { id: 3 }, landing: [] });
  });

  it('连着拖几张：每张落位中的都留着；同一张不重复', () => {
    setDrag(drag({ id: 1, dropping: true }));
    startDrag(drag({ id: 2 }));
    setDrag(drag({ id: 2, dropping: true }));
    startDrag(drag({ id: 3 }));
    expect(useUI.getState().landing.map(l => l.id)).toEqual([1, 2]);
    setDrag(drag({ id: 3, dropping: true }));
    startDrag(drag({ id: 1 }));
    expect(useUI.getState().landing.map(l => l.id)).toEqual([2, 3]);
  });

  it('endDrop：落下的是落位中的那张时只从 landing 去掉，正在拖的不动；是当前的 drag 时清空 drag', () => {
    setDrag(drag({ id: 1, dropping: true }));
    startDrag(drag({ id: 2 }));
    endDrop(1);
    expect(useUI.getState()).toMatchObject({ drag: { id: 2 }, landing: [] });
    endDrop(2);
    expect(useUI.getState().drag).toBeNull();
  });

  it('endDrop：都对不上时不改 store（不通知订阅者）', () => {
    setDrag(drag({ id: 2 }));
    const before = useUI.getState();
    endDrop(7);
    expect(useUI.getState()).toBe(before);
  });
});
