/* 自动检查更新的提示（default/main#327）：只有发现新版本才弹确认框，失败、已是最新都不弹；网页端不自动检查 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { UpdateCheck } from './update';

const shell = vi.hoisted(() => ({ canUpdate: true, checkUpdate: vi.fn() }));

vi.mock('./platform', () => ({
  get canUpdate() {
    return shell.canUpdate;
  },
  checkUpdate: (...a: unknown[]) => shell.checkUpdate(...a),
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
