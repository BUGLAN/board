/* ==========================================================================
   设置 → 桌面端更新的卡片（桌面端）
   - 显示当前版本（外壳版本）；右侧「检查更新」按钮手动查一次，结果（已是最新 / 检查失败）在按钮上停 5 秒再变回来，
     失败原因在下一行；有新版本时展开更新说明，「下载并安装」显示下载进度，装完提示重启
   - 启动后自动检查的开关（默认开）；自动检查发现新版本时也用应用内的确认框问（lib/update-runner.ts）
   - 「发现新版本后自动下载并安装」的开关（默认关）：开着时自动检查发现新版本直接下载安装、装好重启，不再问
   检查和安装的状态都在 lib/update-runner.ts 的 store 里：自动安装的进度、结果这里也看得到
   ========================================================================== */
import { useEffect, useRef, useState } from 'react';
import { restartApp, shellVersion } from '../lib/platform';
import {
  autoCheckUpdate,
  autoInstallUpdate,
  downloadPercent,
  installedText,
  progressText,
  setAutoCheckUpdate,
  setAutoInstallUpdate,
  type CheckState,
} from '../lib/update';
import { cx } from '../lib/util';
import { pauseAutoUpdateCheck, resumeAutoUpdateCheck, useUpdate } from '../lib/update-runner';
import { confirm, toast } from '../store/ui';
import { Markdown } from './bits';
import { Icon } from './Icon';

/** 设置里切换开关的小按钮 */
function Toggle({ on, onChange, label, disabled }: { on: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={`switch${on ? ' on' : ''}`}
      disabled={disabled}
      onClick={() => onChange(!on)}
    >
      <i />
    </button>
  );
}

/** 检查结果在按钮上停留的时间 */
export const FLASH_MS = 5000;

/** 检查刚结束（正在检查 → 已是最新 / 检查失败）时返回这个结果，FLASH_MS 后回到 null：按钮上闪一下结果再变回「检查更新」 */
export function useFlash(kind: CheckState['kind']) {
  const [flash, setFlash] = useState<'up-to-date' | 'failed' | null>(null);
  const prev = useRef(kind);
  useEffect(() => {
    const was = prev.current;
    prev.current = kind;
    if (kind === 'checking') return setFlash(null);
    if (was !== 'checking' || (kind !== 'up-to-date' && kind !== 'failed')) return;
    setFlash(kind);
    const t = window.setTimeout(() => setFlash(null), FLASH_MS);
    return () => window.clearTimeout(t);
  }, [kind]);
  return flash;
}

