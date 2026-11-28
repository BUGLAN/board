/* 输入法组字时的按键（default/main#684）：Chrome 里 isComposing 是 true；WebKit（macOS 桌面端、iOS）结束组字时
   先发 compositionend 再发 keydown，那时 isComposing 已经是 false，只能靠 keyCode 229 认出来 */
import { describe, expect, it } from 'vitest';
import { isImeKey } from './util';

const key = (init: KeyboardEventInit & { keyCode?: number }) => {
  const e = new KeyboardEvent('keydown', init);
  if (init.keyCode != null) Object.defineProperty(e, 'keyCode', { value: init.keyCode });
  return e;
};

describe('isImeKey', () => {
  it('认得出两种组字中的按键，原生事件和 React 事件都行', () => {
    expect(isImeKey(key({ key: 'Enter', isComposing: true }))).toBe(true);
    expect(isImeKey(key({ key: 'Enter', keyCode: 229 }))).toBe(true);
    expect(isImeKey(key({ key: 'Escape', keyCode: 229 }))).toBe(true);
    expect(isImeKey({ nativeEvent: key({ key: 'Enter', keyCode: 229 }) })).toBe(true);
  });

  it('平常的回车、Esc 不算', () => {
    expect(isImeKey(key({ key: 'Enter', keyCode: 13 }))).toBe(false);
    expect(isImeKey(key({ key: 'Escape', keyCode: 27 }))).toBe(false);
    expect(isImeKey({ nativeEvent: key({ key: 'Enter' }) })).toBe(false);
  });
});
