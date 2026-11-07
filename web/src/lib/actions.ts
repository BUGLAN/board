/* ==========================================================================
   跨组件的界面动作：打开白板、整理、复制为提示词、项目与白板的新建 / 编辑 / 删除
   ========================================================================== */
import { api } from './api';
import { boardPrompt, cardPrompt, type QuickPrompt } from './prompt';
import { boardFlow, KEY_RE, PROJECT_COLORS, slug } from './schema';
import type { CardDetail, Pos } from './types';
import { copyText, isMobile } from './util';
import { allBoards, boardMeta, closeDetail, exec, openDetail, refresh, setPath, useData } from '../store/data';
import {
  closeSwitcher,
  confirm,
  openForm,
  reveal,
  setTab,
  toast,
  toastError,
  useUI,
  type FormField,
} from '../store/ui';

const data = () => useData.getState();

/* ------------------------------ 白板 ------------------------------ */

/** 打开一块白板（切到「白板」Tab） */
export function openBoard(path: string, { stay = false } = {}) {
  closeSwitcher();
  setPath(path);
  const tab = useUI.getState().tab;
  if (!stay && tab !== 'board' && tab !== 'inbox') setTab('board');
}

/** 打开卡片详情（「未整理」的条目切到未整理 Tab） */
export function openCard(id: number) {
  const c = data().board?.cards[id];
  const tab = useUI.getState().tab;
  if (c?.column === 'inbox' && tab !== 'inbox' && isMobile()) setTab('inbox');
  else if (c && c.column !== 'inbox' && (tab === 'settings' || tab === 'projects')) setTab('board');
  openDetail(id);
  setTimeout(() => reveal(id), 60);
}

export { closeDetail };

/** 未整理 → 首个可领取状态最下面 */
export async function triage(id: number) {
  const start = boardFlow(useData.getState().board).home;
  try {
    await exec({ op: 'move', id, column: start?.key ?? 'todo' });
    if (isMobile()) toast({ type: 'success', title: `#${id} 已整理到「${start?.name ?? '待办'}」`, duration: 1800 });
    else setTimeout(() => reveal(id), 60);
  } catch (e) {
    toastError(e);
  }
}

/* ------------------------------ 复制为提示词 ------------------------------ */
/* 只复制一行 skill 调用（/board-skills mini/main#2），agent 自己用 board CLI 读取卡片 */

async function copyPrompt(text: string, hint = '发给装了 board-skills 的 agent 即可') {
  const ok = await copyText(text);
  toast({
    type: ok ? 'success' : 'info',
    title: ok ? `已复制：${text}` : text,
    text: ok ? hint : '没能写入剪贴板，请手动复制上面这一行',
    duration: ok ? 3200 : 8000,
  });
}

/** 一块白板：agent 循环领取可领取状态里的卡片，直到为空 */
export const copyBoardPrompt = (path = data().path) => copyPrompt(boardPrompt(path));

/** 一张卡片 */
export const copyCardPrompt = (c: Pick<CardDetail, 'id'>, path = data().path) => copyPrompt(cardPrompt(path, c.id));

/** 快捷提示词：拆解任务、上传任务、做待办全部、做待办中带指定标签的任务 */
export const copyQuickPrompt = (q: QuickPrompt) => copyPrompt(q.text, q.hint);

/* ------------------------------ 项目与白板 ------------------------------ */

/** 从名称生成标识；中文名称等无法生成时用 prefix-1、prefix-2… */
function autoKey(name: string, taken: (k: string) => boolean, prefix: string) {
  const s = slug(name);
  if (s && !taken(s)) return s;
  const base = s || prefix;
  let n = s ? 2 : 1;
  let k: string;
  do k = `${base}-${n++}`;
  while (taken(k));
  return k;
}

