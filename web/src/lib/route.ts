/* ==========================================================================
   地址栏：卡片、白板都有自己的网址（PRD §10.1）
   - /<项目>/<白板>            打开这块白板
   - /<项目>/<白板>/<ID>       打开这块白板，并弹出这张卡片的详情（「未整理」里的条目也一样）
   - 其他 Tab 沿用 ?tab=，地址里带着当前白板
   打开、关闭卡片详情用 push（浏览器后退能关掉、再打开），在详情里切上一张 / 下一张用 replace。
   parseRoute 只认「项目/白板」都是标识、ID 是正整数的路径，认不出的路径当作没打开白板（用本地记住的）；
   这样服务端的单页应用回退（任何路径都返回 index.html）不会把奇怪的地址带进界面。
   ========================================================================== */
import type { Tab } from '../store/ui';

/** 地址里的白板路径必须是「项目/白板」，两段都是标识 */
const KEY = /^[a-z0-9][a-z0-9-]{0,31}$/;

export interface Route {
  /** 白板路径（项目/白板）；地址里没有时为空 */
  path: string | null;
  /** 打开的卡片编号；地址里没有时为空 */
  card: number | null;
  /** 其他 Tab（未整理、大盘、机器、设置…）；没有 ?tab= 时为空 */
  tab: Tab | null;
}

/** 从路径和查询串解析出路由（location.pathname、location.search） */
export function parseRoute(pathname: string, search = ''): Route {
  const parts = pathname.split('/').filter(Boolean);
  const tab = new URLSearchParams(search).get('tab');
  const known: Tab[] = ['board', 'projects', 'inbox', 'stats', 'hosts', 'settings'];
  const found = known.includes(tab as Tab) ? (tab as Tab) : null;
  if (parts.length < 2 || parts.length > 3) return { path: null, card: null, tab: found };
  const [project, board, raw] = parts;
  if (!KEY.test(project) || !KEY.test(board)) return { path: null, card: null, tab: found };
  let card: number | null = null;
  if (raw != null) {
    if (!/^[1-9]\d{0,17}$/.test(raw)) return { path: null, card: null, tab: found };
    card = Number(raw);
  }
  return { path: `${project}/${board}`, card, tab: found };
}

/** 由白板、卡片和 Tab 拼出地址（pathname + search）；路径里的标识已经校验过 */
export function routeUrl(path: string | null, card: number | null, tab: Tab = 'board'): string {
  const base = path ? `/${path}${card != null ? `/${card}` : ''}` : '/';
  // 白板 Tab 是默认的，不写 ?tab=；其他 Tab 带着白板，刷新后回到同一块白板
  return tab === 'board' ? base : `${base}?tab=${tab}`;
}

/* ------------------------------ 和地址栏同步 ------------------------------ */

export interface RouteState {
  path: string | null;
  card: number | null;
  tab: Tab;
}

/** 当前该显示哪块白板、哪张卡片、哪个 Tab：App 启动时注册（store 里的状态） */
let source: (() => RouteState) | null = null;
/** 正在按地址栏（后退、前进、刷新）改状态：这时不往地址栏写，免得把历史推乱 */
let applying = 0;

export function bindRoute(fn: () => RouteState) {
  source = fn;
}
/** 按地址栏改状态：期间不往地址栏写 */
export function applySilently(fn: () => void) {
  applying++;
  try {
    fn();
  } finally {
    applying--;
  }
}
/** 一张卡片（或一块白板）的完整网址：服务端地址 + 路由（复制链接、CLI 输出用） */
export function cardUrl(path: string, id?: number | null): string {
  return `${location.origin}${routeUrl(path, id ?? null)}`;
}

/** 把当前状态写进地址栏。open / close 详情、切换白板用 push（后退能回到原来），
    详情里切上一张 / 下一张、内部收起用 replace；和现在一样时什么也不做 */
export function syncUrl(mode: 'push' | 'replace' = 'replace') {
  if (!source || applying) return;
  const s = source();
  const url = routeUrl(s.path, s.card, s.tab);
  if (url === `${location.pathname}${location.search}`) return;
  history[mode === 'push' ? 'pushState' : 'replaceState'](null, '', url);
}
