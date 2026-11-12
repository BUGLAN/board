/* 桌面端更新：版本比较、检查结果、下载进度、自动检查开关（外壳命令本身由 src-tauri 的单元测试守着） */
import { describe, expect, it } from 'vitest';
import {
  AUTO_CHECK_INTERVAL,
  AUTO_CHECK_START_DELAY,
  autoCheckDelay,
  autoCheckUpdate,
  downloadPercent,
  failCheck,
  failureText,
  finishCheck,
  formatBytes,
  installedText,
  isNewer,
  lastAutoCheck,
  markAutoCheck,
  newerVersion,
  progressText,
  setAutoCheckUpdate,
  versionParts,
  type UpdateCheck,
} from './update';

const check = (current: string, version: string | null, notes: string | null = null, date: string | null = null): UpdateCheck => ({ current, version, notes, date });

describe('versionParts', () => {
  it('按点分成数字，忽略 v 前缀和预发布后缀', () => {
    expect(versionParts('1.2.3')).toEqual([1, 2, 3]);
    expect(versionParts('v0.1.12')).toEqual([0, 1, 12]);
    expect(versionParts('0.2.0-beta.1')).toEqual([0, 2, 0]);
    expect(versionParts(' 1.0 ')).toEqual([1, 0]);
  });
  it('不是纯数字、超过三段的返回 null', () => {
    expect(versionParts('latest')).toBeNull();
    expect(versionParts('0.1.x')).toBeNull();
    expect(versionParts('1.2.3.4')).toBeNull();
    expect(versionParts('')).toBeNull();
  });
});

describe('isNewer', () => {
  it('按数字比较，不按字符串', () => {
    expect(isNewer('0.1.13', '0.1.12')).toBe(true);
    expect(isNewer('0.2.0', '0.1.99')).toBe(true);
    expect(isNewer('0.10.0', '0.9.0')).toBe(true);
    expect(isNewer('0.1.9', '0.1.12')).toBe(false); // 字符串比较会误判
  });
  it('同一版、旧版都不算新版本', () => {
    expect(isNewer('0.1.12', '0.1.12')).toBe(false);
    expect(isNewer('0.1.11', '0.1.12')).toBe(false);
    expect(isNewer('v0.1.12', '0.1.12')).toBe(false);
  });
  it('两段、三段都认', () => {
    expect(isNewer('1.2', '1.1.9')).toBe(true);
    expect(isNewer('1.2.1', '1.2')).toBe(true);
    expect(isNewer('1.2', '1.2.0')).toBe(false);
  });
  it('解析不了的版本不更新，也不把当前版本换掉', () => {
    expect(isNewer('latest', '0.1.12')).toBe(false);
    expect(isNewer('0.1.13', 'nightly')).toBe(false);
    expect(isNewer('', '0.1.12')).toBe(false);
  });
});

describe('newerVersion / finishCheck', () => {
  it('服务端报的版本号更大才算发现新版本', () => {
    expect(newerVersion(check('0.1.12', '0.1.13'))).toBe('0.1.13');
    expect(newerVersion(check('0.1.12', '0.1.12'))).toBeNull();
    // 清单写错成旧版本（或者被挂了旧的 latest）：不提示更新
    expect(newerVersion(check('0.1.12', '0.1.9'))).toBeNull();
    expect(newerVersion(check('0.1.12', null))).toBeNull();
    expect(newerVersion(null)).toBeNull();
  });
  it('检查的状态：已是最新、发现新版本、失败', () => {
    expect(finishCheck(check('0.1.12', '0.1.13'))).toMatchObject({ kind: 'available', version: '0.1.13' });
    expect(finishCheck(check('0.1.13', '0.1.13')).kind).toBe('up-to-date');
    expect(finishCheck(check('0.1.13', null)).kind).toBe('up-to-date');
    const failed = failCheck('检查更新失败：网络不可用');
    expect(failed.kind).toBe('failed');
    expect(failed.kind === 'failed' && failed.error).toContain('当前版本不受影响');
  });
  it('失败的提示保留原因，并说明可以重试', () => {
    expect(failureText('下载或安装 0.1.13 失败：签名不对')).toBe('下载或安装 0.1.13 失败：签名不对。当前版本不受影响，可以重新检查');
  });
});

describe('下载进度', () => {
  it('知道总大小时算百分比，封顶 100', () => {
    expect(downloadPercent({ downloaded: 50, total: 200 })).toBe(25);
    expect(downloadPercent({ downloaded: 300, total: 200 })).toBe(100);
    expect(downloadPercent({ downloaded: 0, total: 0 })).toBeNull();
    expect(downloadPercent({ downloaded: 10, total: null })).toBeNull();
    expect(downloadPercent(null)).toBeNull();
  });
  it('文字里带已下载和总大小', () => {
    expect(progressText({ downloaded: 1024 * 1024, total: 4 * 1024 * 1024 })).toBe('1.0 MB / 4.0 MB（25%）');
    expect(progressText({ downloaded: 2048, total: null })).toBe('已下载 2.0 KB');
    expect(progressText(null)).toBe('');
  });
  it('文件大小', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(2048)).toBe('2.0 KB');
    expect(formatBytes(20 * 1024 * 1024)).toBe('20.0 MB');
  });
});

describe('装完的提示', () => {
  it('要重启（macOS / Linux）和不重启（Windows 由安装器拉起）两种说法都在', () => {
    expect(installedText(true)).toBe('重启 Board 后就是新版本了。');
    expect(installedText(false)).toBe('安装器会关闭 Board，装好后自动打开新版本。');
    expect(installedText(false)).not.toBe(installedText(true));
  });
});

describe('启动后自动检查', () => {
  it('默认开，关掉后记住', () => {
    expect(autoCheckUpdate()).toBe(true);
    setAutoCheckUpdate(false);
    expect(autoCheckUpdate()).toBe(false);
    setAutoCheckUpdate(true);
    expect(autoCheckUpdate()).toBe(true);
  });
  it('打开时查一次，之后每 30 分钟；上次检查时间持久化，刷新不重复查', () => {
    const now = Date.now();
    expect(AUTO_CHECK_INTERVAL).toBe(30 * 60 * 1000);
    expect(autoCheckDelay(0, now)).toBe(AUTO_CHECK_START_DELAY);
    expect(autoCheckDelay(now - 10 * 60 * 1000, now)).toBe(20 * 60 * 1000);
    expect(autoCheckDelay(now - 40 * 60 * 1000, now)).toBe(AUTO_CHECK_START_DELAY);
    markAutoCheck(now);
    expect(lastAutoCheck()).toBe(now);
  });
});
