/* 全局快捷键（default/main#684）：复盘台、手机上的白板抽屉开着时也算弹窗，N 不在它们下面打开看不见的新建弹窗，
   ⌘1–8 不在下面换页；输入法组字时的 Esc 不关详情 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useData } from '../store/data';
import { overlayOpen, useUI } from '../store/ui';
import { globalKey } from './globalKeys';

const press = (init: KeyboardEventInit & { keyCode?: number }, target: EventTarget = document.body) => {
  const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  if (init.keyCode != null) Object.defineProperty(e, 'keyCode', { value: init.keyCode });
  Object.defineProperty(e, 'target', { value: target });
  globalKey(e);
  return e;
};

beforeEach(() => {
  useUI.setState({ review: null, boardSheet: false, creator: { open: false, column: 'todo', seq: 0 }, tab: 'board' } as never);
  useData.setState({ detailId: null } as never);
});

afterEach(() => {
  useUI.setState({ review: null, boardSheet: false, creator: { open: false, column: 'todo', seq: 0 }, tab: 'board' } as never);
});

describe('复盘台、白板抽屉开着时', () => {
  it('overlayOpen() 为 true', () => {
    expect(overlayOpen()).toBe(false);
    useUI.setState({ review: { cols: ['review'], mode: 'review' } } as never);
    expect(overlayOpen()).toBe(true);
    useUI.setState({ review: null, boardSheet: true } as never);
    expect(overlayOpen()).toBe(true);
  });

  it('按 N 不打开新建弹窗；关着时照常打开', () => {
    useUI.setState({ review: { cols: ['review'], mode: 'review' } } as never);
    press({ key: 'n' });
    expect(useUI.getState().creator.open).toBe(false);
    useUI.setState({ review: null } as never);
    press({ key: 'n' });
    expect(useUI.getState().creator.open).toBe(true);
  });

  it('⌘1–8 不在下面换页，但照样挡住浏览器自己的快捷键', () => {
    useUI.setState({ review: { cols: ['review'], mode: 'review' } } as never);
    const e = press({ key: '2', metaKey: true });
    expect(e.defaultPrevented).toBe(true);
    expect(useUI.getState().tab).toBe('board');
    useUI.setState({ review: null } as never);
    press({ key: '2', metaKey: true });
    expect(useUI.getState().tab).not.toBe('board');
  });
});

describe('输入法', () => {
  it('组字时的 Esc 不关详情', () => {
    useData.setState({ detailId: 3 } as never);
    const ta = document.createElement('textarea');
    press({ key: 'Escape', keyCode: 229 }, ta);
    press({ key: 'Escape', isComposing: true }, ta);
    expect(useData.getState().detailId).toBe(3);
  });
});
