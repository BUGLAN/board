/* 机器详情：居中弹窗（概况 / agent / 设置，见 HostDetail.tsx）；添加机器 */
import { useEffect, useLayoutEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { hostNameError, hostsOf, installCommand } from '../../lib/hosts';
import { canHost, isWindows } from '../../lib/platform';
import { copyText, cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { openHost, toast, toastError, useUI } from '../../store/ui';
import { Segmented, Select, Tip } from '../bits';
import { Icon } from '../Icon';
import { usePollHosts } from './bits';
import { HostDetail, type DetailTab } from './HostDetail';
import { LocalHostAdd } from './LocalHost';

export function HostModal() {
  const sel = useUI(s => s.host);
  const want = useUI(s => s.hostTab);
  const ws = useData(s => s.ws);
  const hosts = hostsOf(ws);
  const [last, setLast] = useState<string | null>(sel);
  useEffect(() => {
    if (sel) setLast(sel);
  }, [sel]);
  // 机器被移除（或改名后不在了）：关掉
  useEffect(() => {
    if (sel && sel !== 'new' && ws && !hosts.some(h => h.name === sel)) openHost(null);
  }, [sel, ws, hosts]);
  const on = !!sel;
  const id = sel ?? last;
  // 页签：换机器时留着（几台机器对着看同一页），关掉弹窗后下次从「概况」开始
  const [tab, setDetailTab] = useState<DetailTab>('overview');
  useEffect(() => {
    if (!sel) setDetailTab('overview');
  }, [sel]);
  // 指定了页签（从供应商卡片点进来）：到那一页，用过就清掉
  useEffect(() => {
    if (sel && want) {
      setDetailTab(want);
      useUI.setState({ hostTab: null });
    }
  }, [sel, want]);
  const i = hosts.findIndex(x => x.name === id);
  const close = () => openHost(null);
  const step = (d: number) => {
    const next = hosts[i + d];
    if (next) openHost(next.name);
  };
  usePollHosts(on);
  // 机器详情开着时锁住背景页面：手机上滑到头不会带着后面的页面一起滚（default/main#384）。
  // .app 自己是 100dvh 的网格、里面的 .main 才是滚动的那一层，固定 body 不解决问题，所以锁的是 overflow。
  // 只加 overflow 的话滚动位置会保留，页面不会跳；关掉（和卸载）时把原来的值放回去
  useLayoutEffect(() => {
    if (!on) return;
    const root = document.documentElement;
    const body = document.body;
    const prev = { root: root.style.overflow, body: body.style.overflow, x: root.style.overscrollBehavior };
    root.style.overflow = 'hidden';
    body.style.overflow = 'hidden';
    root.style.overscrollBehavior = 'none';
    return () => {
      root.style.overflow = prev.root;
      body.style.overflow = prev.body;
      root.style.overscrollBehavior = prev.x;
    };
  }, [on]);
  useEffect(() => {
    if (!on) return;
    const key = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest?.('input, textarea, [contenteditable]')) return;
      const ui = useUI.getState();
      if (ui.confirm || ui.palette.open || ui.ctx || ui.session) return;
      // 选文件夹的那一层盖在上面：键盘归它
      if (document.querySelector('.hd-sheet')) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      // 焦点在选择框上：上下键是打开它的选项，不是换机器
      if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && (e.target as HTMLElement).closest?.('.sel')) return;
      if (e.key === 'Escape') close();
      else if (id !== 'new' && (e.key === 'ArrowUp' || e.key === 'k')) step(-1);
      else if (id !== 'new' && (e.key === 'ArrowDown' || e.key === 'j')) step(1);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  });
  const h = i >= 0 ? hosts[i] : null;
  return createPortal(
    <>
      <div className={cx('scrim', on && 'on')} onClick={close} />
      <aside className={cx('drawer hp-modal', on && 'on')} role="dialog" aria-modal="true" aria-hidden={!on} aria-label={id === 'new' ? '添加机器' : h?.name ?? '机器'}>
        {id === 'new' ? <AddHostBody close={close} /> : h && <HostDetail key={h.id} h={h} i={i} n={hosts.length} step={step} close={close} on={on} tab={tab} setTab={setDetailTab} />}
      </aside>
    </>,
    document.body,
  );
}

function AddHostBody({ close }: { close: () => void }) {
  return (
    <div className="drawer-body">
      <div className="d-head">
        <span className="hp-m-head">添加机器</span>
        <span className="spacer" />
        <Tip text="关闭  esc">
          <button aria-label="关闭" className="icon-btn" type="button" data-tip-align="end" onClick={close}>
            <Icon name="close" />
          </button>
        </Tip>
      </div>
      <div className="d-grid hp-single">
        <div className="d-main">
          <section className="d-section">
            <h2 className="hp-m-title">在要干活的机器上运行 board host</h2>
            <div className="hp-m-meta">
              <span>运行起来后它会出现在机器列表里；派给它的卡片在对应目录里启动本机的 agent</span>
            </div>
          </section>
          <section className="d-section">
            <div className="d-label">{canHost ? '这台电脑' : '装了桌面端的电脑'}</div>
            {canHost ? (
              <LocalHostAdd close={close} />
            ) : (
              <p className="hp-note">在那台电脑的 Board 桌面端里打开「机器 → 添加机器」，在「这台电脑」填好机器名和工作空间，点「开始接任务」；访问令牌自动建好。</p>
            )}
          </section>
          <CliInstall />
        </div>
      </div>
    </div>
  );
}

