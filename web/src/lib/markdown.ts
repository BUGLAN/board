/* Markdown：marked 解析 + DOMPurify 清理（描述和评论可能来自任何 agent，agent 又会读外部内容，必须清理）
   只留排版需要的：不能改写、遮盖界面（style、id、class），不能拼出表单，不能向外站发请求（外站图片、媒体）
   白板设了仓库地址时（PRD §4），`提交：<sha>` 和 gh#123 变成指向仓库网页的链接（见 decorate） */
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { repoWeb, type RepoWeb } from './repo';

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

/** repo：这块白板的仓库地址（PRD §4）；给了、能推出网页地址时，提交号和 gh#123 变成链接 */
export function markdown(src: string, repo?: string | null): string {
  const key = `${src || ''}\u0000${repo || ''}`;
  const hit = cache.get(key);
  if (hit != null) return hit;
  const raw = marked.parse(src || '', { async: false }) as string;
  const clean = purify.sanitize(raw, SANITIZE) as string;
  const out = decorate(clean, repoWeb(repo));
  if (cache.size > 400) cache.clear();
  cache.set(key, out);
  return out;
}

/** 任务列表换成自绘的勾选框；gh#123 高亮（设了仓库地址时还变成链接）；链接在新窗口打开 */
function decorate(html: string, repo: RepoWeb | null = null): string {
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
  // gh#123：总是高亮成 .gh；白板设了仓库地址时再变成这个 issue 的链接。
  // 提交：<sha>（7–40 位十六进制，交审评论里的那一行）：设了仓库地址时 sha 变成提交页的链接，没设时不动
  const commitRe = /提交[：:]\s*[0-9a-f]{7,40}\b/g;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const hits: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const t = n as Text;
    if (!(/gh#\d+/.test(t.data) || commitRe.test(t.data))) continue;
    commitRe.lastIndex = 0;
    if (!t.parentElement?.closest('code, pre, a')) hits.push(t);
  }
  for (const t of hits) {
    const frag = document.createDocumentFragment();
    let src = t.data;
    let last = 0;
    while (last < src.length) {
      const rest = src.slice(last);
      const gh = /gh#(\d+)/.exec(rest);
      const commit = repo ? /提交[：:]\s*([0-9a-f]{7,40})\b/.exec(rest) : null;
      const hit = !commit || (gh && gh.index < commit.index) ? gh : commit;
      if (!hit) break;
      const at = last + hit.index;
      frag.append(src.slice(last, at));
      if (hit === commit) {
        // 「提交：<sha>」：前面的「提交：」原样留着，sha 变成提交页的链接
        const sha = commit![1];
        frag.append(src.slice(at, at + hit[0].length - sha.length));
        const a = document.createElement('a');
        a.className = 'commit';
        a.href = repo!.commit(sha);
        a.target = '_blank';
        a.rel = 'noopener noreferrer';
        a.textContent = sha;
        frag.append(a);
      } else {
        const a = document.createElement(repo ? 'a' : 'span');
        a.className = 'gh';
        a.textContent = hit[0];
        if (repo) {
          (a as HTMLAnchorElement).href = repo.issue(Number((hit as RegExpExecArray)[1]));
          a.setAttribute('target', '_blank');
          a.setAttribute('rel', 'noopener noreferrer');
        }
        frag.append(a);
      }
      last = at + hit[0].length;
    }
    frag.append(src.slice(last));
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
