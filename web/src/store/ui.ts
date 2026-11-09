/* ==========================================================================
   界面状态：活动栏 Tab、侧栏、筛选、各种弹窗与浮层、toast、高亮
   ========================================================================== */
import { create } from 'zustand';
import type { ApiError } from '../lib/api';
import type { SortSpec } from '../lib/sort';
import type { ColumnKey } from '../lib/types';
import { pref } from '../lib/util';

export type Tab = 'board' | 'projects' | 'inbox' | 'hosts' | 'settings';

export interface Toast {
  id: number;
  type: 'error' | 'success' | 'info';
  title: string;
  text?: string;
  code?: string;
  duration: number;
  action?: { label: string; fn: () => void };
}

export interface ConfirmReq {
  title: string;
  text: string;
  ok?: string;
  icon?: 'trash' | 'tag' | 'reset' | 'logout' | 'key';
  resolve: (v: boolean) => void;
}

export interface FormField {
  name: string;
  label: string;
  hint?: string;
  type?: 'text' | 'key' | 'color' | 'select';
  value?: string;
  placeholder?: string;
  required?: boolean;
  readonly?: boolean;
  /** 标识自动跟随这个字段，直到用户手动修改 */
  from?: string;
  auto?: (src: string, v: Record<string, string>) => string;
  validate?: (value: string, v: Record<string, string>) => string | null;
  options?: { value: string; label: string }[];
}

export interface FormCfg {
  title: string;
  subtitle?: string;
  submit?: string;
  fields: FormField[];
  preview?: 'project' | 'board' | 'edit-project' | 'edit-board';
  previewData?: Record<string, string>;
  onSubmit: (v: Record<string, string>) => Promise<void>;
}

export interface PickerReq {
  rect: DOMRect;
  project: string;
  selected: number[];
  onToggle: (id: number, on: boolean) => void | Promise<void>;
}

export interface CtxReq {
  /** prompts：白板页头的「提示词」菜单，id 是白板路径；sort：状态块的排序菜单，id 是状态标识；
      move：卡片详情底部的「移动到」，id 是卡片编号 */
  kind: 'project' | 'board' | 'prompts' | 'sort' | 'move';
  id: string;
  x: number;
  y: number;
  /** 从按钮弹出时右对齐 */
  anchor?: DOMRect;
}

export interface Flash {
  key: number;
  color: string;
  label: string;
}

export interface DragUI {
  id: number;
  fromCol: ColumnKey;
  /** 预览中卡片所在的位置（不合法时回到原位） */
  col: ColumnKey;
  index: number;
  /** 指针下的列 */
  over: ColumnKey | null;
  valid: boolean;
  veto: { title: string; message: string; code: string } | null;
}

interface UIState {
  tab: Tab;
  side: boolean;
  filter: string;
  /** 侧栏选中的标签（ID）：卡片带其中任一个就算命中；与 filter 同时生效 */
  labelSel: number[];
  creator: { open: boolean; column: 'todo' | 'inbox'; seq: number };
  palette: { open: boolean; prefill: string };
  switcher: boolean;
  confirm: ConfirmReq | null;
  form: FormCfg | null;
  labelsOf: string | null;
  picker: PickerReq | null;
  ctx: CtxReq | null;
  toasts: Toast[];
  flashes: Record<number, Flash>;
  drag: DragUI | null;
  reveal: { id: number; key: number } | null;
  reject: { col: ColumnKey; key: number } | null;
  /** 放大查看的图片（附件） */
  image: { src: string; alt: string } | null;
  /** 状态块的一次性排序（lib/sort.ts）：sortKey(白板路径, 状态) → 排序依据。不记住，刷新后回到优先级顺序 */
  sorts: Record<string, SortSpec>;
  /** 打开的机器详情：机器名；'new' 是「添加机器」 */
  host: string | null;
  /** 卡片详情里的「派发」对话框：卡片编号 */
  dispatchOf: number | null;
}

const TABS: Tab[] = ['board', 'projects', 'inbox', 'hosts', 'settings'];
const initialTab = (): Tab => {
  const q = new URLSearchParams(location.search).get('tab') as Tab | null;
  if (q && TABS.includes(q)) return q;
  const p = pref.get<Tab>('tab', 'board');
  return TABS.includes(p) ? p : 'board';
};

/** 侧栏展开 / 收起按 Tab 各记一份：切到别的 Tab 再切回来，恢复上次的选择 */
const sides = (): Partial<Record<Tab, boolean>> => pref.get('sides', {});
const sideOf = (tab: Tab) => sides()[tab] ?? true;

export const useUI = create<UIState>(() => ({
  tab: initialTab(),
  side: sideOf(initialTab()),
  filter: '',
  labelSel: [],
  creator: { open: false, column: 'todo', seq: 0 },
  palette: { open: false, prefill: '' },
  switcher: false,
  confirm: null,
  form: null,
  labelsOf: null,
  picker: null,
  ctx: null,
  toasts: [],
  flashes: {},
  drag: null,
  reveal: null,
  reject: null,
  image: null,
  sorts: {},
  host: null,
  dispatchOf: null,
}));

const set = useUI.setState;
const get = useUI.getState;

/* ------------------------------ Tab / 侧栏 ------------------------------ */

export function setTab(tab: Tab, { toggle = false } = {}) {
  const prev = get().tab;
  if (toggle && prev === tab && !window.matchMedia('(max-width: 760px)').matches) {
    setSide(!get().side);
    return;
  }
  if (prev === tab) return;
  const side = sideOf(tab);
  const changed = side !== get().side;
  set({ tab, side });
  pref.set('tab', tab);
  if (changed) notifyResize();
}

