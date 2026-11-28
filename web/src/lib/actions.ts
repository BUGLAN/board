/* ==========================================================================
   跨组件的界面动作：打开白板、整理、复制为提示词、项目与白板的新建 / 编辑 / 删除
   ========================================================================== */
import { api } from './api';
import { boardPrompt, detailPrompt, type QuickPrompt } from './prompt';
import { boardFlow, cardTotal, INBOX, KEY_RE, PROJECT_COLORS, slug } from './schema';
import { projectDirError } from './projectDir';
import type { CardDetail, Pos } from './types';
import { copyText, isMobile } from './util';
import { batchCards, batchMovePatch, boardMeta, canCardBack, canCardFwd, cardBackStep, cardFwdStep, isDraft, jumpDetail, jumpToCard, openDetail, optimistic, patchProject, projectKey, setPath, useData, type BatchResult, type Patch } from '../store/data';
import {
  closeSwitcher,
  confirm,
  openForm,
  openProjectSettings,
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
  if (isDraft(id)) return; // 还在新建：服务端确认后才有详情
  const c = data().board?.cards[id];
  const tab = useUI.getState().tab;
  if (c?.column === 'inbox' && tab !== 'inbox' && isMobile()) setTab('inbox');
  else if (c && c.column !== 'inbox' && (tab === 'settings' || tab === 'projects')) setTab('board');
  // 已经打开着别的卡片（例如从命令面板、搜索结果打开）：同样是换一张，能返回
  if (data().detailId != null && data().detailId !== id) void jumpDetail(id);
  else openDetail(id);
  setTimeout(() => reveal(id), 60);
}

/** 打开另一块白板上的卡片（命令面板的跨白板搜索结果）：先切到那块白板，打开详情 */
export async function openOtherCard(path: string, id: number) {
  if (path !== data().path || !data().board) await openBoard(path);
  if (data().detailId != null && data().detailId !== id) await jumpDetail(id);
  else await openDetail(id);
  setTimeout(() => reveal(id), 60);
}

/** 详情里打开另一张卡片（描述、评论里的 `项目/白板#ID`、上一张 / 下一张）：换掉当前这条地址，
    原来那张记进「返回」，所以卡片详情顶部有返回 / 前进 */
export function openLink(path: string, id: number) {
  if (isDraft(id)) return;
  const tab = useUI.getState().tab;
  if (tab === 'settings' || tab === 'projects') setTab('board');
  if (path === data().path) void jumpDetail(id);
  else void jumpToCard(path, id);
  setTimeout(() => reveal(id), 60);
}

/** 卡片详情顶部的「返回」「前进」（不是浏览器的后退 / 前进：只在打开过的卡片之间走） */
export function cardBack() {
  const id = data().detailId;
  if (canCardBack()) void cardBackStep();
  if (id != null) setTimeout(() => reveal(data().detailId ?? id), 60);
}
export function cardFwd() {
  const id = data().detailId;
  if (canCardFwd()) void cardFwdStep();
  if (id != null) setTimeout(() => reveal(data().detailId ?? id), 60);
}

/** 未整理 → 首个可领取状态最下面（当帧移过去，被拒绝时回到未整理）。返回实际成功 / 失败的 ID；
    提示和离场动画由调用方按结果更新。桌面的白板页上，落位后定位到它 */
export async function triage(path: string, id: number): Promise<BatchResult> {
  if (isDraft(id)) return { ok: [], failed: [] };
  const result = await triageMany(path, [id]);
  if (result.ok.length && useUI.getState().tab === 'board' && !isMobile()) setTimeout(() => reveal(id), 60);
  return result;
}

/** 批量整理到首个可领取状态最下面：整批一个请求；返回实际成功、失败的 ID */
export async function triageMany(path: string, ids: number[]): Promise<BatchResult> {
  const ok = ids.filter(id => !isDraft(id));
  if (!ok.length) return { ok: [], failed: [] };
  const start = boardFlow(useData.getState().board).home;
  const to = start?.key ?? 'todo';
  try {
    return await batchCards(path, ok, { op: 'move', column: to }, batchMovePatch(ok, to));
  } catch (e) {
    return { ok: [], failed: ok.map(id => ({ id, error: e })) };
  }
}

