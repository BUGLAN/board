/* ==========================================================================
   界面状态：活动栏 Tab、侧栏、筛选、各种弹窗与浮层、toast、高亮
   ========================================================================== */
import { create } from 'zustand';
import type { ApiError } from '../lib/api';
import type { ColumnKey, Label } from '../lib/types';
import { pref } from '../lib/util';

/** 一张可以放大查看的图片 */
export interface ImageRef {
  src: string;
  alt: string;
}
export type Tab = 'board' | 'projects' | 'inbox' | 'stats' | 'hosts' | 'settings';

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
  /** 说明文字：换行按段落显示（多行用 \n 分隔），长的内容（目录列表之类）不撑破弹窗、在框里滚动 */
  text: string;
  /** 正文按 Markdown 显示（「发现新版本」的更新说明来自 CHANGELOG）；默认按纯文本，换行照原样显示 */
  markdown?: boolean;
  ok?: string;
  /** 取消按钮的文字，默认「取消」 */
  cancel?: string;
  icon?: ConfirmIcon;
  /** warn：要人确认时把风险说清楚（黄色图标，而不是默认的红色危险图标） */
  tone?: 'danger' | 'warn';
  /** 确认框里的一个勾选项（移除机器时「同时吊销它用的令牌」）：勾没勾由 confirmCheck 返回；why 有值时不能勾，写明原因 */
  check?: { label: string; why?: string; box: { checked: boolean } };
  resolve: (v: boolean) => void;
}

export interface FormField {
  name: string;
  label: string;
  hint?: string;
  /** markdown：多行编辑框，能粘贴、拖入图片和文件（上传到 board 这块白板） */
  type?: 'text' | 'key' | 'color' | 'select' | 'markdown';
  /** markdown 字段的附件传到哪块白板 */
  board?: string;
  value?: string;
  placeholder?: string;
  /** 输入框最多多少字；默认 60，标识（key）固定 32 */
  maxLength?: number;
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
  /** 打开时定位到这个字段（默认第一个可编辑的） */
  focus?: string;
  onSubmit: (v: Record<string, string>) => Promise<void>;
}

export interface PickerReq {
  rect: DOMRect;
  project: string;
  selected: number[];
  onToggle: (id: number, on: boolean) => void | Promise<void>;
  /** 给了就只在这些标签里选（不取 project 的标签），也不能新建、管理：例如机器的领取规则，标签来自几个项目、按名称对应 */
  labels?: Label[];
}

/** 确认框的图标：默认的危险动作图标（trash 等）；update 是「发现新版本」，主按钮用品牌色、不是危险动作 */
export type ConfirmIcon = 'trash' | 'tag' | 'reset' | 'logout' | 'key' | 'unlock' | 'alert' | 'host' | 'update';

