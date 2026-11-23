/* 自动检查更新的提示（default/main#327）：只有发现新版本才弹确认框，失败、已是最新都不弹；网页端不自动检查 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { UpdateCheck } from './update';

const shell = vi.hoisted(() => ({ canUpdate: true, checkUpdate: vi.fn(), installUpdate: vi.fn(), localHost: vi.fn(), restartApp: vi.fn() }));

vi.mock('./platform', () => ({
  get canUpdate() {
    return shell.canUpdate;
  },
  checkUpdate: (...a: unknown[]) => shell.checkUpdate(...a),
  installUpdate: (...a: unknown[]) => shell.installUpdate(...a),
  localHost: (...a: unknown[]) => shell.localHost(...a),
  restartApp: (...a: unknown[]) => shell.restartApp(...a),
}));

const data = vi.hoisted(() => ({ ws: null as null | { dispatches?: { state: string; host: string }[] } }));

vi.mock('../store/data', () => ({
  useData: { getState: () => ({ ws: data.ws }) },
}));

const ui = vi.hoisted(() => ({
  confirm: vi.fn(async (_req: { title: string; text: string; ok?: string }): Promise<boolean> => false),
  setTab: vi.fn(),
  toast: vi.fn(),
  scrollIntoView: vi.fn(),
}));

vi.mock('../store/ui', () => ({
  confirm: (req: { title: string; text: string; ok?: string }) => ui.confirm(req),
  setTab: (...a: unknown[]) => ui.setTab(...a as []),
  toast: (...a: unknown[]) => ui.toast(...a as []),
}));

/** 外壳 update_check 的返回：比当前的 0.1.0 新 */
const NEWER: UpdateCheck = { current: '0.1.0', version: '0.2.0', notes: '修了 bug', date: '2026-11-14T00:00:00Z' };
/** 已是最新 */
const LATEST: UpdateCheck = { current: '0.1.0', version: null, notes: null, date: null };

/** 每个用例重新加载模块：started / timer 都是模块里的状态 */
async function load() {
  vi.resetModules();
  return { ...(await import('./update-runner')), update: await import('./update') };
}

