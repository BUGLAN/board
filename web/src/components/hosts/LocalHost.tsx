/* ==========================================================================
   这台电脑接任务（只在桌面端；default/main#396）：以前在「设置 → 本机」，现在都在机器 Tab
   - LocalHostAdd：「添加机器」弹窗里的「这台电脑」。填机器名（默认本机主机名）、工作空间（系统的文件夹选择框），
     点「开始接任务」：先用应用内的确认框写明机器名、工作空间和免审批运行的风险（内容和以前一样，localHost.ts），
     确认后配置文件里没有服务端地址和令牌就以机器名新建一个访问令牌、写进 ~/.board/config.toml（和「一键写入」
     同一条路），已经有就直接用，再打开接任务。已经在接任务时显示状态、最近的输出，可以停下或打开机器详情
   - LocalHostPanel：本机的机器详情「设置」页最上面一节（别的机器没有）：board host 的状态和最近的输出、
     改机器名（服务端原地改名，只能在本机的桌面端里改）、开机时自动启动、停止接任务。工作空间在下面「工作空间」一节改
   - 网页端、旧版桌面端（外壳没有这些命令）：不显示，添加机器里只有命令行那种方式
   ========================================================================== */
import { useEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { hostNameError, hostsOf } from '../../lib/hosts';
import { hostConfirmText } from '../../lib/localHost';
import { canHost, enableLocalHost, isWindows, localHost, pickFolder, saveLocalHost, setAutostart, setBoardEnv, type LocalHost } from '../../lib/platform';
import type { HostInfo } from '../../lib/types';
import { cx, isImeKey } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast } from '../../store/ui';
import { Icon } from '../Icon';

/** 本机 board host 的现状，开着时每 4 秒看一次；外壳不支持时 old 为 true */
export function useLocalHost(on: boolean) {
  const [st, setSt] = useState<LocalHost | null>(null);
  const [old, setOld] = useState(!canHost);
  const alive = useRef(true);
  const load = async () => {
    if (!canHost) return;
    const s = await localHost();
    if (!alive.current) return;
    if (!s) setOld(true);
    else setSt(s);
  };
  useEffect(() => {
    alive.current = true;
    if (!on || !canHost) return;
    void load();
    const t = window.setInterval(load, 4000);
    return () => {
      alive.current = false;
      clearInterval(t);
    };
  }, [on]); // eslint-disable-line react-hooks/exhaustive-deps
  return { st, old, load };
}

/** 这台电脑的机器名：配置里没写时用本机主机名（和 board host 启动时一致） */
export const localName = (st: LocalHost) => st.name.trim() || st.defaultName;

/** 一句状态：在线 / 正在连接服务端 / 已停止（退出码）/ 没在接任务 */
function stateWords(st: LocalHost, online: boolean) {
  if (!st.enabled) return '没在接任务';
  if (!st.running) return `已停止${st.exit != null ? `（退出码 ${st.exit}）` : ''}：看下面的输出`;
  return online ? `在线：${localName(st)}` : '正在连接服务端…';
}

function Log({ st }: { st: LocalHost }) {
  if (!st.log.length) return null;
  return (
    <details className="errcodes lh-log">
      <summary>board host 的输出</summary>
      <pre className="hp-log">{st.log.join('\n')}</pre>
    </details>
  );
}

/** 停止接任务（关掉不用确认） */
async function stopLocal(st: LocalHost) {
  try {
    await enableLocalHost(false, st.name, st.workspace ?? '');
    toast({ type: 'success', title: '这台电脑不再接任务', text: '已停止 board host' });
  } catch (e) {
    toast({ type: 'error', title: '没有停下', text: String(e) });
  }
}

/* ------------------------------ 添加机器 → 这台电脑 ------------------------------ */

