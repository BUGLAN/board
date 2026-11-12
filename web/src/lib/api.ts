/* ==========================================================================
   与 board-server 通信
   - 同源（/api/...）：网页端和桌面端都直接打开服务端的地址，登录用同一个会话 Cookie
   - 没登录或会话过期：任何请求返回 401，交给 onUnauthorized（界面切到登录页）
   - 实时：SSE /api/stream，任何一处写入（其他窗口、CLI、超时释放）都会推送 { rev, events }
   ========================================================================== */
import type { Attachment, BoardEvent, BoardSnapshot, CardDetail, ClaimRule, Cmd, Dispatch, ErrorCode, HostInfo, Label, Me, Pos, SearchHit, ServerInfo, Stats, StatsScope, Status, Tile, TokenInfo, TrashCard, Workspace } from './types';

declare global {
  interface Window {
    __BOARD_SHELL__?: string;
    /** 'custom'：外壳没有系统标题栏，窗口按钮由界面画（Windows） */
    __BOARD_TITLEBAR__?: string;
    /** 桌面外壳的版本（外壳在加载页面前注入） */
    __BOARD_SHELL_VERSION__?: string;
    /** 桌面端自带的 board 命令行路径（外壳注入；没有时为空） */
    __BOARD_CLI__?: string;
  }
}

let unauthorized: (() => void) | null = null;
/** 请求返回 401（没登录、会话过期）时调用 */
export function onUnauthorized(fn: () => void) {
  unauthorized = fn;
}

/** 每个窗口一个标识：服务端把它记在事件里，窗口据此忽略自己触发的高亮 */
export const CLIENT_ID: string = (() => {
  try {
    return crypto.randomUUID();
  } catch {
    return Math.random().toString(36).slice(2) + Date.now().toString(36);
  }
})();

export class ApiError extends Error {
  code: ErrorCode;
  detail: Record<string, unknown> | null;
  constructor(code: ErrorCode, message: string, detail?: Record<string, unknown> | null) {
    super(message);
    this.code = code;
    this.detail = detail ?? null;
  }
}

/** 请求超时：网络半开（切网、合盖唤醒）时请求会一直挂着，写队列后面的操作跟着一直「保存中」 */
const TIMEOUT_MS = 15000;
/** 上传附件（≤ 10 MB）给得长一些 */
const UPLOAD_TIMEOUT_MS = 120000;

const timeout = (ms: number) => (typeof AbortSignal.timeout === 'function' ? AbortSignal.timeout(ms) : undefined);
const netError = (e: unknown, what: string) =>
  (e as Error)?.name === 'TimeoutError'
    ? new ApiError('NETWORK', `${what}超时：网络可能断了，稍后再试`)
    : new ApiError('NETWORK', '连不上 Board 服务，请确认 board-server 正在运行');

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method,
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json', 'x-board-client': CLIENT_ID },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: timeout(TIMEOUT_MS),
    });
    return await parse<T>(res);
  } catch (e) {
    throw e instanceof ApiError ? e : netError(e, '请求');
  }
}

async function parse<T>(res: Response): Promise<T> {
  // 空响应体算 null；有内容但不是 JSON（例如反向代理返回的网页）：成功的状态码也按错误处理，不把 null 当成数据
  const text = await res.text();
  let data: unknown = null;
  let json = !text.trim();
  if (!json) {
    try {
      data = JSON.parse(text);
      json = true;
    } catch {
      /* 非 JSON */
    }
  }
  if (res.ok && !json) throw new ApiError('INTERNAL', `服务端返回的不是 JSON（${res.status}），可能被代理或网关拦下了`);
  if (!res.ok) {
    const e = (data as { error?: { code: ErrorCode; message: string; detail?: Record<string, unknown> } } | null)?.error;
    if (res.status === 401) unauthorized?.();
    if (res.status === 413) throw new ApiError('USAGE', '文件太大，附件不能超过 10 MB');
    throw new ApiError(e?.code ?? 'INTERNAL', e?.message ?? `请求失败（${res.status}）`, e?.detail);
  }
  return data as T;
}

/** 上传附件：请求体就是文件内容。card 为 null：新建弹窗里先上传，新建卡片时按描述里的引用归到卡片 */
async function upload(board: string, card: number | null, file: File): Promise<Attachment> {
  const qs = `board=${encodeURIComponent(board)}&name=${encodeURIComponent(file.name)}${card ? `&card=${card}` : ''}`;
  let res: Response;
  try {
    res = await fetch(`/api/files?${qs}`, {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'content-type': file.type || 'application/octet-stream', 'x-board-client': CLIENT_ID },
      body: file,
      signal: timeout(UPLOAD_TIMEOUT_MS),
    });
    return await parse<Attachment>(res);
  } catch (e) {
    if (e instanceof ApiError) throw e;
    throw (e as Error)?.name === 'TimeoutError' ? netError(e, '上传') : new ApiError('NETWORK', '连不上 Board 服务，附件没有上传');
  }
}

