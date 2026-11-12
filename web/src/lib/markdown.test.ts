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

describe('markdown 不能改写界面、拼表单、向外站发请求（卡片 #99 的复现用例）', () => {
  it('<style> 去掉', () => {
    const d = dom(markdown('<style>.topbar{outline:4px solid red}</style>\n\n正文'));
    expect(d.querySelector('style')).toBeNull();
    expect(d.textContent).not.toContain('outline');
  });

  it('<form>、输入框、按钮去掉', () => {
    const d = dom(markdown('<form action="https://evil.example/x"><input name="pw" type="password"><button>登录</button><textarea></textarea><select><option>a</option></select></form>'));
    expect(d.querySelector('form, input, button, textarea, select')).toBeNull();
    expect(d.innerHTML).not.toMatch(/action=|formaction=/);
  });

  it('外站图片改成链接，不渲染 <img>；附件图片照常', () => {
    const d = dom(markdown('![追踪](https://evil.example/track.png)\n\n<img src="https://evil.example/t2.png">\n\n![截图](/api/files/12)'));
    const imgs = [...d.querySelectorAll('img')].map(i => i.getAttribute('src'));
    expect(imgs).toEqual(['/api/files/12']);
    const links = [...d.querySelectorAll('a')].map(a => [a.getAttribute('href'), a.textContent]);
    expect(links).toEqual([
      ['https://evil.example/track.png', '图片：追踪'],
      ['https://evil.example/t2.png', '图片：https://evil.example/t2.png'],
    ]);
  });

  it('id、style、借用的 class 去掉', () => {
    const d = dom(markdown('<span id="root" class="dialog-wrap toast" style="position:fixed;inset:0">假弹窗</span>'));
    const span = d.querySelector('span')!;
    expect(span.hasAttribute('id')).toBe(false);
    expect(span.hasAttribute('style')).toBe(false);
    expect(span.hasAttribute('class')).toBe(false);
    expect(d.querySelector('[id], [style]')).toBeNull();
  });

  it('SVG、iframe、媒体、srcset 去掉', () => {
    const d = dom(markdown('<svg><image href="https://evil.example/x.png"/></svg><video src="https://evil.example/v.mp4"></video><img src="/api/files/1" srcset="https://evil.example/a.png 2x"><iframe src="https://evil.example"></iframe>'));
    expect(d.querySelector('svg, image, video, iframe')).toBeNull();
    expect(d.querySelector('img')!.hasAttribute('srcset')).toBe(false);
  });
});

describe('markdown 正常内容不受影响', () => {
  it('代码块保留 language-* 类名', () => {
    const d = dom(markdown('```rust\nfn main() {}\n```'));
    expect(d.querySelector('code')!.className).toBe('language-rust');
  });

  it('表格、普通链接、附件链接', () => {
    const d = dom(markdown('| a | b |\n|---|---|\n| 1 | 2 |\n\n[文档](https://example.com/doc) [日志](/api/files/3)'));
    expect(d.querySelectorAll('table td').length).toBe(2);
    expect([...d.querySelectorAll('a')].map(a => a.getAttribute('href'))).toEqual(['https://example.com/doc', '/api/files/3']);
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
    // 没设仓库地址时只是高亮，不是链接；提交号也不动
    expect(d.querySelector('.gh')!.tagName).toBe('SPAN');
  });

  it('inlineCode：拆出 `代码`', () => {
    expect(inlineCode('改 `a.ts` 和 `b`')).toEqual(['改 ', { code: 'a.ts' }, ' 和 ', { code: 'b' }]);
    expect(inlineCode('没有代码')).toEqual(['没有代码']);
  });
});

describe('markdown 按白板的仓库地址生成链接（PRD §4）', () => {
  const repo = 'git@github.com:o/r.git';
  it('「提交：abc1234」的提交号变成提交页的链接，全角、半角冒号都认', () => {
    const d = dom(markdown('提交：abc1234\n检查：npm test 通过', repo));
    const a = d.querySelector('a.commit')!;
    expect(a.getAttribute('href')).toBe('https://github.com/o/r/commit/abc1234');
    expect(a.textContent).toBe('abc1234');
    expect(a.getAttribute('target')).toBe('_blank');
    expect(d.textContent).toContain('提交：abc1234');
    expect(d.textContent).toContain('检查：npm test 通过');
    const half = dom(markdown('提交: abc1234', repo)).querySelector('a.commit')!;
    expect(half.getAttribute('href')).toBe('https://github.com/o/r/commit/abc1234');
  });

  it('gh#12 变成 issue 的链接，样式仍高亮', () => {
    const a = dom(markdown('见 gh#12', repo)).querySelector('a.gh')!;
    expect(a.getAttribute('href')).toBe('https://github.com/o/r/issues/12');
    expect(a.textContent).toBe('gh#12');
  });

  it('代码块、行内代码里的不变成链接', () => {
    const d = dom(markdown('```\n提交：abc1234\ngh#1\n```\n\n`提交：abc1234` `gh#2`', repo));
    expect(d.querySelector('a.commit, a.gh')).toBeNull();
  });

  it('不是提交号的十六进制不动：太短、太长、后面紧跟着别的字符', () => {
    const d = dom(markdown('提交：abc123（6 位）\n提交：abc1234def\n提交：abc1234g', repo));
    expect([...d.querySelectorAll('a.commit')].map(a => a.textContent)).toEqual(['abc1234def']);
    expect(d.textContent).toContain('提交：abc123（6 位）');
    expect(d.textContent).toContain('提交：abc1234g');
  });
});
