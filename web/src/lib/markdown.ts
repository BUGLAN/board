/* Markdown：marked 解析 + DOMPurify 清理（描述和评论可能来自任何 agent，必须清理） */
import { marked } from 'marked';
import DOMPurify from 'dompurify';

marked.setOptions({ gfm: true, breaks: true });

const cache = new Map<string, string>();

export function markdown(src: string): string {
  const key = src || '';
  const hit = cache.get(key);
  if (hit != null) return hit;
  const raw = marked.parse(key, { async: false }) as string;
  const clean = DOMPurify.sanitize(raw, { ADD_ATTR: ['target', 'rel'] });
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
  root.querySelectorAll('li').forEach(li => {
    const box = li.querySelector(':scope > input[type="checkbox"], :scope > p > input[type="checkbox"]');
    if (!box) return;
    const checked = (box as HTMLInputElement).checked || box.hasAttribute('checked');
    box.remove();
    li.classList.add('task');
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
