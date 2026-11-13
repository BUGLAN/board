/* Markdown：marked 解析 + DOMPurify 清理（描述和评论可能来自任何 agent，agent 又会读外部内容，必须清理）
   只留排版需要的：不能改写、遮盖界面（style、id、class），不能拼出表单，不能向外站发请求（外站图片、媒体）
   白板设了仓库地址时（PRD §4），`提交：<sha>` 和 gh#123 变成指向仓库网页的链接；卡片编号变成打开卡片的链接（见 decorate） */
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { repoWeb, type RepoWeb } from './repo';
import { isTextFile } from './textFile';

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

/** 卡片引用：`#12`（本白板）、`项目/白板#12`。前面的字符不能是字母、数字、/、_、.、-，
    免得把 `gh#12`、`v1#2` 这类文字当成卡片（`gh#12` 单独高亮，见下） */
const CARD_REF_SRC = '(?<![a-z0-9/_.-])(?:([a-z0-9][a-z0-9-]{0,31})\\/([a-z0-9][a-z0-9-]{0,31}))?#(\\d{1,18})(?![\\w])';
export const CARD_REF = new RegExp(CARD_REF_SRC, 'g');
const CARD_REF_TEST = new RegExp(CARD_REF_SRC);

/** 交审评论里的「提交：<sha>」（7–40 位十六进制）；找到时 [1] 是 sha */
const COMMIT = /提交[：:]\s*([0-9a-f]{7,40})\b/;

/** 一张被引用的卡片（悬停提示用） */
export interface RefInfo {
  title: string;
  status: string;
}
/** 描述、评论里的卡片编号：怎么解析成一张卡片（由界面按当前数据提供，lib/xref.ts） */
export interface Refs {
  /** `#12` 指向这块白板 */
  path: string;
  /** 一张卡片：info 是标题和状态；null 是不存在（不画成链接）；undefined 是还查不到（先按链接画，查到后重画） */
  find: (board: string, id: number) => RefInfo | null | undefined;
}

/** markdown 的额外参数（都会影响结果、参与缓存） */
export interface MdOpts {
  /** 卡片编号怎么解析（不给时 `#12` 原样显示） */
  refs?: Refs;
  /** 这块白板的仓库地址（PRD §4）：给了、能推出网页地址时提交号和 gh#123 变成链接 */
  repo?: string | null;
}

const cache = new Map<string, string>();

/** 引用部分的签名：卡片标题、状态变了要重新画（缓存按它分开） */
function refsSig(src: string, refs?: Refs): string {
  if (!refs) return '';
  const out = [refs.path];
  for (const m of src.matchAll(CARD_REF)) {
    const board = m[1] ? `${m[1]}/${m[2]}` : refs.path;
    const id = Number(m[3]);
    const info = refs.find(board, id);
    out.push(`${board}#${id}:${info === undefined ? '?' : info === null ? 'x' : `${info.title}\u0001${info.status}`}`);
  }
  return out.join('\u0000');
}

export function markdown(src: string, opts: MdOpts = {}): string {
  const web = repoWeb(opts.repo);
  const key = `${refsSig(src || '', opts.refs)}\u0000${web?.home ?? ''}\u0002${src || ''}`;
  const hit = cache.get(key);
  if (hit != null) return hit;
  const raw = marked.parse(src || '', { async: false }) as string;
  const clean = purify.sanitize(raw, SANITIZE) as string;
  const out = decorate(clean, opts.refs, web);
  if (cache.size > 400) cache.clear();
  cache.set(key, out);
  return out;
}

/** 任务列表换成自绘的勾选框；gh#123 高亮（设了仓库地址时还变成 issue 链接）、提交号变成链接；
   卡片编号变成链接；链接在新窗口打开 */