const keyError = (k: string) => (KEY_RE.test(k) ? null : '标识只能用小写字母、数字和 -，以字母或数字开头，最长 32 个字符');
const projectTaken = (k: string) => !!data().ws?.projects.some(p => p.key === k);
const boardTaken = (pk: string, k: string) => !!data().ws?.projects.find(p => p.key === pk)?.boards.some(b => b.key === k);

export function newProject({ pos }: { pos?: Pos } = {}) {
  closeSwitcher();
  const fields: FormField[] = [
    { name: 'name', label: '项目名称', placeholder: '例如：官网改版', required: true },
    {
      name: 'key', label: '项目标识', hint: '小写字母、数字和 -；创建后不能修改', type: 'key', from: 'name', placeholder: 'site', required: true,
      auto: v => autoKey(v, projectTaken, 'project'),
      validate: k => keyError(k) || (projectTaken(k) ? `项目标识「${k}」已被占用` : null),
    },
    { name: 'desc', label: '描述', hint: '选填', placeholder: '一句话说明这个项目' },
    { name: 'color', label: '颜色', type: 'color', value: PROJECT_COLORS[Math.floor(Math.random() * PROJECT_COLORS.length)] },
    { name: 'boardName', label: '第一块白板', value: '主白板', required: true },
    { name: 'boardKey', label: '白板标识', type: 'key', from: 'boardName', value: 'main', required: true, auto: v => slug(v) || 'main', validate: keyError },
  ];
  openForm({
    title: '新建项目',
    subtitle: '一个项目下可以有多块白板。先建第一块，之后随时在项目里添加。',
    submit: '创建项目',
    preview: 'project',
    fields,
    onSubmit: async v => {
      const { key } = await api.createProject({
        name: v.name, key: v.key, desc: v.desc, color: v.color, pos,
        board: { name: v.boardName, key: v.boardKey || 'main' },
      });
      const path = `${key}/${v.boardKey || 'main'}`;
      await refresh();
      afterCreate(path);
      toast({ type: 'success', title: `已创建项目「${v.name.trim()}」`, text: `agent 用 BOARD=${path} 接入第一块白板。`, duration: 4200 });
    },
  });
}

export function newBoard(pk = data().path.split('/')[0], { pos }: { pos?: Pos } = {}) {
  closeSwitcher();
  const projects = data().ws?.projects ?? [];
  openForm({
    title: '新建白板',
    subtitle: '白板属于一个项目，用「项目/白板」定位。',
    submit: '创建白板',
    preview: 'board',
    fields: [
      { name: 'project', label: '所属项目', type: 'select', value: pk, options: projects.map(p => ({ value: p.key, label: `${p.name}（${p.key}）` })) },
      { name: 'name', label: '白板名称', placeholder: '例如：v1.0 发布', required: true },
      {
        name: 'key', label: '白板标识', hint: '小写字母、数字和 -；创建后不能修改', type: 'key', from: 'name', placeholder: 'release', required: true,
        auto: (n, v) => autoKey(n, k => boardTaken(v.project, k), 'board'),
        validate: (k, v) => keyError(k) || (boardTaken(v.project, k) ? `「${v.project}」里已经有标识为「${k}」的白板` : null),
      },
      { name: 'desc', label: '描述', hint: '选填', placeholder: '这块白板用来做什么' },
    ],
    onSubmit: async v => {
      const { path } = await api.createBoard({ project: v.project, name: v.name, key: v.key, desc: v.desc, pos: v.project === pk ? pos : undefined });
      await refresh();
      if (useUI.getState().tab === 'projects') {
        toast({ type: 'success', title: `已创建白板「${v.name.trim()}」`, text: `BOARD=${path}`, action: { label: '打开', fn: () => openBoard(path) } });
        window.dispatchEvent(new CustomEvent('board:focus', { detail: path }));
      } else {
        afterCreate(path);
        toast({ type: 'success', title: `已创建白板「${v.name.trim()}」`, text: `agent 用 BOARD=${path} 接入。` });
      }
    },
  });
}