/** 撤销整理：放回「未整理」最下面（只有人能移入，界面上就是人） */
export async function untriage(path: string, ids: number[]) {
  try {
    const result = await batchCards(path, ids, { op: 'move', column: INBOX }, batchMovePatch(ids, INBOX));
    if (result.failed[0]) toastError(result.failed[0].error);
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

/** 一张卡片；在「未整理」里的复制拆解这一句 */
export const copyCardPrompt = (c: Pick<CardDetail, 'id' | 'column'>, path = data().path) => copyPrompt(detailPrompt(path, c));

/** 快捷提示词：拆解 / 上传任务（可到待办或未整理）、做待办全部、做待办中带指定标签的任务 */
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
    {
      name: 'repo', label: '仓库地址', hint: '选填，git 远端；项目下所有白板共用，机器上克隆到 <工作空间>/<项目目录>。之后在项目设置里改', type: 'text',
      maxLength: 500, placeholder: 'git@github.com:org/repo.git',
    },
    {
      name: 'dir', label: '项目目录', hint: '选填，每台机器上这个项目的目录是 <工作空间>/<项目目录>；留空用项目标识。本地仓库已经叫别的名字（例如 board）时填它', type: 'text',
      maxLength: 64, placeholder: '留空用项目标识',
      validate: (d, v) => projectDirError(d, v.key ?? '', useData.getState().ws?.projects ?? []),
    },
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
      const { key } = await optimistic(null, () =>
        api.createProject({
          name: v.name, key: v.key, desc: v.desc, color: v.color, pos, dir: v.dir?.trim() || undefined,
          board: { name: v.boardName, key: v.boardKey || 'main' },
        }),
      );
      const path = `${key}/${v.boardKey || 'main'}`;
      // 仓库地址在项目建好之后存（新建项目的接口不带它）；没存上时项目照样建好，提示里能打开项目设置再填
      const repo = v.repo?.trim();
      if (repo) {
        await optimistic(null, () => api.updateProject(key, { repo })).catch((e: Error) =>
          toast({ type: 'error', title: '项目已建好，仓库地址没存上', text: e.message, action: { label: '项目设置', fn: () => openProjectSettings(key) } }),
        );
      }
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
      const { path } = await optimistic(null, () =>
        api.createBoard({ project: v.project, name: v.name, key: v.key, desc: v.desc, pos: v.project === pk ? pos : undefined }),
      );
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

const saved = () => toast({ type: 'success', title: '已保存', duration: 1400 });

/** 编辑白板。focus: 'readme' 时打开后定位到「说明」；draft 是上次没保存成功的内容 */
export function editBoard(path: string, { focus, draft }: { focus?: string; draft?: Record<string, string> } = {}) {
  const m = boardMeta(data().ws, path);
  if (!m) return;
  openForm({
    title: '编辑白板',
    submit: '保存',
    preview: 'edit-board',
    previewData: { path, projectName: m.project.name, color: m.project.color },
    focus,
    fields: [
      { name: 'name', label: '白板名称', value: draft?.name ?? m.board.name, required: true },
      { name: 'key', label: '白板标识', hint: 'agent 用它定位，创建后不能修改', type: 'key', value: m.board.key, readonly: true },
      { name: 'desc', label: '描述', hint: '选填，一句话', value: draft?.desc ?? m.board.desc },
      {
        name: 'readme', label: '说明', hint: '选填，Markdown，可以粘贴图片；打开大盘的人第一眼看到', type: 'markdown', board: path,
        placeholder: '这块白板是做什么的、有哪些约定', value: draft?.readme ?? m.board.readme ?? '',
      },
      { name: 'color', label: '颜色', hint: `白板用所属项目的颜色，改这里会一起改项目「${m.project.name}」`, type: 'color', value: draft?.color ?? m.project.color },
    ],
    // 乐观保存：表单当即关闭、改动当帧生效；被拒绝时改回来并提示，提示里可以带着刚才写的内容重新打开
    onSubmit: async v => {
      const patch = { name: v.name.trim(), desc: v.desc.trim(), readme: v.readme.trimEnd() };
      const recolor = !!v.color && v.color.toUpperCase() !== m.project.color.toUpperCase();
      optimistic(
        {
          ...patchProject(m.project.key, p => ({
            ...p,
            color: recolor ? v.color : p.color,
            boards: p.boards.map(b => (b.path === path ? { ...b, ...patch } : b)),
          })),
          keys: [projectKey(m.project.key)],
        },
        async () => {
          await api.updateBoard(path, patch);
          if (recolor) await api.updateProject(m.project.key, { color: v.color });
        },
      ).then(saved, e => toastError(e, { action: { label: '重新编辑', fn: () => editBoard(path, { focus, draft: v }) } }));
    },
  });
}

export async function setProjectColor(pk: string, color: string) {
  try {
    await optimistic({ ...patchProject(pk, p => ({ ...p, color })), keys: [projectKey(pk)] }, () => api.updateProject(pk, { color }));
  } catch (e) {
    toastError(e);
  }
}

/** 删除白板 / 项目：确认后当帧从工作区拿掉，被拒绝时回来并提示 */
const dropBoard = (path: string): Patch => {
  const [pk] = path.split('/');
  return { ...patchProject(pk, p => ({ ...p, boards: p.boards.filter(b => b.path !== path) })), keys: [projectKey(pk)] };
};
const dropProject = (pk: string): Patch => ({ ws: ws => ({ ...ws, projects: ws.projects.filter(p => p.key !== pk) }), keys: [projectKey(pk)] });

export async function deleteBoard(path: string) {
  const m = boardMeta(data().ws, path);
  if (!m) return;
  const total = cardTotal(m.board.counts);
  const ok = await confirm({
    title: `删除白板「${m.board.name}」？`,
    text: `${total} 张卡片和全部动态会被永久删除，无法恢复。使用 BOARD=${path} 的 agent 会收到 BOARD_NOT_FOUND。`,
    ok: '删除白板',
  });
  if (!ok) return;
  try {
    await optimistic(dropBoard(path), () => api.deleteBoard(path));
    toast({ type: 'success', title: `已删除白板「${m.board.name}」` });
  } catch (e) {
    toastError(e);
  }
}

export async function deleteProject(pk: string) {
  const p = data().ws?.projects.find(x => x.key === pk);
  if (!p) return;
  const total = p.boards.reduce((n, b) => n + cardTotal(b.counts), 0);
  const ok = await confirm({
    title: `删除项目「${p.name}」？`,
    text: `项目下的 ${p.boards.length} 块白板、${total} 张卡片会被永久删除，无法恢复。`,
    ok: '删除项目',
  });
  if (!ok) return;
  try {
    await optimistic(dropProject(pk), () => api.deleteProject(pk));
    toast({ type: 'success', title: `已删除项目「${p.name}」` });
  } catch (e) {
    toastError(e);
  }
}

export async function copyBoardEnv(path: string) {
  await copyText(`export BOARD=${path}`);
  toast({ type: 'success', title: '已复制', text: `export BOARD=${path}`, duration: 1800 });
}