export interface CtxReq {
  /** prompts：白板页头的「提示词」菜单，id 是白板路径；
      move：卡片详情底部的「移动到」，id 是卡片编号；select：选择框的选项 */
  kind: 'project' | 'board' | 'prompts' | 'move' | 'select';
  id: string;
  x: number;
  y: number;
  /** 从按钮弹出时右对齐（select 左对齐，至少和选择框一样宽） */
  anchor?: DOMRect;
  /** select：选择框（components/bits Select）点开的选项列表，id 是那个选择框的标识 */
  items?: { value: string; label: string; disabled?: boolean }[];
  value?: string;
  pick?: (value: string) => void;
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

export type HostsView = 'racks' | 'compare' | 'shared';
function initialHostsView(): HostsView {
  const v = pref.get<string>('hosts.view', 'racks');
  return v === 'compare' || v === 'shared' ? v : 'racks';
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
  /** 定位到卡片；pulse 为 false 时只滚过去，不放光环（刚在本窗口新建的卡片） */
  reveal: { id: number; key: number; pulse: boolean } | null;
  reject: { col: ColumnKey; key: number } | null;
  /** 放大查看的图片（附件）：同一张卡片 / 同一条动态里的全部图片，index 是正在看的那张 */
  image: { list: ImageRef[]; index: number } | null;
  /** 打开的白板设置（居中弹窗）：白板路径；focus 是要定位到的状态（点状态块标题上的排序标记打开时） */
  boardSettings: { path: string; focus?: string } | null;
  /** 打开的机器详情：机器名；'new' 是「添加机器」 */
  host: string | null;
  /** 机器 Tab 主区：总览、对比、共用配置（选过的记在本机） */
  hostsView: HostsView;
  /** 回收站弹窗：哪块白板（路径）的回收站；null 是没打开 */
  trash: string | null;
  /** 卡片详情里的「派发」对话框：卡片编号 */
  dispatchOf: number | null;
  /** 打开的「会话」弹窗：会话里任一次派发的 ID；card 是要定位到的那张卡 */
  session: { id: number; card?: number } | null;
  /** 手机白板页的底部抽屉：筛选框、标签、正在工作、最近动态（桌面端在侧栏里） */
  boardSheet: boolean;
}

/** 活动栏从上到下的顺序，也是数字快捷键 ⌘/Ctrl 1~6 的顺序（设置固定在活动栏底部） */
export const TAB_ORDER: Tab[] = ['board', 'inbox', 'stats', 'projects', 'hosts', 'settings'];
const initialTab = (): Tab => {
  const q = new URLSearchParams(location.search).get('tab') as Tab | null;
  if (q && TAB_ORDER.includes(q)) return q;
  const p = pref.get<Tab>('tab', 'board');
  return TAB_ORDER.includes(p) ? p : 'board';
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
  boardSettings: null,
  host: null,
  hostsView: initialHostsView(),
  trash: null,
  dispatchOf: null,
  session: null,
  boardSheet: false,
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
export const setBoardSheet = (open: boolean) => set({ boardSheet: open });
export const clearLabelSel = () => set(s => (s.labelSel.length ? { labelSel: [] } : s));

/* ------------------------------ 弹窗 ------------------------------ */

export const openCreator = (column: 'todo' | 'inbox' = get().tab === 'inbox' ? 'inbox' : 'todo') =>
  set(s => ({ creator: { open: true, column, seq: s.creator.seq + 1 } }));
export const closeCreator = () => set(s => ({ creator: { ...s.creator, open: false } }));

/** 放大查看图片。list 是可以左右切换的同组图片（附件列表、同一段 Markdown 里的图片），不给就只看这一张 */
export const openImage = (src: string, alt = '', list?: ImageRef[]) => {
  const all = list?.length ? list : [{ src, alt }];
  const i = all.findIndex(x => x.src === src);
  set({ image: i < 0 ? { list: [{ src, alt }], index: 0 } : { list: all, index: i } });
};
/** 上一张 / 下一张：到第一张、最后一张时停住（按钮禁用），不循环 */
export const stepImage = (d: -1 | 1) =>
  set(s => {
    if (!s.image) return {};
    const index = s.image.index + d;
    return index < 0 || index >= s.image.list.length ? {} : { image: { ...s.image, index } };
  });
export const closeImage = () => set({ image: null });
export const openPalette = (prefill = '') => set({ palette: { open: true, prefill } });
export const closePalette = () => set(s => ({ palette: { ...s.palette, open: false } }));

export const openSwitcher = () => set({ switcher: true });
export const closeSwitcher = () => set({ switcher: false });

export function confirm(req: Omit<ConfirmReq, 'resolve'>): Promise<boolean> {
  return new Promise(resolve => set({ confirm: { ...req, resolve } }));
}
/** 带一个勾选项的确认框：返回点没点确认、勾没勾 */
export async function confirmCheck(req: Omit<ConfirmReq, 'resolve' | 'check'> & { check: { label: string; why?: string } }) {
  const box = { checked: false };
  const ok = await confirm({ ...req, check: { ...req.check, box } });
  return { ok, checked: ok && box.checked && !req.check.why };
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
/** 机器 Tab 主区切到总览、对比或共用配置 */
export const setHostsView = (v: HostsView) => {
  pref.set('hosts.view', v);
  set({ hostsView: v });
};
/** 打开 / 关掉（null）这块白板的回收站 */
export const openTrash = (path: string | null) => set({ trash: path });
export const openDispatch = (card: number | null) => set({ dispatchOf: card });
/** 打开 / 关掉（null）「会话」弹窗：id 是会话里任一次派发，card 是要定位到的卡 */
export const openSession = (req: { id: number; card?: number } | null) => set({ session: req });

export const openCtx = (req: CtxReq) => set({ ctx: req });
export const closeCtx = () => set({ ctx: null });

/** 打开 / 关掉（null）白板设置；focus：定位到这个状态的排序依据 */
export const openBoardSettings = (path: string | null, focus?: string) => set({ boardSettings: path ? { path, focus } : null });


/** 有弹窗或浮层打开时，全局快捷键不生效 */
export function overlayOpen() {
  const s = get();
  return !!(
    s.confirm || s.form || s.labelsOf || s.picker || s.ctx || s.palette.open || s.switcher || s.creator.open || s.image || s.host ||
    s.dispatchOf != null || s.boardSettings || s.trash || s.session
  );
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
  BUSY: '还在保存',
  NOT_LOGGED_IN: '需要重新登录',
  ALREADY_DISPATCHED: '已经派发过了',
  NO_SUCH_HOST: '机器不存在',
  AGENT_NOT_ON_HOST: '机器上没有这种 agent',
  BOARD_NOT_ON_HOST: '机器没有这块白板的目录',
  NOT_IN_TODO: '卡片不在可领取状态',
  IN_TRASH: '卡片在回收站里',
};
/** 操作被拒绝的提示；extra 可以补充说明或加一个按钮（例如「恢复草稿」） */
export function toastError(err: unknown, extra: Partial<Pick<Toast, 'text' | 'action' | 'duration'>> = {}) {
  const e = err as ApiError;
  return toast({ type: 'error', title: ERROR_TITLES[e.code] || '操作被拒绝', code: e.code, text: e.message, ...extra });
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
export const reveal = (id: number, pulse = true) => set({ reveal: { id, key: ++revealSeq, pulse } });
export const rejectColumn = (col: ColumnKey) => set({ reject: { col, key: ++revealSeq } });

export const setDrag = (drag: DragUI | null) => set({ drag });
