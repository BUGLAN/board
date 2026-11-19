/* 评论框占位文字（#520）：触屏设备（手机宽度、粗指针）不提示发送快捷键，桌面提示当前平台的快捷键 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { accel, commentPlaceholder, touchFirst } from './util';

const media = (matches: Record<string, boolean>) =>
  vi.stubGlobal('matchMedia', (q: string) => ({ matches: !!matches[q], media: q, addEventListener() {}, removeEventListener() {} }));

afterEach(() => vi.unstubAllGlobals());

describe('评论框占位文字', () => {
  it('390px 手机视口：只写「写评论…」，没有 Ctrl / ⌘ / ↵', () => {
    media({ '(max-width: 760px)': true, '(pointer: coarse)': true });
    expect(touchFirst()).toBe(true);
    const p = commentPlaceholder();
    expect(p).toBe('写评论…');
    expect(p).not.toMatch(/Ctrl|⌘|↵|Enter/);
  });

  it('宽屏但主指针是手指（平板）：同样不提示快捷键', () => {
    media({ '(pointer: coarse)': true });
    expect(commentPlaceholder()).toBe('写评论…');
  });

  it('桌面：写当前平台的发送快捷键', () => {
    media({});
    expect(touchFirst()).toBe(false);
    expect(commentPlaceholder()).toBe(`写评论…  ${accel('↵')} 发送`);
    expect(commentPlaceholder(false)).toMatch(/(⌘|Ctrl\+)↵ 发送$/);
  });
});