/**
 * 只有命令行的机器：填机器名和工作空间，「生成命令」时以机器名新建一个访问令牌，给出一行命令——
 * 在新机器上执行就依次安装 board、board login、board host --install（装成服务，开机自动接任务）。
 * 令牌原文只在这里显示这一次；生成之后再改机器名、工作空间，命令跟着变，令牌不变。
 * 「上线后照着配」选了一台已有的机器：记在这个令牌上，新机器第一次上线时照那台当时的设置配（模型、同步开关、会话数、接单时段；
 * 工作空间、接单方式不复制）；生成之后不能再改
 */
function CliInstall() {
  // 就是这个页面的服务端地址（网页端、桌面端都一样，不写死地址）
  const origin = typeof location !== 'undefined' ? location.origin : '<服务端地址>';
  const [os, setOs] = useState<'unix' | 'windows'>(isWindows ? 'windows' : 'unix');
  const [name, setName] = useState('');
  const [workspace, setWorkspace] = useState('');
  const [copyFrom, setCopyFrom] = useState('');
  const others = hostsOf(useData(s => s.ws));
  const src = others.some(x => x.name === copyFrom) ? copyFrom : '';
  const [token, setToken] = useState<{ raw: string; name: string; from: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const nameErr = hostNameError(name);
  const wsErr = !workspace.trim() ? '先填工作空间：每个项目的目录是 <工作空间>/<项目>' : null;
  const err = nameErr ?? wsErr;
  const cmd = token && !err ? installCommand({ origin, token: token.raw, name, workspace, os }) : null;
  const copy = async (t: string) => {
    if (await copyText(t)) toast({ type: 'success', title: '已复制', text: '在新机器的终端里粘贴执行', duration: 1800 });
  };
  const generate = async () => {
    if (err || busy) return;
    setBusy(true);
    try {
      const r = await optimistic(null, () => api.createToken(name, src || undefined));
      setToken({ raw: r.token, name: r.info.name, from: src });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="d-section">
      <div className="d-label">只有命令行的机器（服务器、devbox）</div>
      <p className="hp-note">填好机器名和工作空间，生成一行命令，在那台机器上执行：安装 board、登录、装成服务（开机自动接任务）。</p>
      <div className="hp-add">
        <div className="hp-add-row">
          <span className="hp-add-k">系统</span>
          <Segmented
            value={os}
            options={[
              ['unix', 'macOS / Linux'],
              ['windows', 'Windows'],
            ]}
            onChange={setOs}
          />
        </div>
        <label className="hp-add-row">
          <span className="hp-add-k">机器名</span>
          <input
            className="input mono"
            value={name}
            placeholder="devbox"
            spellCheck={false}
            autoComplete="off"
            onChange={e => setName(e.target.value.toLowerCase().trim())}
          />
        </label>
        <label className="hp-add-row">
          <span className="hp-add-k">工作空间</span>
          <input
            className="input mono"
            value={workspace}
            placeholder={os === 'windows' ? 'C:\\repo' : '~/repo'}
            spellCheck={false}
            autoComplete="off"
            onChange={e => setWorkspace(e.target.value)}
          />
        </label>
        <p className="hp-note hp-add-note">
          一台机器只填这一个目录：每个项目的目录是 <code>{workspace.trim() ? `${workspace.trim().replace(/[\\/]+$/, '')}/${'<项目>'}` : '<工作空间>/<项目>'}</code>
          ，项目下的几块白板共用它；还没有时 board host 自动建好（项目设了仓库地址时自动克隆）。之后新加的项目也一样，不用再到这台机器上配。
        </p>
        {others.length > 0 && (
          <div className="hp-add-row">
            <span className="hp-add-k">照着配</span>
            <Select
              value={token ? token.from : src}
              options={[{ value: '', label: '不用：按默认设置' }, ...others.map(x => ({ value: x.name, label: `上线后照 ${x.name} 配` }))]}
              onChange={setCopyFrom}
              disabled={!!token}
              aria-label="上线后照哪台机器配"
            />
          </div>
        )}
        {(token ? token.from : src) && (
          <p className="hp-note hp-add-note">
            第一次上线时照 {token ? token.from : src} 当时的设置配：每个 agent 的模型、skills / MCP / Git 凭据的同步开关、会话数、接单时段；工作空间、接单方式（新机器先「只做派给它的」）不复制，之后两台各改各的。
          </p>
        )}
      </div>
      {!token ? (
        <div className="hp-add-go">
          <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={!!err || busy} onClick={() => void generate()}>
            <Icon name="key" />
            生成命令
          </button>
          <span className="hp-note">{err ?? `会新建一个名为「${name}」的访问令牌，命令里带着它`}</span>
        </div>
      ) : (
        <>
          {cmd ? (
            <Tip text="复制">
              <button type="button" className="hp-cmd" onClick={() => copy(cmd)}>
                <code>{cmd}</code>
                <Icon name="copy" />
              </button>
            </Tip>
          ) : (
            <p className="hp-note lh-warn">{err}</p>
          )}
          <p className="hp-note">
            令牌「{token.name}」的原文只在这里显示这一次，关掉弹窗后看不到了；不再用这台机器时在「Agent → 访问令牌」吊销它。
            {os === 'windows' ? '在 PowerShell 里执行。' : '在终端里执行（装到 ~/.local/bin）。'}执行完几分钟内，这台机器会出现在列表里并在线。
          </p>
          <p className="hp-note">
            最后一步 <code>board host --install</code> 把它装成开机自启的服务（{os === 'windows' ? '登录时运行的计划任务' : 'macOS 是 launchd，Linux 是 systemd 用户服务'}）：重启后自动上线，board host 异常退出 10 秒后自动重启，日志在 <code>~/.board/host/host.log</code>，卸载用 <code>board host --uninstall</code>。
            {os === 'unix' && ' Linux 上要在没人登录时也运行，再执行一次 loginctl enable-linger。'}
          </p>
        </>
      )}
    </section>
  );
}
