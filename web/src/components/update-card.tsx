/* ==========================================================================
   设置 → 桌面端更新的卡片（桌面端）
   - 显示当前版本（外壳版本）和上次检查的结果：已是最新、发现新版本、检查失败
   - 「检查更新」手动查一次；有新版本时展开更新说明，「下载并安装」显示下载进度，装完提示重启
   - 启动后自动检查的开关（默认开）；自动检查发现新版本时也用应用内的确认框问（lib/update-runner.ts）
   单独成文件：设置页本身很大，这一块自己管检查和安装的状态
   ========================================================================== */
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { installUpdate, restartApp, shellVersion } from '../lib/platform';
import { autoCheckUpdate, downloadPercent, progressText, setAutoCheckUpdate, type InstallState, type UpdateProgress } from '../lib/update';
import { useUpdate } from '../lib/update-runner';
import { confirm, toast } from '../store/ui';
import { Markdown } from './bits';
import { Icon } from './Icon';

/** 设置里切换开关的小按钮 */
function Toggle({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} className={`switch${on ? ' on' : ''}`} onClick={() => onChange(!on)}>
      <i />
    </button>
  );
}

export function UpdateCard() {
  const check = useUpdate(s => s.check);
  const run = useUpdate(s => s.run);
  const [auto, setAuto] = useState(autoCheckUpdate);
  const [install, setInstall] = useState<InstallState>({ kind: 'idle' });
  const alive = useRef(true);
  useEffect(() => () => void (alive.current = false), []);

  const checkNow = () => {
    // 重新检查：上一次的安装结果（装好了等重启的除外）不再显示
    if (install.kind !== 'ready') setInstall({ kind: 'idle' });
    void run();
  };

  const doInstall = async (version: string) => {
    const yes = await confirm({
      title: `下载并安装 ${version}？`,
      text: '下载完会先校验签名，再安装。安装过程中会关闭 Board，装好后自动打开新版本（macOS 装好后需要重启一次）。',
      ok: '下载并安装',
      icon: 'update',
    });
    if (!yes) return;
    setInstall({ kind: 'downloading', progress: null });
    try {
      const r = await installUpdate(version, (p: UpdateProgress) => {
        if (alive.current) setInstall({ kind: 'downloading', progress: p });
      });
      if (!alive.current) return;
      setInstall({ kind: 'ready', version: r.version, restart: r.restart });
    } catch (e) {
      if (alive.current) setInstall({ kind: 'failed', error: String(e) });
    }
  };

  const version = shellVersion ?? '未知';
  let status: ReactNode;
  if (check.kind === 'checking') {
    status = (
      <span className="up-state checking">
        <Icon name="renew" />
        正在检查…
      </span>
    );
  } else if (check.kind === 'available') {
    status = (
      <span className="up-state">
        <i className="up-dot" style={{ background: 'var(--brand)' }} />
        有新版本 <b className="up-ver">{check.version}</b>
      </span>
    );
  } else if (check.kind === 'up-to-date') {
    status = (
      <span className="up-state ok">
        <Icon name="checkCircle" />
        已是最新版本
      </span>
    );
  } else if (check.kind === 'failed') {
    // 错误的原因在下一行（up-err-line），这里只说结果：长地址不挤占这一行的宽度
    status = (
      <span className="up-state">
        <Icon name="alert" />
        检查失败
      </span>
    );
  } else {
    status = <span className="up-state">还没有检查过</span>;
  }

  const pct = install.kind === 'downloading' ? downloadPercent(install.progress) : null;

  return (
    <>
      <div className="set-row up-row">
        <div className="set-label">
          <b>桌面端更新</b>
          <span>
            当前版本 <code className="up-ver">{version}</code>
            {shellVersion ? '' : '（页面没拿到外壳版本，更新由桌面端外壳处理）'}
          </span>
        </div>
        <div className="set-control up-state-wrap">
          {status}
          <button type="button" className={`btn sm${check.kind === 'checking' ? ' is-busy' : ''}`} disabled={check.kind === 'checking'} onClick={checkNow}>
            <Icon name="renew" />
            检查更新
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
              <Markdown src={check.result.notes} />
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
            {install.restart ? '重启 Board 后就是新版本了。' : '安装器马上会关闭 Board 并打开新版本。'}
          </p>
          <div className="lh-actions">
            <button type="button" className="btn sm brand" onClick={() => restartApp()}>
              <Icon name="renew" />
              立即重启
            </button>
            <span style={{ fontSize: 12, color: 'var(--text-3)' }}>稍后重启也可以，下次打开就是新版本</span>
          </div>
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
          <span>打开 Board 后隔几秒检查一次；发现新版本时用应用内的确认框问，不打断正在做的事</span>
        </div>
        <div className="set-control">
          <Toggle
            on={auto}
            label="启动后自动检查更新"
            onChange={v => {
              setAuto(v);
              setAutoCheckUpdate(v);
              toast({ type: 'info', title: v ? '启动后会自动检查更新' : '启动后不再自动检查更新', duration: 1800 });
            }}
          />
        </div>
      </div>
    </>
  );
}