export function LocalHostAdd({ close }: { close: () => void }) {
  const ws = useData(s => s.ws);
  const { st, old, load } = useLocalHost(true);
  const [name, setName] = useState<string | null>(null);
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [noPick, setNoPick] = useState(false);
  if (old) return null;
  if (!st) return <p className="hp-note">正在读取这台电脑…</p>;
  const n = name ?? st.name;
  const w = workspace ?? st.workspace ?? '';
  const me = n.trim() || st.defaultName;
  const online = hostsOf(ws).find(h => h.name === localName(st) && h.online);

  if (st.enabled) {
    return (
      <div className="lh-add">
        <p className="hp-note">
          这台电脑已经在接任务 · {stateWords(st, !!online)}
          {st.workspace ? (
            <>
              {' '}
              · 工作空间 <code>{st.workspace}</code>
            </>
          ) : null}
        </p>
        <div className="lh-add-go">
          {online && (
            <button
              className="btn sm"
              type="button"
              onClick={() => {
                close();
                openHost(online.name, 'settings');
              }}
            >
              <Icon name="host" />
              打开 {online.name}
            </button>
          )}
          <button
            className={cx('btn sm ghost', busy && 'is-busy')}
            type="button"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              await stopLocal(st);
              await load();
              setBusy(false);
            }}
          >
            停止接任务
          </button>
        </div>
        <Log st={st} />
      </div>
    );
  }

  const err = hostNameError(me) ?? (!w.trim() ? '先填工作空间：每个项目的目录是 <工作空间>/<项目>' : null) ?? (!st.cli ? '没找到桌面端自带的 board 命令行，重新安装桌面端试试' : null);
  const pick = async () => {
    try {
      const p = await pickFolder(w.trim());
      if (p) setWorkspace(p);
    } catch (e) {
      if (e === 'unsupported') setNoPick(true);
      else toast({ type: 'error', title: '没有打开文件夹选择框', text: String(e) });
    }
  };
  const start = async () => {
    if (err || busy) return;
    // 确认框和以前「设置 → 本机」打开开关时一样：机器名、工作空间、免审批运行的风险
    const yes = await confirm({ title: '让这台电脑开始接任务？', text: hostConfirmText(n, st.defaultName, w), ok: '开始接任务', icon: 'alert', tone: 'warn' });
    if (!yes) return;
    setBusy(true);
    try {
      // 配置文件里没有服务端地址和令牌、或者这是一台还没登记过的新机器：以机器名新建一个绑定这个机器名的
      // 机器令牌写进去（「一键写入」同一条路）。普通访问令牌登记不了新机器（#507）；已经登记过的机器直接用原来的令牌
      let made = '';
      if (!st.loggedIn || !hostsOf(ws).some(h => h.name === me)) {
        const r = await optimistic(null, () => api.createToken(me, { host: me }));
        await setBoardEnv(r.token);
        made = r.info.name;
      }
      await enableLocalHost(true, n.trim(), w.trim());
      toast({
        type: 'success',
        title: '这台电脑开始接任务',
        text: `${made ? `新建了访问令牌「${made}」并写进 ~/.board/config.toml。` : ''}桌面端在后台运行 board host，关掉窗口也继续接；开机时也会自动启动`,
      });
      setName(null);
      setWorkspace(null);
      await load();
    } catch (e) {
      toast({ type: 'error', title: '没有开始接任务', text: e === 'unsupported' ? '这个版本的桌面端写不了配置文件：升级桌面端后再试' : String(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="lh-add">
      <p className="hp-note">填好机器名和工作空间，点「开始接任务」：桌面端在后台运行 board host，关掉窗口也继续接任务，退出桌面端时结束。</p>
      <div className="hp-add">
        <label className="hp-add-row">
          <span className="hp-add-k">机器名</span>
          <input className="input mono" value={n} placeholder={st.defaultName} spellCheck={false} autoComplete="off" onChange={e => setName(e.target.value.toLowerCase().trim())} />
        </label>
        <div className="hp-add-row">
          <span className="hp-add-k">工作空间</span>
          <span className="lh-ws">
            <input
              className="input mono lh-ws-path"
              value={w}
              placeholder={isWindows ? 'C:\\repo' : '~/repo'}
              spellCheck={false}
              autoComplete="off"
              aria-label="工作空间"
              onChange={e => setWorkspace(e.target.value)}
            />
            {!noPick && (
              <button className="icon-btn sm" type="button" aria-label="选择文件夹" title="选择文件夹" onClick={() => void pick()}>
                <Icon name="folder" />
              </button>
            )}
          </span>
        </div>
        <p className="hp-note hp-add-note">
          每个项目的目录是 <code>{w.trim() ? `${w.trim().replace(/[\\/]+$/, '')}/<项目>` : '<工作空间>/<项目>'}</code>，还没有时自动建好（项目设了仓库地址时自动克隆）。
          {st.loggedIn ? '用配置文件里已有的服务端地址和令牌。' : `会以「${me}」新建一个访问令牌，写进 ~/.board/config.toml。`}
        </p>
        <div className="hp-add-go">
          <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={!!err || busy} onClick={() => void start()}>
            <Icon name="play" />
            开始接任务
          </button>
          {err && <span className="hp-note">{err}</span>}
        </div>
        <Log st={st} />
      </div>
    </div>
  );
}

/* ------------------------------ 本机的机器详情 ------------------------------ */

/** 这台机器就是本机桌面端在跑的 board host（配置里的机器名对得上、接任务开着）：只有这时显示 LocalHostPanel */
export const isThisComputer = (h: HostInfo, st: LocalHost | null) => !!st?.enabled && localName(st) === h.name;

export function LocalHostPanel({ h }: { h: HostInfo }) {
  const { st, load } = useLocalHost(canHost);
  const [name, setName] = useState<string | null>(null);
  const [busy, setBusy] = useState('');
  const [auto, setAuto] = useState<boolean | null>(null);
  if (!st || !isThisComputer(h, st)) return null;
  const n = name ?? h.name;
  const autoOn = auto ?? !!st.autostart;
  const rename = async () => {
    const next = n.trim();
    if (!next || next === h.name) return setName(null);
    const bad = hostNameError(next);
    if (bad) return toast({ type: 'error', title: '没有改名', text: bad });
    setBusy('name');
    try {
      // 服务端先把机器记录原地改名（设置和历史派发记录保留），成功了再写配置、通知 board host 重新读
      await saveLocalHost(next, st.workspace ?? '');
      toast({ type: 'success', title: `改名为 ${next}`, text: '写进了 ~/.board/config.toml，board host 不用重启就换上新名字' });
      setName(null);
      await load();
      openHost(next, 'settings');
    } catch (e) {
      toast({ type: 'error', title: '没有改名', text: String(e) });
    } finally {
      setBusy('');
    }
  };
  const toggleAuto = async () => {
    const next = !autoOn;
    setAuto(next);
    try {
      setAuto(await setAutostart(next));
    } catch (e) {
      setAuto(!next);
      toast({ type: 'error', title: next ? '没有打开开机自启' : '没有关掉开机自启', text: String(e) });
    }
  };
  return (
    <section className="lh-panel">
      <h3>这台电脑</h3>
      <div className="hd-maint">
        <div className="hd-mrow">
          <Icon name="monitor" />
          <div>
            <b>接任务</b>
            <span>{stateWords(st, h.online)} · 桌面端在后台运行 board host，关掉窗口也继续接，退出桌面端时结束</span>
          </div>
          <button
            className={cx('btn sm', busy === 'stop' && 'is-busy')}
            type="button"
            disabled={!!busy}
            onClick={async () => {
              setBusy('stop');
              await stopLocal(st);
              await load();
              setBusy('');
            }}
          >
            停止接任务
          </button>
        </div>
        <div className="hd-mrow lh-name-row">
          <Icon name="edit" />
          <div>
            <b>机器名</b>
            <span>只能在这台电脑的桌面端里改：服务端原地改名，设置和派发记录都保留</span>
          </div>
          <span className="lh-name-ctl">
            <input
              className="input mono lh-name"
              value={n}
              spellCheck={false}
              aria-label="机器名"
              onChange={e => setName(e.target.value.toLowerCase().trim())}
              onKeyDown={e => !isImeKey(e) && e.key === 'Enter' && void rename()}
            />
            <button className={cx('btn sm', busy === 'name' && 'is-busy')} type="button" disabled={!!busy || n.trim() === h.name} onClick={() => void rename()}>
              改名
            </button>
          </span>
        </div>
        {st.autostart !== undefined && (
          <div className="hd-mrow">
            <Icon name="replay" />
            <div>
              <b>开机时自动启动</b>
              <span>登录系统时在后台启动桌面端：不弹窗口，只显示托盘 / Dock 图标，照常接任务</span>
            </div>
            <button className={cx('toggle', autoOn && 'on')} type="button" role="switch" aria-checked={autoOn} aria-label="开机时自动启动" onClick={() => void toggleAuto()}>
              <i />
            </button>
          </div>
        )}
      </div>
      <Log st={st} />
    </section>
  );
}
