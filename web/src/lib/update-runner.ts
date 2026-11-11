/* ==========================================================================
   启动后自动检查更新（lib/update.ts 是逻辑，components/update-card.tsx 是界面，这里把它们接起来）
   - 桌面端打开时检查一次，之后每 30 分钟一次；设置里关掉后不再检查
   - 上次检查时间持久化：页面刷新、切换设置页、重复挂载不会多查一次
   - 检查结果放在这里（store）：设置页的更新卡片打开时直接显示，不用再查一遍
   - 有新版本：应用内的确认框（store/ui 的 confirm），「以后再说」就是这次先不装；不弹系统框、不阻塞操作
   - 连不上、清单不对、旧版桌面端：只发一条 toast，不打扰；可以到设置页手动重试
   ========================================================================== */
import { create } from 'zustand';
import { checkUpdate } from './platform';
import { autoCheckDelay, autoCheckUpdate, failCheck, finishCheck, lastAutoCheck, markAutoCheck, type CheckState } from './update';
import { confirm, setTab, toast } from '../store/ui';

interface UpdateStore {
  check: CheckState;
  /** 检查一次，返回结果（设置页的按钮也用它）；正在查的时候再点直接返回这次的结果 */
  run: () => Promise<CheckState>;
}

let running: Promise<CheckState> | null = null;

export const useUpdate = create<UpdateStore>(set => ({
  check: { kind: 'idle' },
  run: () => {
    if (running) return running;
    set({ check: { kind: 'checking' } });
    running = (async () => {
      let next: CheckState;
      try {
        next = finishCheck(await checkUpdate());
      } catch (e) {
        next = failCheck(e);
      }
      set({ check: next });
      running = null;
      return next;
    })();
    return running;
  },
}));

/** 检查过一次（不管在哪个窗口）就不再自动查 */
let started = false;
let timer: number | null = null;

export function autoUpdateCheck() {
  if (started) return;
  started = true;
  if (!autoCheckUpdate()) return;
  scheduleNext();
}

/** 设置里重新打开「启动后自动检查」：如果之前已经启动过，重新排下一次 */
export function resumeAutoUpdateCheck() {
  if (!autoCheckUpdate()) return;
  started = true;
  scheduleNext();
}

/** 设置里关掉后不再自动检查（手动检查不受影响） */
export function pauseAutoUpdateCheck() {
  if (timer != null) window.clearTimeout(timer);
  timer = null;
}

/** 打开时先查一次（上次查过就等满 30 分钟），之后每 30 分钟一次；关掉开关后下一次不再排 */
function scheduleNext() {
  if (timer != null) window.clearTimeout(timer);
  timer = window.setTimeout(async () => {
    timer = null;
    if (!autoCheckUpdate()) return;
    await checkQuietly();
    markAutoCheck();
    scheduleNext();
  }, autoCheckDelay(lastAutoCheck(), Date.now()));
}

async function checkQuietly() {
  const state = await useUpdate.getState().run();
  // 旧版桌面端没有这个命令（platform.ts 抛出 'unsupported'）：不算出错，什么都不说
  if (state.kind === 'failed') {
    if (state.error === 'unsupported') return;
    toast({ type: 'info', title: '没有检查到更新', text: state.error, duration: 6000 });
    return;
  }
  if (state.kind !== 'available') return;
  const r = state.result;
  const yes = await confirm({
    title: `发现新版本 ${state.version}`,
    text: notesText(r.notes, r.date, r.current),
    markdown: true,
    ok: '去设置里安装',
    cancel: '以后再说',
    icon: 'update',
  });
  if (yes) goToUpdateCard();
}

/** 确认框里的正文（Markdown）：发布时间和当前版本那行，加更新说明（太长时截断） */
export function notesText(notes: string | null, date: string | null, current: string, limit = 700): string {
  const head = date ? `${date.slice(0, 10)} 发布，当前版本 ${current}` : `当前版本 ${current}`;
  const body = (notes ?? '').trim();
  const shown = !body ? '这一版没有写更新说明' : body.length > limit ? `${body.slice(0, limit).trimEnd()}…` : body;
  return `${head}\n\n${shown}`;
}

/** 打开设置页并滚到更新卡片（自动检查后点确认时用） */
export function goToUpdateCard() {
  setTab('settings');
  setTimeout(() => document.getElementById('set-update')?.scrollIntoView({ block: 'center', behavior: 'smooth' }), 120);
}
