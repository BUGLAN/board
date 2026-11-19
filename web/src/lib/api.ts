/* ==========================================================================
   与 board-server 通信
   - 同源（/api/...）：网页端和桌面端都直接打开服务端的地址，登录用同一个会话 Cookie
   - 没登录或会话过期：任何请求返回 401，交给 onUnauthorized（界面切到登录页）
   - 实时：SSE /api/stream，任何一处写入（其他窗口、CLI、超时释放）都会推送 { rev, events }
   ========================================================================== */
import { setAgentMeta } from './agentMeta';
import type { Milestones } from './milestones';
import { normDispatches, normHosts, normMcp, normProviders, normSession, normSkills, normWorkspace } from './normalize';
import type { Attachment, DirList, BoardEvent, BoardSnapshot, CardCron, CardCronInput, CardDetail, ClaimRule, Cmd, Dispatch, ErrorCode, GitCred, HostInfo, HostWorkspace, Label, McpServer, Me, Pos, Provider, ProviderView, SearchHit, ServerInfo, SessionView, SkillInfo, Stats, StatsScope, Status, Tile, TokenInfo, TrashCard, Workspace, ProviderEndpoint, SkillImportPick, Sched, RunMode, HostCopy } from './types';

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
  /** copyFrom：添加机器时选了「上线后照 <机器> 配」，用这个令牌的机器第一次上线时照它配 */
  /** host：添加机器时绑定的机器名（机器令牌，只有它能登记这台新机器，#507）；「新建访问令牌」不填 */
  createToken: (name: string, opts: { copyFrom?: string; host?: string } = {}) =>
    req<{ token: string; info: TokenInfo }>('POST', '/api/tokens', { name, ...(opts.copyFrom ? { copyFrom: opts.copyFrom } : {}), ...(opts.host ? { host: opts.host } : {}) }),
  revokeToken: (id: number) => req('DELETE', `/api/tokens/${id}`),

  // 机器、派发、共用配置这些接口的数据进门时补齐缺的数组（lib/normalize.ts）：界面和服务端版本不一致时不整页崩
  workspace: () => req<Workspace>('GET', '/api/workspace').then(normWorkspace),
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
    /** 项目目录（#488）：留空用项目标识 */
    dir?: string;
    board: { name: string; key: string; desc?: string };
  }) => req<{ key: string }>('POST', '/api/projects', p),
  updateProject: (
    key: string,
    patch: { name?: string; desc?: string; color?: string; hiddenLabels?: string[]; repo?: string; branch?: string; dir?: string },
  ) =>
    req('PATCH', `/api/projects/${q(key)}`, patch),
  deleteProject: (key: string) => req('DELETE', `/api/projects/${q(key)}`),

  createBoard: (b: { project: string; name: string; key: string; desc?: string; pos?: Pos }) =>
    req<{ path: string }>('POST', '/api/boards', b),
  updateBoard: (path: string, patch: { name?: string; desc?: string; readme?: string; repo?: string; branch?: string; auto?: boolean }) =>
    req('PATCH', `/api/boards/${path}`, patch),
  /** 定时建卡（PRD §4）：只有人能改 */
  createCardCron: (path: string, c: CardCronInput) => req<CardCron>('POST', `/api/card-crons/${path}`, c),
  updateCardCron: (path: string, id: number, c: CardCronInput) => req<CardCron>('PATCH', `/api/card-crons/${path}/${id}`, c),
  deleteCardCron: (path: string, id: number) => req('DELETE', `/api/card-crons/${path}/${id}`),
  /** 现在建一张：同样受跳过规则约束，返回这条（ranCard / ranSkip 是这一次的结果） */
  runCardCron: (path: string, id: number) => req<CardCron>('POST', `/api/card-crons/${path}/${id}/run`),
  /** 里程碑（PRD §10.15）：项目里的版本、带版本标签的卡片、未发版和没有版本的卡片 */
  milestones: (project: string) => req<Milestones>('GET', `/api/milestones?project=${encodeURIComponent(project)}`),
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
  hosts: () => req<{ hosts: HostInfo[]; dispatches: Dispatch[] }>('GET', '/api/hosts').then(normHosts),
  /** 修改机器设置：最多几个会话、暂停接单、默认 agent、自动接单和接单时段 */
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
      /** 一个 agent 的模型怎么配：不改 / 指定（供应商、模型、强度、接口；模型空 = 第一个，强度空 = 不写，接口空 = 第一个对得上的） */
      run?: { agent: string; mode: RunMode; provider?: string; model?: string; effort?: string; protocol?: string };
      /** skills、MCP 用服务器的（true）还是不改（false） */
      syncSkills?: boolean;
      syncMcp?: boolean;
      /** Git 凭据用服务器的（true）还是不改（false） */
      syncGit?: boolean;
      /** 恢复一个连续异常退出、暂停了接单的 agent（写它的标识） */
      resume?: string;
      /** 参与自动接单 */
      auto?: boolean;
      /** 内置 agent 自动更新（默认关） */
      agentAuto?: boolean;
      /** 接单时段（整个替换）：一天里的几段 [起, 止)，分钟 */
      windows?: [number, number][];
      /** 工作空间（整个替换）：每个项目的目录是 `<工作空间>/<项目>`；空路径表示不配 */
      workspace?: HostWorkspace;
    },
  ) =>
    req<HostInfo>('PATCH', `/api/hosts/${q(name)}`, patch),
  /** 请那台机器的 board host 把仓库克隆到这块白板所在项目的目录（`<工作空间>/<项目>`） */
  cloneHostDir: (name: string, board: string, url: string) => req<HostInfo>('POST', `/api/hosts/${q(name)}/dirs/clone`, { board, url }),
  removeHost: (name: string, revoke = false) => req('DELETE', `/api/hosts/${q(name)}${revoke ? '?revoke=1' : ''}`),
  /** 远程管理：重新检查、重启 board host、升级（now：不等在跑的派发结束）、取消请求 */
  askHost: (name: string, kind: 'recheck' | 'restart' | 'upgrade' | 'cancel', now = false) =>
    req<HostInfo>('POST', `/api/hosts/${q(name)}/ask`, { kind, now }),
  /** 安装或更新这台机器上的一个内置 agent：board host 按实际安装方式执行，不占会话位 */
  askAgent: (name: string, kind: 'agent-install' | 'agent-update', agent: string, to?: string) =>
    req<HostInfo>('POST', `/api/hosts/${q(name)}/ask`, { kind, agent, to: to || null }),
  /** skills 共用库（PRD §10.16「共用配置」）：列出、上传（同名换成新版本）、删除 */
  skills: () => req<SkillInfo[]>('GET', '/api/skills').then(normSkills),
  removeSkill: (name: string) => req('DELETE', `/api/skills/${q(name)}`),
  /** 从这台机器导入 skills：board host 把选中的目录传进库 */
  importSkills: (host: string, picks: SkillImportPick[]) => req<HostInfo>('POST', `/api/hosts/${q(host)}/skills/import`, { picks }),
  /** MCP 服务器共用配置：列表（密钥只给末尾 4 位）、新增或修改（密钥留空保留原值）、删除 */
  mcp: () => req<McpServer[]>('GET', '/api/mcp').then(normMcp),
  putMcp: (s: McpServer) => req<McpServer>('POST', '/api/mcp', s),
  removeMcp: (name: string) => req('DELETE', `/api/mcp/${q(name)}`),
  /** 模型供应商共用配置：列表（API key 只给末尾 4 位）、新增或修改（留空保留原值）、删除、每个 agent 用哪个 */
  /** 顺带换上内置 agent 的说明（agentMeta.ts） */
  providers: () =>
    req<ProviderView>('GET', '/api/providers')
      .then(normProviders)
      .then(v => {
        setAgentMeta(v.meta);
        return v;
      }),
  putProvider: (p: Provider) => req<Provider>('POST', '/api/providers', p),
  removeProvider: (name: string) => req('DELETE', `/api/providers/${q(name)}`),
  /** 获取模型：服务端用这些地址和 API key 问供应商有哪些模型；key 留空时用 name 对应的供应商存着的 */
  providerModels: (body: { name?: string; endpoints: ProviderEndpoint[]; apiKey?: string | null }) => req<{ models: string[]; from: string }>('POST', '/api/providers/models', body),
  /** 同步配置写入失败后「重试」：重新下发同步请求 */
  /** Git 凭据（共用配置）：令牌只有末尾 4 位；没设时 null */
  git: () => req<{ cred: GitCred | null }>('GET', '/api/git').then(r => ({ cred: r?.cred ?? null })),
  /** 设置或修改：令牌留空保留原来的 */
  putGit: (c: { host: string; username: string; token: string; commitName: string; commitEmail: string }) => req<GitCred>('POST', '/api/git', c),
  removeGit: () => req('DELETE', '/api/git'),
  syncHost: (name: string) => req<HostInfo>('POST', `/api/hosts/${q(name)}/sync`),
  /** 照另一台机器配：dry 只列出要变哪些，确认后再不带 dry 调一次写进去 */
  copyHost: (name: string, from: string, dry: boolean) => req<HostCopy>('POST', `/api/hosts/${q(name)}/copy`, { from, dry }),
  /** 列那台机器上一个文件夹的子文件夹（服务端转给它的 board host，最多等 20 秒）；空路径是最上面一层 */
  hostLs: (name: string, path: string) => req<DirList>('POST', `/api/hosts/${q(name)}/ls`, { path }),
  /** 自检：整台机器，或只查一个 agent（PRD §10.12「自检」） */
  selfTest: (name: string, agent?: string) => req<HostInfo>('POST', `/api/hosts/${q(name)}/selftest`, agent ? { agent } : {}),
  /** 全部暂停 / 全部恢复：所有机器一起设成暂停接单（排队的派发照样排队） */
  pauseAllHosts: (paused: boolean) => req<HostInfo[]>('POST', '/api/hosts/pause-all', { paused }),
  /** 全部停止：停止所有正在跑的派发，同时全部暂停 */
  stopAllHosts: () => req<HostInfo[]>('POST', '/api/hosts/stop-all'),
  /** 改全局的领取规则（整个列表替换）和兜底（空字符串 = 不领）：所有机器共用。
      另带 `agent` + `add` / `remove`（勾上 / 取消的机器 id）或 `all`（回到所有装了它的机器都参与）：改这个 agent「由这些机器做」 */
  updateSched: (patch: {
    rules?: Pick<ClaimRule, 'op' | 'labels' | 'agent'>[];
    fallback?: string;
    autoUpgrade?: boolean;
    maxRun?: number;
    idleRun?: number;
    agent?: string;
    add?: number[];
    remove?: number[];
    all?: boolean;
  }) => req<Sched>('PATCH', '/api/sched', patch),
  /** 这台机器最近 50 个结束的派发 */
  hostRecent: (name: string) => req<Dispatch[]>('GET', `/api/hosts/${q(name)}/recent`).then(normDispatches),
  /** host 是空的 = 任意机器：服务端在能接的机器里挑空位最多的一台 */
  dispatch: (board: string, card: number, host: string, agent?: string) => req<Dispatch>('POST', '/api/dispatches', { board, card, host, agent }),
  /** 改派一条还在排队的派发：换到 host 这台机器，host 是空的 = 交给任意机器 */
  moveDispatch: (id: number, host: string, agent?: string) => req<Dispatch>('POST', '/api/dispatches/move', { id, host, agent }),
  /** 取消排队中的派发，或停止正在跑的派发 */
  cancelDispatch: (id: number) => req<Dispatch>('POST', '/api/dispatches/cancel', { id }),
  /** 一张卡片的全部派发，新的在前（卡片详情「派发记录」） */
  cardDispatches: (board: string, card: number) =>
    req<Dispatch[]>('GET', `/api/dispatches?board=${encodeURIComponent(board)}&card=${card}`).then(normDispatches),
  /** 一个会话的全部派发和各自的输出：id 是会话里任一次派发；since 是上次取到的 runs，只取之后新增、改过的事件 */
  session: (id: number, since = 0) => req<SessionView>('GET', `/api/sessions/${id}${since > 0 ? `?since=${since}` : ''}`).then(normSession),
};

export type LiveStatus = 'connecting' | 'live' | 'offline';

/** 推送：rev 变了是工作区的变更；runs 变了是有新的结构化事件，ran 是有新事件的派发（会话「过程」据此马上取） */
interface PushMessage {
  rev: number;
  events: BoardEvent[];
  runs?: number;
  ran?: number[];
}

/** 订阅变更。返回取消订阅函数。
    onReopen：推送断开后重新连上时调用（第一次连上不算）：服务端可能刚升级过，界面据此查一次版本（default/main#368） */
export function connect(onMessage: (m: PushMessage) => void, onStatus: (s: LiveStatus) => void, onReopen?: () => void) {
  let es: EventSource | null = null;
  let closed = false;
  let retry = 0;
  let timer = 0;
  /** 连上过没有：断开后重连才算「重新连上」 */
  let opened = false;
  const open = () => {
    if (closed) return;
    onStatus('connecting');
    es = new EventSource('/api/stream');
    es.onopen = () => {
      retry = 0;
      onStatus('live');
      if (opened) onReopen?.();
      opened = true;
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
