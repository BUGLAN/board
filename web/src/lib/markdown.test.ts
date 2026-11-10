/* Markdown 清理：描述和评论可能来自任何 agent，渲染前必须去掉脚本、事件属性、javascript: 链接 */
import { describe, expect, it } from 'vitest';
import { inlineCode, markdown } from './markdown';

const dom = (html: string) => {
  const d = document.createElement('div');
  d.innerHTML = html;
  return d;
};

describe('markdown 清理注入', () => {
  it('去掉 <script>', () => {
    const out = markdown('hi\n\n<script>alert(1)</script>');
    expect(out).not.toMatch(/<script/i);
    expect(out).not.toContain('alert(1)');
    expect(out).toContain('hi');
  });

  it('去掉 javascript: 链接（Markdown 写法和 HTML 写法）', () => {
    for (const src of ['[点我](javascript:alert(1))', '<a href="javascript:alert(1)">点我</a>', '<a href="JaVaScRiPt:alert(1)">点我</a>']) {
      const a = dom(markdown(src)).querySelector('a');
      expect(a?.getAttribute('href') ?? '').not.toMatch(/javascript:/i);
    }
  });

  it('去掉事件属性（onerror、onclick）和 iframe', () => {
    const out = markdown('<img src="x" onerror="alert(1)">\n\n<p onclick="alert(2)">x</p>\n\n<iframe src="https://example.com"></iframe>');
    expect(out).not.toMatch(/onerror|onclick|<iframe/i);
  });
});

describe('markdown 装饰', () => {
  it('普通链接在新窗口打开；附件链接直接下载', () => {
    const d = dom(markdown('[外站](https://example.com) [附件](/api/files/1/a.txt)'));
    const [ext, file] = [...d.querySelectorAll('a')];
    expect(ext.getAttribute('target')).toBe('_blank');
    expect(ext.getAttribute('rel')).toBe('noopener noreferrer');
    expect(file.hasAttribute('download')).toBe(true);
    expect(file.getAttribute('target')).toBeNull();
  });

  it('任务列表换成自绘勾选框', () => {
    const d = dom(markdown('- [ ] 待做\n- [x] 做完'));
    const lis = d.querySelectorAll('li.task');
    expect(lis.length).toBe(2);
    expect(lis[1].classList.contains('checked')).toBe(true);
    expect(d.querySelector('input')).toBeNull();
  });

  it('gh#123 高亮，代码里的不动', () => {
    const d = dom(markdown('见 gh#12，`gh#34`'));
    expect([...d.querySelectorAll('.gh')].map(s => s.textContent)).toEqual(['gh#12']);
  });

  it('inlineCode：拆出 `代码`', () => {
    expect(inlineCode('改 `a.ts` 和 `b`')).toEqual(['改 ', { code: 'a.ts' }, ' 和 ', { code: 'b' }]);
    expect(inlineCode('没有代码')).toEqual(['没有代码']);
  });
});