const q = encodeURIComponent;

export const api = {
  me: () => req<Me>('GET', '/api/me'),
  /** 服务端版本和编译时的提交号 */
  info: () => req<ServerInfo>('GET', '/api/info'),
  logout: () => req('POST', '/api/auth/logout'),
  tokens: () => req<TokenInfo[]>('GET', '/api/tokens'),
  createToken: (name: string) => req<{ token: string; info: TokenInfo }>('POST', '/api/tokens', { name }),
  revokeToken: (id: number) => req('DELETE', `/api/tokens/${id}`),

  workspace: () => req<Workspace>('GET', '/api/workspace'),
  /** full：终点状态也带全部卡片（默认只带最近的几张，见 BoardSnapshot.omitted） */
  board: (path: string, full = false) => req<BoardSnapshot>('GET', `/api/board?path=${q(path)}${full ? '&full=1' : ''}`),
  card: (path: string, id: number) => req<CardDetail>('GET', `/api/card?path=${q(path)}&id=${id}`),
  /** 这块白板回收站里的卡片（按删除时间，最近删的在最后） */
  trash: (path: string) => req<TrashCard[]>('GET', `/api/trash?path=${q(path)}`),
  /** 一块白板的动态：按 id 倒序分页（新的在前）。before 是上一页最后一条的 id，不传从最新一条开始；返回空数组 = 没有更早的了 */
  events: (path: string, before?: number | null, limit = 20) =>
    req<BoardEvent[]>('GET', `/api/events?path=${q(path)}&limit=${limit}${before != null ? `&before=${before}` : ''}`),
  cmd: (board: string, cmd: Cmd) => req<CardDetail>('POST', '/api/cmd', { board, ...cmd }),
  /** 跨白板搜卡片（不需要白板）：文字、`#ID`、`项目/白板#ID`、`label:标签` */
  search: (query: string, limit = 20) => req<SearchHit[]>('GET', `/api/search?q=${q(query)}&limit=${limit}`),
  upload,

  createProject: (p: {
    name: string;
    key: string;
    desc?: string;
    color?: string;
    pos?: Pos;
    board: { name: string; key: string; desc?: string };
  }) => req<{ key: string }>('POST', '/api/projects', p),
  updateProject: (key: string, patch: { name?: string; desc?: string; color?: string }) =>
    req('PATCH', `/api/projects/${q(key)}`, patch),
  deleteProject: (key: string) => req('DELETE', `/api/projects/${q(key)}`),

  createBoard: (b: { project: string; name: string; key: string; desc?: string; pos?: Pos }) =>
    req<{ path: string }>('POST', '/api/boards', b),
  updateBoard: (path: string, patch: { name?: string; desc?: string; readme?: string; readyCheck?: string; readyCheckWin?: string }) =>
    req('PATCH', `/api/boards/${path}`, patch),
  /** 大盘：一个范围在 [from, to) 里的全部统计，按 tz 分桶 */
  stats: (q: { scope: StatsScope; path?: string; from: number; to: number; tz: string }) => {
    const p = new URLSearchParams({ scope: q.scope, from: String(q.from), to: String(q.to), tz: q.tz });
    if (q.path) p.set('path', q.path);
    return req<Stats>('GET', `/api/stats?${p}`);
  },
  deleteBoard: (path: string) => req('DELETE', `/api/boards/${path}`),

  createLabel: (project: string, name: string, color?: string, humanOnly?: boolean) =>
    req<Label>('POST', '/api/labels', { project, name, color, humanOnly }),
  updateLabel: (project: string, id: number, patch: { name?: string; color?: string; humanOnly?: boolean; priority?: boolean }) =>
    req('PATCH', `/api/labels/${id}`, { project, ...patch }),
  /** 标签顺序：ids 依次排在前面，没列出的按原顺序排在后面 */
  reorderLabels: (project: string, ids: number[]) => req('PUT', '/api/labels/order', { project, ids }),
  deleteLabel: (project: string, id: number) => req('DELETE', `/api/labels/${q(project)}/${id}`),

  layout: (kind: 'project' | 'board', id: string, pos: Pos) => req('PUT', '/api/layout', { kind, id, x: pos.x, y: pos.y }),

  /* 工作流（board 是「项目/白板」路径） */
  createStatus: (board: string, name: string, color?: string) => req<Status>('POST', `/api/flow/${board}`, { name, color }),
  reorderStatuses: (board: string, keys: string[]) => req('PUT', `/api/flow/${board}`, { keys }),
  updateStatus: (board: string, key: string, patch: Partial<Pick<Status, 'name' | 'color' | 'claim' | 'lock' | 'final' | 'error' | 'view' | 'order'>>) =>
    req('PATCH', `/api/flow/${board}/${q(key)}`, patch),
  deleteStatus: (board: string, key: string) => req('DELETE', `/api/flow/${board}/${q(key)}`),
  /** 空对象 = 复原默认布局 */
  flowLayout: (board: string, tiles: Record<string, Tile>) => req('PUT', `/api/flow/${board}/layout`, { tiles }),
  /** 白板设置：保存布局（不改当前布局）、应用已保存的布局、保留顺序开关（每个状态的排序依据用 updateStatus 的 order） */
  saveLayout: (board: string, tiles: Record<string, Tile>) => req('PUT', `/api/flow/${board}/layout/saved`, { tiles }),
  applyLayout: (board: string) => req('POST', `/api/flow/${board}/layout/apply`),
  keepSorted: (board: string, keepSorted: boolean) => req('PUT', `/api/flow/${board}/order`, { keepSorted }),
  /** 连线：一次加上、去掉若干条，[起点, 终点] */
  updateEdges: (board: string, add: [string, string][], remove: [string, string][] = []) =>
    req('PUT', `/api/flow/${board}/edges`, { add, remove }),

  /* 任务下发（board host） */
  /** 机器和排队、正在跑的派发（工作区里也有；机器页每 10 秒取一次，刷新心跳时间） */
  hosts: () => req<{ hosts: HostInfo[]; dispatches: Dispatch[] }>('GET', '/api/hosts'),
  /** 修改机器设置：同时跑几个、暂停接单、默认 agent、定时领取 */
  updateHost: (
    name: string,
    patch: {
      max?: number;
      perSession?: number;
      rules?: ClaimRule[];
      fallback?: string;
      paused?: boolean;
      agent?: string;
      cronOn?: boolean;
      cron?: string[];
      env?: { agent: string; env: string };
      /** 一个 agent 的模型、思考强度：不带的那一项不改，空字符串回到 agent 自己的默认 */
      run?: { agent: string; model?: string; effort?: string };
      dirs?: { board: string; path: string }[];
    },
  ) =>
    req<HostInfo>('PATCH', `/api/hosts/${q(name)}`, patch),
  /** 请那台机器的 board host 把仓库克隆到这一行的目录 */
  cloneHostDir: (name: string, board: string, url: string) => req<HostInfo>('POST', `/api/hosts/${q(name)}/dirs/clone`, { board, url }),
  removeHost: (name: string, revoke = false) => req('DELETE', `/api/hosts/${q(name)}${revoke ? '?revoke=1' : ''}`),
  /** 远程管理：重新检查、重启 board host、升级（now：不等在跑的派发结束）、排空、取消请求 */
  askHost: (name: string, kind: 'recheck' | 'restart' | 'upgrade' | 'drain' | 'cancel', now = false) =>
    req<HostInfo>('POST', `/api/hosts/${q(name)}/ask`, { kind, now }),
  /** 现在领一轮（效果与定时到点相同） */
  hostRound: (name: string) => req<HostInfo>('POST', `/api/hosts/${q(name)}/round`, {}),
  /** 这台机器最近 50 个结束的派发 */
  hostRecent: (name: string) => req<Dispatch[]>('GET', `/api/hosts/${q(name)}/recent`),
  dispatch: (board: string, card: number, host: string, agent?: string) => req<Dispatch>('POST', '/api/dispatches', { board, card, host, agent }),
  /** 取消排队中的派发，或停止正在跑的派发 */
  cancelDispatch: (id: number) => req<Dispatch>('POST', '/api/dispatches/cancel', { id }),
  /** 派发日志的最后 500 行 */
  dispatchLog: (id: number) => req<{ lines: string[] }>('GET', `/api/dispatches/${id}/log`),
};

export type LiveStatus = 'connecting' | 'live' | 'offline';

/** 订阅变更。返回取消订阅函数 */
export function connect(onMessage: (m: { rev: number; events: BoardEvent[] }) => void, onStatus: (s: LiveStatus) => void) {
  let es: EventSource | null = null;
  let closed = false;
  let retry = 0;
  let timer = 0;
  const open = () => {
    if (closed) return;
    onStatus('connecting');
    es = new EventSource('/api/stream');
    es.onopen = () => {
      retry = 0;
      onStatus('live');
    };
    es.onmessage = e => {
      try {
        onMessage(JSON.parse(e.data));
      } catch {
        /* 忽略 */
      }
    };
    es.onerror = () => {
      onStatus('offline');
      // 浏览器会自动重连；连不上时自己退避重连
      if (es?.readyState === EventSource.CLOSED) {
        es = null;
        clearTimeout(timer);
        timer = window.setTimeout(open, Math.min(8000, 500 * 2 ** retry++));
      }
    };
  };
  open();
  return () => {
    closed = true;
    clearTimeout(timer);
    es?.close();
  };
}
