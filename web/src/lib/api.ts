/* ==========================================================================
   与 board-server 通信
   - 网页端：同源（/api/...）
   - 桌面端：Tauri 注入 window.__BOARD_API__（内嵌服务的地址）
   - 实时：SSE /api/stream，任何一处写入（其他窗口、CLI、超时释放）都会推送 { rev, events }
   ========================================================================== */
import type { Status, Tile, BoardEvent, BoardSnapshot, CardDetail, Cmd, ErrorCode, Label, Pos, Workspace } from './types';

declare global {
  interface Window {
    __BOARD_API__?: string;
    __BOARD_SHELL__?: string;
  }
}

export const API_BASE = (window.__BOARD_API__ ?? '').replace(/\/$/, '');

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
    res = await fetch(API_BASE + path, {
      method,
      headers: { 'content-type': 'application/json', 'x-board-client': CLIENT_ID },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError('NETWORK', '连不上 Board 服务，请确认 board-server 正在运行');
  }
  let data: unknown = null;
  try {
    data = await res.json();
  } catch {
    /* 非 JSON */
  }
  if (!res.ok) {
    const e = (data as { error?: { code: ErrorCode; message: string; detail?: Record<string, unknown> } } | null)?.error;
    throw new ApiError(e?.code ?? 'INTERNAL', e?.message ?? `请求失败（${res.status}）`, e?.detail);
  }
  return data as T;
}

const q = encodeURIComponent;

export const api = {
  workspace: () => req<Workspace>('GET', '/api/workspace'),
  board: (path: string) => req<BoardSnapshot>('GET', `/api/board?path=${q(path)}`),
  card: (path: string, id: number) => req<CardDetail>('GET', `/api/card?path=${q(path)}&id=${id}`),
  cmd: (board: string, cmd: Cmd) => req<CardDetail>('POST', '/api/cmd', { board, ...cmd }),

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
    es = new EventSource(API_BASE + '/api/stream');
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
