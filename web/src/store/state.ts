import { create } from 'zustand';
import type { LiveStatus } from '../lib/api';
import type { Prompt } from '../lib/prompts';
import { parseRoute } from '../lib/route';
import type { BoardSnapshot, CardDetail, Workspace } from '../lib/types';

export interface DataState {
  ws: Workspace | null;
  path: string;
  board: BoardSnapshot | null;
  detailId: number | null;
  detail: CardDetail | null;
  status: LiveStatus;
  rev: number;
  /** 服务端时钟 - 本地时钟 */
  offset: number;
  /** 切换白板时 +1：卡片整体重建，不做位置补间 */
  generation: number;
  error: string | null;
  /** 详情没取回来的原因（取回或换一张卡片时清空）；有值时详情里不再显示骨架 */
  detailError: string | null;
  /** 服务端版本（GET /api/info）：知道之前是 null。页面打包时的版本和它不一样时，界面提示刷新（lib/stale.ts） */
  serverVersion: string | null;
  /** 看这个窗口要不要提示刷新（computed，跟着 serverVersion 变） */
  stale: boolean;
  /** 正在写入的标签（乐观更新）：「白板路径#卡片」→ 标签 ID。正在加上的已经并进 board 里卡片的 labels，正在去掉的还留在里面；界面都画成 pending 态 */
  pending: Record<string, number[]>;
  /** 正在发送的评论（乐观更新）：「白板路径#卡片」→ 按发送顺序。动态底部画成「发送中」的气泡，服务端确认后换成正式评论 */
  sending: Record<string, Sending[]>;
  /** 还没确认的写操作涉及的卡片（「白板路径#卡片」）、评论（「白板路径@动态 ID」）和其他对象（Patch.keys，例如「host:mbp」）
      → 进行中的个数。界面据此画成「保存中」，并禁用对它们的操作（见 busy） */
  saving: Record<string, number>;
  /** 新建卡片的快捷提示词（#572，服务端一份）：打开浮层时取（loadPrompts），还没取到时是 null */
  prompts: Prompt[] | null;
}

export interface Sending {
  key: number;
  text: string;
  at: number;
}

const CURRENT = 'board.current';

export const savedPath = () => {
  try {
    return sessionStorage.getItem(CURRENT) || localStorage.getItem(CURRENT) || '';
  } catch {
    return '';
  }
};

export const savePath = (path: string) => {
  try {
    sessionStorage.setItem(CURRENT, path);
    localStorage.setItem(CURRENT, path);
  } catch {
    /* 忽略 */
  }
};

/** 启动时的地址：/<项目>/<白板>[/<ID>]，没有时用本地记住的白板 */
export const initialRoute = typeof location !== 'undefined' ? parseRoute(location.pathname, location.search) : { path: null, card: null, tab: null };

export const useData = create<DataState>(() => ({
  ws: null,
  path: initialRoute.path ?? savedPath(),
  board: null,
  detailId: null,
  detail: null,
  status: 'connecting',
  rev: 0,
  offset: 0,
  generation: 0,
  error: null,
  pending: {},
  sending: {},
  saving: {},
  prompts: null,
  detailError: null,
  serverVersion: null,
  stale: false,
}));

export const get = useData.getState;

export const serverNow = () => Date.now() + get().offset;
