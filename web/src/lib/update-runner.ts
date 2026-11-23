/* ==========================================================================
   启动后自动检查更新（lib/update.ts 是逻辑，components/update-card.tsx 是界面，这里把它们接起来）
   - 只在桌面端（canUpdate）打开时检查一次，之后每 30 分钟一次；设置里关掉后不再检查；网页端什么都不做
   - 上次检查时间持久化：页面刷新、切换设置页、重复挂载不会多查一次
   - 检查结果放在这里（store）：设置页的更新卡片打开时直接显示，不用再查一遍
   - 有新版本：应用内的确认框（store/ui 的 confirm），「以后再说」就是这次先不装；不弹系统框、不阻塞操作
   - 打开了「发现新版本后自动下载并安装」：不弹确认框，直接下载、安装（进度和结果同样在 store 里，设置页的卡片看得到），
     装好后 macOS / Linux 重启 Board，Windows 由安装器关掉再打开新版本；装不上时提示一次，当前版本不受影响
   - 自动安装前先看这台电脑还有没有 running 派发：有就等 running 为空后重新检查再装，等待期间不停 board host（#633）
   - 已是最新、连不上、清单不对、旧版桌面端：一律静默，不弹任何提示（default/main#327）；
     要重试或看失败原因就点设置页的「检查更新」
   ========================================================================== */
import { create } from 'zustand';
import { canUpdate, checkUpdate, installUpdate, localHost, restartApp } from './platform';
import {
  autoCheckDelay,
  autoCheckUpdate,
  autoInstallUpdate,
  failCheck,
  finishCheck,
  lastAutoCheck,
  localRunningCount,
  markAutoCheck,
  type CheckState,
  type InstallState,
} from './update';
import { autoNotice } from './update-notice';
import { confirm, setTab, toast } from '../store/ui';
import { useData } from '../store/data';

interface UpdateStore {
  check: CheckState;
  /** 下载安装的状态（设置页的「下载并安装」和自动安装共用） */
  install: InstallState;
  /** 检查一次，返回结果（设置页的按钮也用它）；正在查的时候再点直接返回这次的结果 */
  run: () => Promise<CheckState>;
  /** 下载并安装这个版本，返回结果；正在下载时再调直接返回这次的结果 */
  installNow: (version: string) => Promise<InstallState>;
  /** 重新检查前清掉上一次的安装结果（装好了等重启的留着） */
  clearInstall: () => void;
}

let running: Promise<CheckState> | null = null;
let installing: Promise<InstallState> | null = null;
let waitTimer: number | null = null;

/** 自动安装装好后等这么久再重启：先让「正在重启」的提示露一下 */
export const AUTO_RESTART_DELAY = 1500;
/** 自动安装等待期间多久看一次派发是否跑完 */
export const AUTO_INSTALL_WAIT_MS = 2000;

export const useUpdate = create<UpdateStore>((set, get) => ({
  check: { kind: 'idle' },
  install: { kind: 'idle' },
  clearInstall: () => {
    clearAutoWait();
    if (get().install.kind !== 'ready' && !installing) set({ install: { kind: 'idle' } });
  },
  installNow: version => {
    if (installing) return installing;
    clearAutoWait();
    set({ install: { kind: 'downloading', progress: null } });
    installing = (async () => {
      let next: InstallState;
      try {
        const r = await installUpdate(version, p => set({ install: { kind: 'downloading', progress: p } }));
        next = { kind: 'ready', version: r.version, restart: r.restart };
      } catch (e) {
        next = { kind: 'failed', error: String(e) };
      }
      set({ install: next });
      installing = null;
      return next;
    })();
    return installing;
  },
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
  // 网页端没有外壳的 update_check（checkUpdate 会抛 unsupported）：不排定时器、不检查，什么都不做
  if (!canUpdate) return;
  if (started) return;
  started = true;
  if (!autoCheckUpdate()) return;
  scheduleNext();
}

/** 设置里重新打开「启动后自动检查」：如果之前已经启动过，重新排下一次 */
export function resumeAutoUpdateCheck() {
  if (!canUpdate) return;
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
  // 只有发现新版本才弹确认框；已是最新、失败（含旧版桌面端的 unsupported）都静默（lib/update-notice.ts）
  if (autoNotice(state).action !== 'confirm' || state.kind !== 'available') return;
  if (autoInstallUpdate()) return installQuietly(state.version);
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

/** 自动下载并安装：不问，装好后按平台重启（Windows 上安装器会关掉 Board 再打开新版本，走不到这里） */
async function installQuietly(version: string) {
  // 设置页里正在装、已经装好等重启：不再装一遍
  const current = useUpdate.getState().install.kind;
  if (current === 'ready' || current === 'downloading') return;
  const busy = await localRunning();
  if (busy == null || busy > 0) return waitForIdle(version, busy);
  clearAutoWait();
  const done = await useUpdate.getState().installNow(version);
  if (done.kind === 'failed') {
    toast({ type: 'error', title: `自动安装 ${version} 没成`, text: `${done.error}。当前版本不受影响，可以在设置里重新检查后再装` });
    return;
  }
  if (done.kind !== 'ready' || !done.restart) return;
  toast({ type: 'info', title: `${done.version} 已安装，正在重启 Board`, duration: AUTO_RESTART_DELAY });
  window.setTimeout(() => restartApp(), AUTO_RESTART_DELAY);
}

/** 本机还有多少 running 派发；工作区没取到时返回 null，自动安装会继续等 */
async function localRunning(): Promise<number | null> {
  return localRunningCount(useData.getState().ws, await localHost());
}

function waitForIdle(version: string, running: number | null) {
  useUpdate.setState({ install: { kind: 'waiting', version, running } });
  if (waitTimer != null) window.clearTimeout(waitTimer);
  waitTimer = window.setTimeout(() => {
    waitTimer = null;
    void retryQuietInstall(version);
  }, AUTO_INSTALL_WAIT_MS);
}

function clearAutoWait() {
  if (waitTimer != null) window.clearTimeout(waitTimer);
  waitTimer = null;
}

/** 等待期间定时看一次：还在跑就接着等；跑完了重新检查，仍可更新才安装 */
async function retryQuietInstall(version: string) {
  if (!autoInstallUpdate()) {
    useUpdate.setState({ install: { kind: 'idle' } });
    return;
  }
  const current = useUpdate.getState().install.kind;
  if (current === 'ready' || current === 'downloading') return;
  const busy = await localRunning();
  if (busy == null || busy > 0) return waitForIdle(version, busy);
  const checked = await useUpdate.getState().run();
  if (checked.kind !== 'available') {
    useUpdate.setState({ install: { kind: 'idle' } });
    return;
  }
  await installQuietly(checked.version);
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
