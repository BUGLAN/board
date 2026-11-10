/* Markdown：marked 解析 + DOMPurify 清理（描述和评论可能来自任何 agent，agent 又会读外部内容，必须清理）
   只留排版需要的：不能改写、遮盖界面（style、id、class），不能拼出表单，不能向外站发请求（外站图片、媒体） */
import { marked } from 'marked';
import DOMPurify from 'dompurify';

marked.setOptions({ gfm: true, breaks: true });

/** 单独的实例：钩子不影响别处的 DOMPurify */
const purify = DOMPurify(window);
// class 只留代码块高亮的 language-*（借用应用的类名能盖出假弹窗）
purify.addHook('uponSanitizeAttribute', (_node, data) => {
  if (data.attrName !== 'class') return;
  const keep = data.attrValue.split(/\s+/).filter(c => /^language-[\w+#.-]+$/.test(c));
  if (keep.length) data.attrValue = keep.join(' ');
  else data.keepAttr = false;
});
const SANITIZE = {
  USE_PROFILES: { html: true }, // 不要 SVG、MathML
  ADD_ATTR: ['target', 'rel'],
  FORBID_TAGS: ['style', 'link', 'meta', 'base', 'form', 'button', 'textarea', 'select', 'option', 'optgroup', 'fieldset', 'iframe', 'frame', 'object', 'embed', 'video', 'audio', 'source', 'track', 'picture'],
  FORBID_ATTR: ['style', 'id', 'name', 'action', 'formaction', 'srcset', 'ping', 'background', 'poster'],
};

/** 只有附件（同源的 /api/files/）显示成图片；外站图片会把「谁在什么时候看了这张卡」告诉外站，改成链接 */
const isAttachment = (src: string) => src.startsWith('/api/files/');

const cache = new Map<string, string>();

export function markdown(src: string): string {
  const key = src || '';
  const hit = cache.get(key);
  if (hit != null) return hit;
  const raw = marked.parse(key, { async: false }) as string;
  const clean = purify.sanitize(raw, SANITIZE) as string;
  const out = decorate(clean);
  if (cache.size > 400) cache.clear();
  cache.set(key, out);
  return out;
}

/** 任务列表换成自绘的勾选框；gh#123 高亮；链接在新窗口打开 */
function decorate(html: string): string {
  const tpl = document.createElement('template');
  tpl.innerHTML = html;
  const root = tpl.content;
  // input 只留任务列表的复选框（marked 生成的 type=checkbox disabled）
  root.querySelectorAll('input').forEach(i => {
    if (!(i.getAttribute('type') === 'checkbox' && i.hasAttribute('disabled'))) i.remove();
  });
  // 外站图片换成链接（template 里的图片不会加载）
  root.querySelectorAll('img').forEach(img => {
    const src = img.getAttribute('src') || '';
    if (isAttachment(src)) return;
    const label = img.getAttribute('alt') || src || '图片';
    if (/^https?:\/\//i.test(src)) {
      const a = document.createElement('a');
      a.href = src;
      a.textContent = `图片：${label}`;
      img.replaceWith(a);
    } else img.replaceWith(document.createTextNode(label));
  });
  let task = 0;
  root.querySelectorAll('li').forEach(li => {
    const box = li.querySelector(':scope > input[type="checkbox"], :scope > p > input[type="checkbox"]');
    if (!box) return;
    const checked = (box as HTMLInputElement).checked || box.hasAttribute('checked');
    box.remove();
    li.classList.add('task');
    // 第几个任务项（按源文本顺序），点击勾选时用 toggleTask 找回源文本里的那一行
    li.dataset.task = String(task++);
    if (checked) li.classList.add('checked');
    const span = document.createElement('span');
    while (li.firstChild) span.appendChild(li.firstChild);
    li.append(document.createElement('i'), span);
    const ul = li.parentElement;
    if (ul) ul.classList.add('tasks');
  });
  // 附件：图片延迟加载，点击放大（bits.tsx 的 Markdown）；文件链接直接下载（不开新窗口，桌面端新窗口没有登录状态）
  root.querySelectorAll('img').forEach(img => {
    img.setAttribute('loading', 'lazy');
    img.setAttribute('decoding', 'async');
  });
  root.querySelectorAll('a').forEach(a => {
    if ((a.getAttribute('href') || '').startsWith('/api/files/')) {
      a.setAttribute('download', a.textContent || '');
      a.classList.add('file-link');
      return;
    }
    a.setAttribute('target', '_blank');
    a.setAttribute('rel', 'noopener noreferrer');
  });
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const hits: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const t = n as Text;
    if (/gh#\d+/.test(t.data) && !t.parentElement?.closest('code, pre, a')) hits.push(t);
  }
  for (const t of hits) {
    const frag = document.createDocumentFragment();
    let last = 0;
    t.data.replace(/gh#\d+/g, (m, i: number) => {
      frag.append(t.data.slice(last, i));
      const s = document.createElement('span');
      s.className = 'gh';
      s.textContent = m;
      frag.append(s);
      last = i + m.length;
      return m;
    });
    frag.append(t.data.slice(last));
    t.replaceWith(frag);
  }
  const div = document.createElement('div');
  div.appendChild(root);
  return div.innerHTML;
}

/** 卡片标题：只处理 `code` */
export function inlineCode(text: string): (string | { code: string })[] {
  const out: (string | { code: string })[] = [];
  let last = 0;
  text.replace(/`([^`]+)`/g, (m, c: string, i: number) => {
    if (i > last) out.push(text.slice(last, i));
    out.push({ code: c });
    last = i + m.length;
    return m;
  });
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/* ---------- 任务项（- [ ] / - [x]）---------- */

const TASK = /^(\s*(?:>\s*)*(?:[-*+]|\d+[.)])\s+\[)([ xX])(\])(?=\s|$)/;
const FENCE = /^\s{0,3}(`{3,}|~{3,})/;

/** 源文本里每个任务项所在的行号（按顺序，和渲染出的 li.task 一一对应）；围栏代码块里的不算 */
export function taskLines(src: string): number[] {
  const out: number[] = [];
  let fence: string | null = null;
  src.split('\n').forEach((line, i) => {
    const f = line.match(FENCE);
    if (f) {
      if (!fence) fence = f[1][0].repeat(f[1].length);
      else if (f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      return;
    }
    if (!fence && TASK.test(line)) out.push(i);
  });
  return out;
}

/** 切换第 n 个任务项的勾选：只改那一行的 [ ] / [x]，其余原样；找不到时返回 null */
export function toggleTask(src: string, n: number): string | null {
  const at = taskLines(src)[n];
  if (at == null) return null;
  const lines = src.split('\n');
  lines[at] = lines[at].replace(TASK, (_m, a: string, mark: string, b: string) => `${a}${mark === ' ' ? 'x' : ' '}${b}`);
  return lines.join('\n');
}