function afterCreate(path: string) {
  if (useUI.getState().tab === 'projects') {
    setPath(path);
    window.dispatchEvent(new CustomEvent('board:focus', { detail: path }));
  } else openBoard(path);
}

export function editProject(pk: string) {
  const p = data().ws?.projects.find(x => x.key === pk);
  if (!p) return;
  openForm({
    title: '编辑项目',
    submit: '保存',
    preview: 'edit-project',
    previewData: { key: p.key, boards: String(p.boards.length) },
    fields: [
      { name: 'name', label: '项目名称', value: p.name, required: true },
      { name: 'key', label: '项目标识', hint: '创建后不能修改', type: 'key', value: p.key, readonly: true },
      { name: 'desc', label: '描述', hint: '选填', value: p.desc },
      { name: 'color', label: '颜色', type: 'color', value: p.color },
    ],
    onSubmit: async v => {
      await api.updateProject(pk, { name: v.name, desc: v.desc, color: v.color });
      await refresh();
      toast({ type: 'success', title: '已保存', duration: 1400 });
    },
  });
}

export function editBoard(path: string) {
  const m = boardMeta(data().ws, path);
  if (!m) return;
  openForm({
    title: '编辑白板',
    submit: '保存',
    preview: 'edit-board',
    previewData: { path, projectName: m.project.name, color: m.project.color },
    fields: [
      { name: 'name', label: '白板名称', value: m.board.name, required: true },
      { name: 'key', label: '白板标识', hint: 'agent 用它定位，创建后不能修改', type: 'key', value: m.board.key, readonly: true },
      { name: 'desc', label: '描述', hint: '选填', value: m.board.desc },
      { name: 'color', label: '颜色', hint: `白板用所属项目的颜色，改这里会一起改项目「${m.project.name}」`, type: 'color', value: m.project.color },
    ],
    onSubmit: async v => {
      await api.updateBoard(path, { name: v.name, desc: v.desc });
      if (v.color && v.color.toUpperCase() !== m.project.color.toUpperCase()) await api.updateProject(m.project.key, { color: v.color });
      await refresh();
      toast({ type: 'success', title: '已保存', duration: 1400 });
    },
  });
}

export async function setProjectColor(pk: string, color: string) {
  try {
    await api.updateProject(pk, { color });
    await refresh();
  } catch (e) {
    toastError(e);
  }
}

export async function deleteBoard(path: string) {
  const m = boardMeta(data().ws, path);
  if (!m) return;
  const total = Object.values(m.board.counts).reduce((a, x) => a + x, 0);
  const ok = await confirm({
    title: `删除白板「${m.board.name}」？`,
    text: `${total} 张卡片和全部动态会被永久删除，无法恢复。使用 BOARD=${path} 的 agent 会收到 BOARD_NOT_FOUND。`,
    ok: '删除白板',
  });
  if (!ok) return;
  try {
    await api.deleteBoard(path);
    await refresh();
    toast({ type: 'success', title: `已删除白板「${m.board.name}」` });
  } catch (e) {
    toastError(e);
  }
}

export async function deleteProject(pk: string) {
  const p = data().ws?.projects.find(x => x.key === pk);
  if (!p) return;
  const total = p.boards.reduce((a, b) => a + Object.values(b.counts).reduce((x, y) => x + y, 0), 0);
  const ok = await confirm({
    title: `删除项目「${p.name}」？`,
    text: `项目下的 ${p.boards.length} 块白板、${total} 张卡片会被永久删除，无法恢复。`,
    ok: '删除项目',
  });
  if (!ok) return;
  try {
    await api.deleteProject(pk);
    await refresh();
    toast({ type: 'success', title: `已删除项目「${p.name}」` });
  } catch (e) {
    toastError(e);
  }
}

export async function copyBoardEnv(path: string) {
  await copyText(`export BOARD=${path}`);
  toast({ type: 'success', title: '已复制', text: `export BOARD=${path}`, duration: 1800 });
}

export const boardsCount = () => allBoards(data().ws).length;