beforeEach(() => {
  shell.canUpdate = true;
  shell.checkUpdate.mockReset();
  shell.installUpdate.mockReset();
  shell.localHost.mockReset();
  shell.localHost.mockResolvedValue({ enabled: false, running: false, name: '', defaultName: 'win' });
  shell.restartApp.mockReset();
  data.ws = null;
  localStorage.clear();
  ui.confirm.mockClear();
  ui.confirm.mockImplementation(async () => false as unknown as never);
  ui.setTab.mockClear();
  ui.toast.mockClear();
  Element.prototype.scrollIntoView = ui.scrollIntoView as unknown as () => void;
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

/** 让启动延迟那次定时检查跑完 */
async function flushCheck() {
  await vi.advanceTimersByTimeAsync(1600);
}

describe('网页端不发起自动检查（autoUpdateCheck）', () => {
  it('canUpdate 为 false：不检查、不排定时器、不弹任何东西', async () => {
    shell.canUpdate = false;
    const { autoUpdateCheck, update } = await load();
    autoUpdateCheck();
    await flushCheck();
    expect(shell.checkUpdate).not.toHaveBeenCalled();
    expect(update.lastAutoCheck()).toBe(0);
    expect(ui.confirm).not.toHaveBeenCalled();
    expect(ui.toast).not.toHaveBeenCalled();
  });

  it('canUpdate 为 false 时 resumeAutoUpdateCheck 也不检查', async () => {
    shell.canUpdate = false;
    const { resumeAutoUpdateCheck } = await load();
    resumeAutoUpdateCheck();
    await flushCheck();
    expect(shell.checkUpdate).not.toHaveBeenCalled();
  });

  it('桌面端：等启动延迟后检查一次', async () => {
    shell.checkUpdate.mockResolvedValue(LATEST);
    const { autoUpdateCheck, update } = await load();
    autoUpdateCheck();
    await vi.advanceTimersByTimeAsync(1000);
    expect(shell.checkUpdate).not.toHaveBeenCalled();
    await flushCheck();
    expect(shell.checkUpdate).toHaveBeenCalledTimes(1);
    expect(update.lastAutoCheck()).toBeGreaterThan(0);
  });
});

describe('只有发现新版本才弹确认框（checkQuietly）', () => {
  it('发现新版本：弹「发现新版本」确认框；点「去设置里安装」跳到更新卡片', async () => {
    shell.checkUpdate.mockResolvedValue(NEWER);
    ui.confirm.mockImplementation(async () => true as unknown as never);
    const { autoUpdateCheck, useUpdate } = await load();
    autoUpdateCheck();
    await flushCheck();
    expect(ui.toast).not.toHaveBeenCalled();
    expect(ui.confirm).toHaveBeenCalledTimes(1);
    const req = ui.confirm.mock.calls[0][0] as { title: string; text: string; ok?: string };
    expect(req.title).toBe('发现新版本 0.2.0');
    expect(req.text).toContain('修了 bug');
    expect(req.ok).toBe('去设置里安装');
    expect(useUpdate.getState().check).toMatchObject({ kind: 'available', version: '0.2.0' });
    expect(ui.setTab).toHaveBeenCalledWith('settings');
  });

  it('已是最新：不弹确认框，也不发 toast', async () => {
    shell.checkUpdate.mockResolvedValue(LATEST);
    const { autoUpdateCheck, useUpdate } = await load();
    autoUpdateCheck();
    await flushCheck();
    expect(ui.confirm).not.toHaveBeenCalled();
    expect(ui.toast).not.toHaveBeenCalled();
    expect(useUpdate.getState().check.kind).toBe('up-to-date');
  });

  it('检查失败（网络错误）：静默，结果留在 store 里给设置页看', async () => {
    shell.checkUpdate.mockRejectedValue(new Error('network down'));
    const { autoUpdateCheck, useUpdate } = await load();
    autoUpdateCheck();
    await flushCheck();
    expect(ui.confirm).not.toHaveBeenCalled();
    expect(ui.toast).not.toHaveBeenCalled();
    const check = useUpdate.getState().check;
    expect(check.kind).toBe('failed');
    expect(check.kind === 'failed' && check.error).toContain('network down');
    expect(check.kind === 'failed' && check.error).toContain('当前版本不受影响，可以重新检查');
  });

  it('旧版桌面端（unsupported）：静默，不发「没有检查到更新」toast', async () => {
    shell.checkUpdate.mockRejectedValue('unsupported');
    const { autoUpdateCheck, useUpdate } = await load();
    autoUpdateCheck();
    await flushCheck();
    expect(ui.toast).not.toHaveBeenCalled();
    expect(ui.confirm).not.toHaveBeenCalled();
    expect(useUpdate.getState().check).toMatchObject({ kind: 'failed' });
  });
});

describe('发现新版本后自动下载并安装（default/main#352）', () => {
  it('默认关：发现新版本照样弹确认框，不下载', async () => {
    shell.checkUpdate.mockResolvedValue(NEWER);
    const { autoUpdateCheck, update } = await load();
    expect(update.autoInstallUpdate()).toBe(false);
    autoUpdateCheck();
    await flushCheck();
    expect(ui.confirm).toHaveBeenCalledTimes(1);
    expect(shell.installUpdate).not.toHaveBeenCalled();
    expect(shell.restartApp).not.toHaveBeenCalled();
  });

  it('打开后：不弹确认框，直接下载安装，进度在 store 里；装好后（macOS / Linux）重启', async () => {
    shell.checkUpdate.mockResolvedValue(NEWER);
    shell.installUpdate.mockImplementation(async (_v: string, onProgress?: (p: { downloaded: number; total: number | null }) => void) => {
      onProgress?.({ downloaded: 5, total: 10 });
      return { version: '0.2.0', restart: true };
    });
    const { autoUpdateCheck, useUpdate, AUTO_RESTART_DELAY, update } = await load();
    update.setAutoInstallUpdate(true);
    autoUpdateCheck();
    await flushCheck();
    expect(ui.confirm).not.toHaveBeenCalled();
    expect(shell.installUpdate).toHaveBeenCalledTimes(1);
    expect(shell.installUpdate.mock.calls[0][0]).toBe('0.2.0');
    expect(useUpdate.getState().install).toEqual({ kind: 'ready', version: '0.2.0', restart: true });
    expect(ui.toast).toHaveBeenCalledWith(expect.objectContaining({ title: '0.2.0 已安装，正在重启 Board' }));
    expect(shell.restartApp).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(AUTO_RESTART_DELAY);
    expect(shell.restartApp).toHaveBeenCalledTimes(1);
  });

  it('Windows（装好由安装器重开）：不调重启', async () => {
    shell.checkUpdate.mockResolvedValue(NEWER);
    shell.installUpdate.mockResolvedValue({ version: '0.2.0', restart: false });
    const { autoUpdateCheck, AUTO_RESTART_DELAY, update } = await load();
    update.setAutoInstallUpdate(true);
    autoUpdateCheck();
    await flushCheck();
    await vi.advanceTimersByTimeAsync(AUTO_RESTART_DELAY);
    expect(shell.installUpdate).toHaveBeenCalledTimes(1);
    expect(shell.restartApp).not.toHaveBeenCalled();
    expect(ui.confirm).not.toHaveBeenCalled();
  });

  it('有 running 派发：先显示等待，不停 host；跑完重新检查一次再安装', async () => {
    shell.checkUpdate.mockResolvedValue(NEWER);
    shell.installUpdate.mockResolvedValue({ version: '0.2.0', restart: true });
    shell.localHost.mockResolvedValue({ enabled: true, running: true, name: 'win', defaultName: 'WIN' });
    data.ws = { dispatches: [{ state: 'running', host: 'win' }] };
    const { autoUpdateCheck, useUpdate, update, AUTO_INSTALL_WAIT_MS } = await load();
    update.setAutoInstallUpdate(true);
    autoUpdateCheck();
    await flushCheck();
    expect(ui.confirm).not.toHaveBeenCalled();
    expect(shell.installUpdate).not.toHaveBeenCalled();
    expect(useUpdate.getState().install).toEqual({ kind: 'waiting', version: '0.2.0', running: 1 });
    // 还在跑：继续等，不安装
    await vi.advanceTimersByTimeAsync(AUTO_INSTALL_WAIT_MS);
    expect(shell.installUpdate).not.toHaveBeenCalled();
    expect(useUpdate.getState().install).toEqual({ kind: 'waiting', version: '0.2.0', running: 1 });
    // 跑完了：重新检查（第二次 check），仍可更新才安装
    data.ws = { dispatches: [] };
    await vi.advanceTimersByTimeAsync(AUTO_INSTALL_WAIT_MS);
    expect(shell.checkUpdate).toHaveBeenCalledTimes(2);
    expect(shell.installUpdate).toHaveBeenCalledTimes(1);
    expect(useUpdate.getState().install).toEqual({ kind: 'ready', version: '0.2.0', restart: true });
  });

  it('工作区还没取到：先等，不把不知道当成空闲', async () => {
    shell.checkUpdate.mockResolvedValue(NEWER);
    shell.localHost.mockResolvedValue({ enabled: true, running: true, name: 'win', defaultName: 'WIN' });
    const { autoUpdateCheck, useUpdate, update } = await load();
    update.setAutoInstallUpdate(true);
    autoUpdateCheck();
    await flushCheck();
    expect(shell.installUpdate).not.toHaveBeenCalled();
    expect(useUpdate.getState().install).toEqual({ kind: 'waiting', version: '0.2.0', running: null });
  });

  it('装不上：提示一次，不重启，结果留在 store 里给设置页看', async () => {
    shell.checkUpdate.mockResolvedValue(NEWER);
    shell.installUpdate.mockRejectedValue('签名对不上');
    const { autoUpdateCheck, useUpdate, AUTO_RESTART_DELAY, update } = await load();
    update.setAutoInstallUpdate(true);
    autoUpdateCheck();
    await flushCheck();
    await vi.advanceTimersByTimeAsync(AUTO_RESTART_DELAY);
    expect(shell.restartApp).not.toHaveBeenCalled();
    expect(ui.toast).toHaveBeenCalledTimes(1);
    expect(ui.toast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error', title: '自动安装 0.2.0 没成' }));
    expect(useUpdate.getState().install).toEqual({ kind: 'failed', error: '签名对不上' });
  });

  it('已是最新：打开了也不下载', async () => {
    shell.checkUpdate.mockResolvedValue(LATEST);
    const { autoUpdateCheck, update } = await load();
    update.setAutoInstallUpdate(true);
    autoUpdateCheck();
    await flushCheck();
    expect(shell.installUpdate).not.toHaveBeenCalled();
    expect(ui.toast).not.toHaveBeenCalled();
  });
});

describe('安装状态（installNow / clearInstall）', () => {
  it('正在下载时再装一次：直接返回这一次，不重复下载；装好后重新检查不清掉「等重启」', async () => {
    let finish: (v: { version: string; restart: boolean }) => void = () => {};
    shell.installUpdate.mockImplementation(() => new Promise(r => (finish = r)));
    const { useUpdate } = await load();
    const a = useUpdate.getState().installNow('0.2.0');
    const b = useUpdate.getState().installNow('0.2.0');
    expect(a).toBe(b);
    expect(useUpdate.getState().install.kind).toBe('downloading');
    useUpdate.getState().clearInstall();
    expect(useUpdate.getState().install.kind).toBe('downloading');
    finish({ version: '0.2.0', restart: true });
    await a;
    expect(shell.installUpdate).toHaveBeenCalledTimes(1);
    useUpdate.getState().clearInstall();
    expect(useUpdate.getState().install.kind).toBe('ready');
  });

  it('失败后重新检查：清掉失败结果', async () => {
    shell.installUpdate.mockRejectedValue('网络不通');
    const { useUpdate } = await load();
    await useUpdate.getState().installNow('0.2.0');
    expect(useUpdate.getState().install.kind).toBe('failed');
    useUpdate.getState().clearInstall();
    expect(useUpdate.getState().install.kind).toBe('idle');
  });
});