export function UpdateCard() {
  const check = useUpdate(s => s.check);
  const run = useUpdate(s => s.run);
  const install = useUpdate(s => s.install);
  const installNow = useUpdate(s => s.installNow);
  const clearInstall = useUpdate(s => s.clearInstall);
  const [auto, setAuto] = useState(autoCheckUpdate);
  const [autoInstall, setAutoInstall] = useState(autoInstallUpdate);

  const checkNow = () => {
    // 重新检查：上一次的安装结果（装好了等重启的除外）不再显示
    clearInstall();
    void run();
  };

  const doInstall = async (version: string) => {
    const yes = await confirm({
      title: `下载并安装 ${version}？`,
      text: '下载完会先校验签名，再安装。安装过程中会关闭 Board，装好后自动打开新版本（macOS / Linux 装好后需要重启一次）。',
      ok: '下载并安装',
      icon: 'update',
    });
    if (!yes) return;
    await installNow(version);
  };

  const version = shellVersion ?? '未知';
  const flash = useFlash(check.kind);
  const busy = check.kind === 'checking';

  const pct = install.kind === 'downloading' ? downloadPercent(install.progress) : null;

  return (
    <>
      <div className="set-row">
        <div className="set-label">
          <b>桌面端更新</b>
          <span>
            当前版本 <code className="up-ver">{version}</code>
            {shellVersion ? '' : '（页面没拿到外壳版本，更新由桌面端外壳处理）'}
          </span>
        </div>
        <div className="set-control">
          <button
            type="button"
            className={cx('btn sm up-btn', busy && 'is-busy', flash === 'up-to-date' && 'ok', flash === 'failed' && 'fail')}
            disabled={busy}
            onClick={checkNow}
            aria-live="polite"
          >
            <Icon name={flash === 'up-to-date' ? 'checkCircle' : flash === 'failed' ? 'alert' : 'renew'} />
            {busy ? '正在检查…' : flash === 'up-to-date' ? '已是最新' : flash === 'failed' ? '检查失败' : '检查更新'}
          </button>
        </div>
      </div>
      {check.kind === 'failed' && <div className="up-err up-err-line">{check.error}</div>}

      {check.kind === 'available' && install.kind === 'idle' && (
        <div className="up-card is-new">
          <div className="up-head">
            <Icon name="renew" />
            <span className="up-title">
              新版本 <b className="up-ver">{check.version}</b>
              {check.result.date && <span className="up-sub"> · {check.result.date.slice(0, 10)} 发布</span>}
            </span>
            <button type="button" className="btn sm brand" onClick={() => void doInstall(check.version)}>
              <Icon name="renew" />
              下载并安装
            </button>
          </div>
          {check.result.notes?.trim() ? (
            <div className="up-notes md">
              <Markdown src={check.result.notes} links={false} />
            </div>
          ) : (
            <div className="up-notes">这一版没有写更新说明</div>
          )}
          <div className="up-foot">安装包会先校验签名，对不上不会安装</div>
        </div>
      )}

      {install.kind === 'downloading' && (
        <div className="up-card">
          <div className="up-head">
            <Icon name="renew" />
            正在下载并安装…
          </div>
          <div className={`up-bar${pct == null ? ' indet' : ''}`}>
            <i style={{ width: `${pct ?? 0}%` }} />
          </div>
          <div className="up-meter">
            <span>{install.progress ? progressText(install.progress) : '正在连接…'}</span>
            {pct != null && <b>{pct}%</b>}
          </div>
        </div>
      )}

      {install.kind === 'ready' && (
        <div className="up-card is-new">
          <div className="up-head">
            <Icon name="checkCircle" />
            {install.version} 已安装
          </div>
          <p className="set-p" style={{ margin: '8px 0 0' }}>
            {installedText(install.restart)}
          </p>
          {/* 只有要界面重启的平台才给按钮：Windows 由安装器关掉 Board 再打开新版本，没有可点的 */}
          {install.restart && (
            <div className="lh-actions">
              <button type="button" className="btn sm brand" onClick={() => restartApp()}>
                <Icon name="renew" />
                立即重启
              </button>
              <span style={{ fontSize: 12, color: 'var(--text-3)' }}>稍后重启也可以，下次打开就是新版本</span>
            </div>
          )}
        </div>
      )}

      {install.kind === 'failed' && (
        <div className="up-card">
          <div className="up-err">
            <Icon name="alert" />
            {install.error}
          </div>
          <div className="lh-actions">
            <button type="button" className="btn sm" onClick={checkNow}>
              <Icon name="renew" />
              重新检查
            </button>
            <span style={{ fontSize: 12, color: 'var(--text-3)' }}>当前版本不受影响，可以稍后再试</span>
          </div>
        </div>
      )}

      <div className="set-row">
        <div className="set-label">
          <b>启动后自动检查</b>
          <span>打开 Board 时检查一次，之后每 30 分钟检查一次；发现新版本时用应用内的确认框问，不打断正在做的事</span>
        </div>
        <div className="set-control">
          <Toggle
            on={auto}
            label="启动后自动检查更新"
            onChange={v => {
              setAuto(v);
              setAutoCheckUpdate(v);
              if (v) resumeAutoUpdateCheck();
              else pauseAutoUpdateCheck();
              toast({ type: 'info', title: v ? '启动后会自动检查更新' : '启动后不再自动检查更新', duration: 1800 });
            }}
          />
        </div>
      </div>

      <div className="set-row">
        <div className="set-label">
          <b>发现新版本后自动下载并安装</b>
          <span>
            {auto
              ? '自动检查发现新版本时不再弹确认框，直接下载、校验签名并安装，装好后重启 Board。重启会结束这台机器上正在跑的会话，默认关闭'
              : '要先打开「启动后自动检查」：自动检查发现新版本时才会直接下载并安装'}
          </span>
        </div>
        <div className="set-control">
          <Toggle
            on={autoInstall}
            disabled={!auto}
            label="发现新版本后自动下载并安装"
            onChange={v => {
              setAutoInstall(v);
              setAutoInstallUpdate(v);
              toast({ type: 'info', title: v ? '发现新版本后会自动下载并安装' : '发现新版本后先问再装', duration: 1800 });
            }}
          />
        </div>
      </div>
    </>
  );
}