/** 展开 / 收起当前 Tab 的侧栏，并记住 */
export function setSide(open: boolean) {
  if (open === get().side) return;
  set({ side: open });
  pref.set('sides', { ...sides(), [get().tab]: open });
  notifyResize();
}

const notifyResize = () => setTimeout(() => window.dispatchEvent(new Event('resize')), 440);

export const setFilter = (filter: string) => set({ filter });
export const toggleLabelSel = (id: number) =>
  set(s => ({ labelSel: s.labelSel.includes(id) ? s.labelSel.filter(x => x !== id) : [...s.labelSel, id] }));
export const clearLabelSel = () => set(s => (s.labelSel.length ? { labelSel: [] } : s));

/* ------------------------------ 弹窗 ------------------------------ */

export const openCreator = (column: 'todo' | 'inbox' = get().tab === 'inbox' ? 'inbox' : 'todo') =>
  set(s => ({ creator: { open: true, column, seq: s.creator.seq + 1 } }));
export const closeCreator = () => set(s => ({ creator: { ...s.creator, open: false } }));

export const openImage = (src: string, alt = '') => set({ image: { src, alt } });
export const closeImage = () => set({ image: null });
export const openPalette = (prefill = '') => set({ palette: { open: true, prefill } });
export const closePalette = () => set(s => ({ palette: { ...s.palette, open: false } }));

export const openSwitcher = () => set({ switcher: true });
export const closeSwitcher = () => set({ switcher: false });

export function confirm(req: Omit<ConfirmReq, 'resolve'>): Promise<boolean> {
  return new Promise(resolve => set({ confirm: { ...req, resolve } }));
}
export function settleConfirm(v: boolean) {
  const c = get().confirm;
  set({ confirm: null });
  c?.resolve(v);
}

export const openForm = (form: FormCfg) => set({ form });
export const closeForm = () => set({ form: null });

export const manageLabels = (project: string) => set({ labelsOf: project, picker: null });
export const closeLabels = () => set({ labelsOf: null });

export const openPicker = (req: PickerReq) => set({ picker: req });
export const updatePicker = (selected: number[]) => set(s => (s.picker ? { picker: { ...s.picker, selected } } : {}));
export const closePicker = () => set({ picker: null });

export const openHost = (name: string | null) => set({ host: name });
export const openDispatch = (card: number | null) => set({ dispatchOf: card });

export const openCtx = (req: CtxReq) => set({ ctx: req });
export const closeCtx = () => set({ ctx: null });

/** 设置 / 取消（null）一块的排序 */
export const setSort = (key: string, spec: SortSpec | null) =>
  set(s => {
    const sorts = { ...s.sorts };
    if (spec) sorts[key] = spec;
    else delete sorts[key];
    return { sorts };
  });


/** 有弹窗或浮层打开时，全局快捷键不生效 */
export function overlayOpen() {
  const s = get();
  return !!(s.confirm || s.form || s.labelsOf || s.picker || s.ctx || s.palette.open || s.switcher || s.creator.open || s.image || s.host || s.dispatchOf != null);
}

/* ------------------------------ Toast ------------------------------ */

let toastSeq = 0;
export function toast(t: Omit<Toast, 'id' | 'duration'> & { duration?: number }) {
  const id = ++toastSeq;
  const duration = t.duration ?? (t.type === 'error' ? 5200 : 3200);
  set(s => ({ toasts: [...s.toasts, { ...t, id, duration }].slice(-4) }));
  return () => dismissToast(id);
}
export const dismissToast = (id: number) => set(s => ({ toasts: s.toasts.filter(t => t.id !== id) }));

const ERROR_TITLES: Record<string, string> = {
  FORBIDDEN_COLUMN: '不能放到这个状态',
  NOT_HOLDER: '这张卡片正被 agent 持有',
  EMPTY_TITLE: '标题不能为空',
  NOT_FOUND: '卡片不存在',
  BOARD_NOT_FOUND: '白板不存在',
  USAGE: '操作无效',
  NETWORK: '连接失败',
  NOT_LOGGED_IN: '需要重新登录',
  ALREADY_DISPATCHED: '已经派发过了',
  NO_SUCH_HOST: '机器不存在',
  AGENT_NOT_ON_HOST: '机器上没有这种 agent',
  BOARD_NOT_ON_HOST: '机器没有这块白板的目录',
  NOT_IN_TODO: '卡片不在可领取状态',
};
export function toastError(err: unknown) {
  const e = err as ApiError;
  return toast({ type: 'error', title: ERROR_TITLES[e.code] || '操作被拒绝', code: e.code, text: e.message });
}

/* ------------------------------ 高亮 ------------------------------ */

let flashSeq = 0;
export function flash(cardId: number, color: string, label: string) {
  const key = ++flashSeq;
  set(s => ({ flashes: { ...s.flashes, [cardId]: { key, color, label } } }));
  setTimeout(() => {
    set(s => {
      if (s.flashes[cardId]?.key !== key) return {};
      const next = { ...s.flashes };
      delete next[cardId];
      return { flashes: next };
    });
  }, 2600);
}

let revealSeq = 0;
export const reveal = (id: number) => set({ reveal: { id, key: ++revealSeq } });
export const rejectColumn = (col: ColumnKey) => set({ reject: { col, key: ++revealSeq } });

export const setDrag = (drag: DragUI | null) => set({ drag });
