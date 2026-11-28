/* 卡片详情的返回 / 前进在 macOS 上（default/main#684）：⌘[ ⌘] 才是返回 / 前进；Option+← / → 是按词移动光标
   （加 Shift 是选词），不拦、不换卡 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cardBack } from '../lib/actions';
import { useData } from '../store/data';
import { mount, press, unmount } from '../../test/detailKeys';

vi.mock('../lib/util', async importOriginal => ({ ...(await importOriginal<typeof import('../lib/util')>()), isMac: true }));
vi.mock('../lib/actions', async importOriginal => ({ ...(await importOriginal<typeof import('../lib/actions')>()), cardBack: vi.fn(), cardFwd: vi.fn() }));

let host: HTMLElement;
beforeEach(() => {
  vi.mocked(cardBack).mockReset();
  host = mount();
});
afterEach(() => unmount());

const comment = () => host.querySelector<HTMLTextAreaElement>('textarea[aria-label="评论"]')!;

describe('macOS', () => {
  it('评论框里 Option+←：不拦（光标按词移动），不换卡', () => {
    const e = press(comment(), { key: 'ArrowLeft', altKey: true });
    expect(e.defaultPrevented).toBe(false);
    expect(cardBack).not.toHaveBeenCalled();
    expect(useData.getState().detailId).toBe(2);
    expect(press(comment(), { key: 'ArrowLeft', altKey: true, shiftKey: true }).defaultPrevented).toBe(false);
  });

  it('焦点不在输入框里：Option+← 也不换卡，⌘[ 返回', () => {
    press(document.body, { key: 'ArrowLeft', altKey: true });
    expect(cardBack).not.toHaveBeenCalled();
    press(document.body, { key: '[', metaKey: true });
    expect(cardBack).toHaveBeenCalledTimes(1);
  });

  it('评论框里 ⌘[：挡住浏览器后退，不换卡', () => {
    const e = press(comment(), { key: '[', metaKey: true });
    expect(e.defaultPrevented).toBe(true);
    expect(cardBack).not.toHaveBeenCalled();
  });
});
