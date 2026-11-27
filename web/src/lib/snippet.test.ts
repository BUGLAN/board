import { describe, expect, it } from 'vitest';
import { snippet } from './snippet';

describe('snippet（描述首行）', () => {
  it('用第一行正文，跳过 Markdown 小标题', () => {
    expect(snippet('## 背景\n\n第一行正文\n第二行')).toBe('第一行正文');
  });

  it('只有小标题时退回用小标题', () => {
    expect(snippet('### 登录页按钮圆角')).toBe('登录页按钮圆角');
  });

  it('去掉列表、引用、勾选框符号，链接和图片只留文字', () => {
    expect(snippet('- [ ] 看 [设计稿](https://example.com)\n> 补充说明')).toBe('看 设计稿');
    expect(snippet('![截图](https://example.com/a.png)')).toBe('截图');
  });
});