function decorate(html: string, refs?: Refs, repo: RepoWeb | null = null): string {
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
  // 附件：图片延迟加载，点击放大（bits.tsx 的 Markdown）；文本类文件点开在界面里查看（data-text），其余直接下载
  // （不开新窗口，桌面端新窗口没有登录状态）
  root.querySelectorAll('img').forEach(img => {
    img.setAttribute('loading', 'lazy');
    img.setAttribute('decoding', 'async');
  });
  root.querySelectorAll('a').forEach(a => {
    if ((a.getAttribute('href') || '').startsWith('/api/files/')) {
      a.setAttribute('download', a.textContent || '');
      a.classList.add('file-link');
      if (isTextFile(a.textContent || '')) a.setAttribute('data-text', '');
      return;
    }
    a.setAttribute('target', '_blank');
    a.setAttribute('rel', 'noopener noreferrer');
  });
  // 一段文字里依次找 gh#123（总是高亮，设了仓库地址时变成 issue 链接）、「提交：<sha>」（设了仓库地址时 sha
  // 变成提交页的链接）、卡片编号（变成打开卡片的链接），其余原样保留
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const hits: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const t = n as Text;
    const card = !!refs && CARD_REF_TEST.test(t.data);
    if (!(/gh#\d+/.test(t.data) || card || (repo && COMMIT.test(t.data)))) continue;
    if (!t.parentElement?.closest('code, pre, a')) hits.push(t);
  }
  for (const t of hits) {
    const src = t.data;
    const frag = document.createDocumentFragment();
    const cards = refs ? [...src.matchAll(CARD_REF)] : [];
    let ci = 0;
    let last = 0;
    while (last < src.length) {
      while (ci < cards.length && cards[ci].index! < last) ci++;
      const card = cards[ci];
      const rest = src.slice(last);
      const gh = /gh#(\d+)/.exec(rest);
      const commit = repo ? COMMIT.exec(rest) : null;
      // 下一个命中的位置（三种里最靠前的）
      let at = Infinity;
      let kind: 'gh' | 'commit' | 'card' = 'gh';
      let hit: RegExpExecArray | RegExpMatchArray | null = null;
      if (gh) {
        at = last + gh.index;
        hit = gh;
      }
      if (commit && commit.index < at - last) {
        at = last + commit.index;
        kind = 'commit';
        hit = commit;
      }
      if (card && card.index! < at) {
        at = card.index!;
        kind = 'card';
        hit = card;
      }
      if (!hit) break;
      if (at > last) frag.append(document.createTextNode(src.slice(last, at)));
      if (kind === 'commit') {
        // 「提交：<sha>」：前面的「提交：」原样留着，sha 变成提交页的链接
        const sha = hit[1];
        frag.append(document.createTextNode(src.slice(at, at + hit[0].length - sha.length)));
        const a = document.createElement('a');
        a.className = 'commit';
        a.href = repo!.commit(sha);
        a.target = '_blank';
        a.rel = 'noopener noreferrer';
        a.textContent = sha;
        frag.append(a);
      } else if (kind === 'gh') {
        const a = document.createElement(repo ? 'a' : 'span');
        a.className = 'gh';
        a.textContent = hit[0];
        if (repo) {
          (a as HTMLAnchorElement).href = repo.issue(Number(hit[1]));
          a.setAttribute('target', '_blank');
          a.setAttribute('rel', 'noopener noreferrer');
        }
        frag.append(a);
      } else {
        const board = hit[1] ? `${hit[1]}/${hit[2]}` : refs!.path;
        const id = Number(hit[3]);
        const info = refs!.find(board, id);
        if (info === null) {
          frag.append(document.createTextNode(hit[0])); // 卡片不存在：不是链接
        } else {
          const a = document.createElement('a');
          a.className = 'xref';
          a.href = `/${board}/${id}`;
          a.dataset.card = `${board}#${id}`;
          a.textContent = hit[0];
          // 悬停显示标题和状态（还查不到时只写白板路径）
          a.title = info ? `#${id} ${info.title} · ${info.status}` : `${board}#${id}`;
          a.dataset.tip = a.title;
          frag.append(a);
        }
      }
      last = at + hit[0].length;
    }
    if (last < src.length) frag.append(document.createTextNode(src.slice(last)));
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
