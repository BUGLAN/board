/* ==========================================================================
   与 board-server 通信
   - 同源（/api/...）：网页端和桌面端都直接打开服务端的地址，登录用同一个会话 Cookie
   - 没登录或会话过期：任何请求返回 401，交给 onUnauthorized（界面切到登录页）
   - 实时：SSE /api/stream，任何一处写入（其他窗口、CLI、超时释放）都会推送 { rev, events }
   ========================================================================== */
import type { Attachment, Status, Tile, BoardEvent, BoardSnapshot, CardDetail, Cmd, ErrorCode, Label, Me, Pos, ServerInfo, TokenInfo, Workspace } from './types';

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

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method,
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json', 'x-board-client': CLIENT_ID },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError('NETWORK', '连不上 Board 服务，请确认 board-server 正在运行');
  }
  return parse<T>(res);
}

async function parse<T>(res: Response): Promise<T> {
  let data: unknown = null;
  try {
    data = await res.json();
  } catch {
    /* 非 JSON */
  }
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
    });
  } catch {
    throw new ApiError('NETWORK', '连不上 Board 服务，附件没有上传');
  }
  return parse<Attachment>(res);
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
  board: (path: string) => req<BoardSnapshot>('GET', `/api/board?path=${q(path)}`),
  card: (path: string, id: number) => req<CardDetail>('GET', `/api/card?path=${q(path)}&id=${id}`),
  /** 一块白板的动态：按 id 倒序分页（新的在前）。before 是上一页最后一条的 id，不传从最新一条开始；返回空数组 = 没有更早的了 */
  events: (path: string, before?: number | null, limit = 20) =>
    req<BoardEvent[]>('GET', `/api/events?path=${q(path)}&limit=${limit}${before != null ? `&before=${before}` : ''}`),
  cmd: (board: string, cmd: Cmd) => req<CardDetail>('POST', '/api/cmd', { board, ...cmd }),
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
  updateBoard: (path: string, patch: { name?: string; desc?: string }) => req('PATCH', `/api/boards/${path}`, patch),
  deleteBoard: (path: string) => req('DELETE', `/api/boards/${path}`),

  createLabel: (project: string, name: string, color?: string) =>
    req<Label>('POST', '/api/labels', { project, name, color }),
  updateLabel: (project: string, id: number, patch: { name?: string; color?: string }) =>
    req('PATCH', `/api/labels/${id}`, { project, ...patch }),
  deleteLabel: (project: string, id: number) => req('DELETE', `/api/labels/${q(project)}/${id}`),

  layout: (kind: 'project' | 'board', id: string, pos: Pos) => req('PUT', '/api/layout', { kind, id, x: pos.x, y: pos.y }),

  /* 工作流（board 是「项目/白板」路径） */
  createStatus: (board: string, name: string, color?: string) => req<Status>('POST', `/api/flow/${board}`, { name, color }),
  reorderStatuses: (board: string, keys: string[]) => req('PUT', `/api/flow/${board}`, { keys }),
  updateStatus: (board: string, key: string, patch: Partial<Pick<Status, 'name' | 'color' | 'claim' | 'lock' | 'final'>>) =>
    req('PATCH', `/api/flow/${board}/${q(key)}`, patch),
  deleteStatus: (board: string, key: string) => req('DELETE', `/api/flow/${board}/${q(key)}`),
  /** 空对象 = 复原默认布局 */
  flowLayout: (board: string, tiles: Record<string, Tile>) => req('PUT', `/api/flow/${board}/layout`, { tiles }),
  /** 连线：一次加上、去掉若干条，[起点, 终点] */
  updateEdges: (board: string, add: [string, string][], remove: [string, string][] = []) =>
    req('PUT', `/api/flow/${board}/edges`, { add, remove }),
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
