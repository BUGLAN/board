/* ==========================================================================
   设置 Tab（PRD §10.6）：账号 / 外观 / 本机 / 桌面端更新 / 快捷键（agent 的接入、访问令牌在 Agent tab）
   侧栏是目录，点击跳到对应部分；滚动时目录跟随
   ========================================================================== */
import { useEffect, useRef, useState, type FormEvent } from 'react';
import { api } from '../lib/api';
import {
  canHost, canSwitchServer, canUpdate, checkServer, connectServer, currentServer, enableLocalHost, getTheme, isWindows, localHost, pickFolder,
  saveLocalHost, setAutostart, setDensity, setMotion, setTheme, shellVersion, type Density, type LocalHost, type Motion, type ServerInfo, type ThemePref,
} from '../lib/platform';
import { hostsOf } from '../lib/hosts';
import { hostConfirmText, serverConfirmText } from '../lib/localHost';
import { accel, cx, pref } from '../lib/util';
import { logout, useAuth } from '../store/auth';
import { refresh, useData } from '../store/data';
import { confirm, openHost, setTab, toast, useUI } from '../store/ui';
import { Segmented, SetRow as Row } from './bits';
import { Icon } from './Icon';
import { SECTIONS, useSpy } from './SettingsNav';
import { UpdateCard } from './update-card';

function Account() {
  const me = useAuth(s => s.me);
  if (me?.mode !== 'github') {
    return (
      <Row label="本机模式" hint="服务端以 --local 运行：只接受本机访问，不需要登录">
        <span className="ro">不需要登录</span>
      </Row>
    );
  }
  return (
    <div className="set-row">
      {me.avatar && <img className="acct-avatar" src={me.avatar} alt="" />}
      <div className="set-label">
        <b>{me.name || me.login}</b>
        <span>GitHub · {me.login}</span>
      </div>
      <div className="set-control">
        <button
          type="button"
          className="btn sm"
          onClick={async () => {
            const ok = await confirm({ title: '退出登录？', text: '退出后需要重新用 GitHub 登录。agent 的访问令牌不受影响。', ok: '退出登录', icon: 'logout' });
            if (ok) logout();
          }}
        >
          <Icon name="logout" />
          退出登录
        </button>
      </div>
    </div>
  );
}

/* ---------------- 切换服务端（桌面端） ---------------- */

/** 地址来自哪里，给人看的一句话 */
function serverFromHint(s: ServerInfo): string {
  switch (s.from) {
    case 'env':
      return '来自环境变量 BOARD_URL（会盖过配置文件：改了也只写配置文件，重新打开桌面端后生效）';
    case 'config':
      return `来自配置文件 ${s.target} 的 url（和 board 命令行同一项）`;
    case 'default':
      return '来自构建时注入的默认地址';
    default:
      return '是这个桌面端这次连上的地址';
  }
}

/** 设置 → 账号 · 切换服务端：填新地址 → 先校验是 Board 服务端 → 写进配置文件 → 桌面端按新地址重新打开。
    本机在跑 board host 时先确认（它会按新地址重启，正在跑的 agent 会结束） */
function SwitchServer() {
  const [cur, setCur] = useState<ServerInfo | null>(null);
  const [host, setHost] = useState<LocalHost | null>(null);
  const [open, setOpen] = useState(false);
  const [raw, setRaw] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void currentServer().then(s => alive && setCur(s));
    void localHost().then(s => alive && setHost(s));
    return () => {
      alive = false;
    };
  }, []);

  const run = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    setError(null);
    setNote(null);
    setBusy(true);
    try {
      // 先只校验：不是 Board 服务端时在这里报错，什么都不写
      const checked = await checkServer(raw);
      const text = serverConfirmText(cur?.url || location.origin, checked.url, checked.version, checked.target, !!host?.running);
      const ok = await confirm({ title: '切换到新的服务端？', text, ok: '切换', icon: 'host' });
      if (!ok) return;
      const r = await connectServer(checked.url);
      // 正常情况：外壳已经关掉窗口、按新地址重新打开，这里不会再执行到
      setNote(r.restart ? `地址已写进 ${checked.target}：重新打开桌面端后按 ${r.url} 连接` : `已切换到 ${r.url}`);
      setOpen(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  if (!cur) return null;
  return (
    <div className="set-row srv-row">
      <div className="set-label">
        <b>服务端</b>
        <span>
          {cur.url}
          {cur.version ? ` · Board ${cur.version}` : ''}
          {cur.local ? ' · 本机' : ''}；{serverFromHint(cur)}
        </span>
      </div>
      <div className="set-control">
        <button type="button" className="btn sm" aria-expanded={open} onClick={() => setOpen(v => !v)}>
          <Icon name="monitor" />
          切换服务端
        </button>
      </div>
      {open && (
        <form className="srv-form" onSubmit={e => void run(e)}>
          <label htmlFor="srv-url">新服务端地址</label>
          <input
            id="srv-url"
            className="input"
            value={raw}
            placeholder={cur.url || 'https://board.example.com'}
            spellCheck={false}
            autoComplete="off"
            inputMode="url"
            onChange={e => {
              setRaw(e.target.value);
              setError(null);
            }}
          />
          <p className="env-cmd-note">
            先请求 <code>{'{地址}'}/api/health</code>，确认对方是 Board 服务端才写进 <code>{cur.target}</code>。本机跑的填 <code>http://127.0.0.1:7420</code>。
          </p>
          {error && <p className="srv-err" role="alert">{error}</p>}
          {note && <p className="srv-ok" role="status">{note}</p>}
          <div className="lh-actions">
            <button type="button" className="btn sm ghost" onClick={() => setOpen(false)}>取消</button>
            <button type="submit" className={cx('btn sm brand', busy && 'is-busy')} disabled={busy || !raw.trim()}>
              {busy ? '正在校验…' : '切换'}
            </button>
          </div>
        </form>
      )}
    </div>
  );
}

/* ---------------- 本机：让这台电脑接任务（桌面端） ---------------- */

function LocalHostCard() {
  const tab = useUI(s => s.tab);
  const ws = useData(s => s.ws);
  const [st, setSt] = useState<LocalHost | null>(null);
  const [old, setOld] = useState(false);
  const [name, setName] = useState('');
  /** 工作空间：每个项目的目录是 `<工作空间>/<项目>` */
  const [workspace, setWorkspace] = useState('');
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  /** 外壳不支持选择文件夹（旧版桌面端）：点过一次后不再显示按钮 */
  const [noPick, setNoPick] = useState(false);
  const [picking, setPicking] = useState(false);
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  const load = async () => {
    const s = await localHost();
    if (!s) {
      setOld(true);
      return;
    }
    setSt(s);
    if (!dirtyRef.current) {
      setName(s.name);
      setWorkspace(s.workspace ?? '');
    }
  };
  // 设置页打开时每 4 秒看一次 board host 还在不在跑
  useEffect(() => {
    if (tab !== 'settings') return;
    void load();
    const t = window.setInterval(load, 4000);
    return () => clearInterval(t);
  }, [tab]); // eslint-disable-line react-hooks/exhaustive-deps

  const me = st ? (st.name || st.defaultName) : '';
  const online = hostsOf(ws).find(h => h.name === me);
  const edit = (w: string) => {
    setWorkspace(w);
    setDirty(true);
  };
  const wsTrim = () => workspace.trim();
  const sameWs = (a: string, b: string) => a.trim() === b.trim();
  /** 工作空间里已有的目录（这台电脑盘上有的）：每个项目一个 `<工作空间>/<项目>` */
  const derived = (online?.dirs ?? []).filter(d => d.reported || d.status);

  // 打开开关 / 改了工作空间要保存：先用应用内的确认框说清机器名、工作空间和免审批运行的风险，
  // 人确认之后再调外壳（确认过就不再弹系统确认框）。取消时什么都不做：不保存
  const ask = async (title: string, ok: string, w: string) =>
    confirm({
      title,
      text: hostConfirmText(name, st?.defaultName ?? '', w),
      ok,
      icon: 'alert',
      tone: 'warn',
    });

  // 弹系统的文件夹选择框，选中后填进去；取消保持原值
  const pick = async () => {
    setPicking(true);
    try {
      const p = await pickFolder(wsTrim());
      if (p) edit(p);
    } catch (e) {
      if (e === 'unsupported') {
        setNoPick(true);
        toast({ type: 'info', title: '这个版本的桌面端不支持选择文件夹', text: '升级桌面端后再用；现在可以直接在输入框里填目录' });
      } else toast({ type: 'error', title: '没有打开文件夹选择框', text: String(e) });
    } finally {
      setPicking(false);
    }
  };

  const save = async () => {
    // 正在接任务时改了工作空间：先确认（agent 会去新的目录干活），取消就不保存。
    // 保存不重启 board host：它自己读到新配置，正在跑的 agent 不受影响
    if (st?.enabled && !sameWs(st.workspace ?? '', workspace) && !(await ask('修改这台电脑的工作空间？', '保存', workspace))) return;
    setBusy(true);
    try {
      await saveLocalHost(name.trim(), wsTrim());
      setDirty(false);
      toast({
        type: 'success',
        title: '已保存',
        text: st?.enabled ? '写进了 ~/.board/config.toml，board host 不用重启就换上新设置，正在跑的 agent 不受影响' : '写进了 ~/.board/config.toml 的 [host]',
      });
      await load();
    } catch (e) {
      toast({ type: 'error', title: '没有保存', text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const toggle = async () => {
    if (!st) return;
    // 打开前确认：机器名、工作空间一起写进配置，派到这台电脑的卡片会在 `<工作空间>/<项目>` 里免审批运行
    // 打开时改了还没保存的先确认（关掉不用确认）
    if (!st.enabled && !(await ask('让这台电脑开始接任务？', '开始接任务', workspace))) return;
    setBusy(true);
    try {
      await enableLocalHost(!st.enabled, name.trim(), wsTrim());
      // 打开时把还没保存的机器名、工作空间一起存了（和确认框里看到的一致），不用再点保存
      if (!st.enabled) setDirty(false);
      toast({ type: 'success', title: st.enabled ? '这台电脑不再接任务' : '这台电脑开始接任务', text: st.enabled ? '已停止 board host' : '桌面端在后台运行 board host，关掉窗口也继续接；开机时也会自动启动' });
      await load();
      // 打开接任务时外壳一起打开了开机自启：按外壳报的现状画
      setAuto(null);
      setTimeout(() => void refresh(), 1500);
    } catch (e) {
      toast({ type: 'error', title: '没有切换', text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  // 开机时自动启动：当帧切换，失败时拨回去并提示
  const [auto, setAuto] = useState<boolean | null>(null);
  const autoOn = auto ?? !!st?.autostart;
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

  let state = '关';
  if (st?.enabled) state = st.running ? (online?.online ? `在线：${me}` : '正在连接服务端…') : `已停止${st.exit != null ? `（退出码 ${st.exit}）` : ''}：看下面的输出`;
  // 有没保存的改动：确认框里列的是输入框里的（和打开开关时一起写进配置的一致）
  const unsaved = name.trim() !== (st?.name ?? '') || !sameWs(st?.workspace ?? '', workspace);

  return (
    <section className="set-card" id="set-host" style={{ ['--i' as string]: 3 }}>
      <h2>
        <Icon name="host" />
        本机
      </h2>
      <p className="set-p">
        让这台电脑接任务：桌面端在后台用自带的 <code>board</code> 运行 <code>board host</code>，派给这台电脑的卡片在{' '}
        <code>&lt;工作空间&gt;/&lt;项目&gt;/&lt;白板&gt;</code> 里启动本机的 agent（提示词仍是「复制为提示词」那一行）。关掉窗口也继续运行，退出桌面端时结束。
      </p>
      {old ? (
        <p className="set-p">这个版本的桌面端还不支持，升级桌面端后再打开。</p>
      ) : !st ? (
        <p className="set-p">正在读取…</p>
      ) : (
        <>
          {!st.loggedIn && <p className="set-p lh-warn">还没有服务端地址和令牌：先在「Agent → 访问令牌」新建令牌并写入配置文件，board host 才连得上。</p>}
          {!st.cli && <p className="set-p lh-warn">没找到桌面端自带的 board 命令行，重新安装桌面端试试。</p>}
          <Row label="让这台电脑接任务" hint={unsaved ? `${state} · 还有没保存的改动，打开时会一起保存` : state}>
            <button className={cx('toggle', st.enabled && 'on')} type="button" role="switch" aria-checked={st.enabled} aria-label="让这台电脑接任务" disabled={busy || !st.cli} onClick={toggle}>
              <i />
            </button>
          </Row>
          {st.autostart !== undefined && (
            <Row label="开机时自动启动" hint="登录系统时在后台启动桌面端：接任务时不弹窗口，只显示托盘 / Dock 图标，照常运行 board host。打开「让这台电脑接任务」时一起打开">
              <button className={cx('toggle', autoOn && 'on')} type="button" role="switch" aria-checked={autoOn} aria-label="开机时自动启动" onClick={() => void toggleAuto()}>
                <i />
              </button>
            </Row>
          )}
          <Row label="机器名" hint="小写字母、数字、. 和 -，最长 24 个字符；不填用本机主机名">
            <input
              className="input lh-name"
              value={name}
              placeholder={st.defaultName}
              spellCheck={false}
              onChange={e => {
                setName(e.target.value.toLowerCase());
                setDirty(true);
              }}
            />
          </Row>
          <Row label="工作空间" hint="这台电脑只配这一个目录：每个项目的目录是 <工作空间>/<项目>（项目下的白板共用），派来的卡片在那里启动 agent；还没有时自动建好，项目设了仓库地址时自动克隆">
            <span className="lh-ws">
              <input
                className="input lh-ws-path"
                value={workspace}
                placeholder={isWindows ? 'C:\\repo' : '~/repo'}
                spellCheck={false}
                aria-label="工作空间"
                onChange={e => edit(e.target.value)}
              />
              {!noPick && (
                <button
                  className={cx('icon-btn sm', picking && 'is-busy')}
                  type="button"
                  aria-label="选择文件夹"
                  title="选择文件夹"
                  disabled={picking}
                  onClick={() => void pick()}
                >
                  <Icon name="folder" />
                </button>
              )}
            </span>
          </Row>
          {derived.length > 0 && (
            <div className="lh-derived">
              <span className="lh-derived-k">这台电脑上的项目目录</span>
              {/* 同一个项目的几块白板是同一个目录：一个目录一行 */}
              {derived
                .filter((d, i, a) => a.findIndex(x => x.path === d.path) === i)
                .map(d => (
                  <span className="lh-derived-row" key={d.path}>
                    <b>{d.board.split('/')[0]}</b>
                    <code>{d.path}</code>
                  </span>
                ))}
            </div>
          )}
          {!wsTrim() && <p className="set-p lh-warn">还没有填工作空间：这台电脑不接任何白板的卡。</p>}
          <div className="lh-actions">
            {online && (
              <button
                className="btn sm ghost"
                type="button"
                onClick={() => {
                  setTab('hosts');
                  openHost(online.name);
                }}
              >
                <Icon name="host" />
                在「机器」里查看
              </button>
            )}
            <span className="lh-sp" />
            <button className="btn sm brand" type="button" disabled={!dirty || busy} onClick={save}>
              保存
            </button>
          </div>
          {online?.dirsChangedAt && online.dirsChangedBy && online.dirsChangedBy !== '本机' && (
            <p className="set-p lh-warn">
              {new Date(online.dirsChangedAt).toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })} 由{' '}
              {online.dirsChangedBy} 在别的设备上修改；board host 已按服务端的设置更新本机配置。
            </p>
          )}
          {st.log.length > 0 && (
            <details className="errcodes lh-log">
              <summary>board host 的输出</summary>
              <pre className="hp-log">{st.log.join('\n')}</pre>
            </details>
          )}
        </>
      )}
    </section>
  );
}

export function SettingsPage() {
  const tab = useUI(s => s.tab);
  const page = useRef<HTMLDivElement>(null);
  const [theme, setT] = useState<ThemePref>(getTheme);
  const [density, setD] = useState<Density>(() => pref.get('density', 'comfy'));
  const [motion, setM] = useState<Motion>(() => pref.get('motion', 'full'));
  const [server, setServer] = useState<{ version: string; commit: string } | null>(null);
  const [serverFailed, setServerFailed] = useState(false);

  // 底部显示服务端版本（部署后用来核对线上是不是新版）；每次进入设置页刷新
  useEffect(() => {
    if (tab !== 'settings') return;
    let alive = true;
    api
      .info()
      .then(i => {
        if (!alive) return;
        setServer(i);
        setServerFailed(false);
      })
      .catch(() => alive && setServerFailed(true));
    return () => {
      alive = false;
    };
  }, [tab]);

  useEffect(() => {
    const el = page.current;
    if (!el) return;
    const spy = () => {
      if (useSpy.getState().lock) return;
      let cur = SECTIONS[0].id;
      for (const s of SECTIONS) {
        const n = document.getElementById(`set-${s.id}`);
        if (n && n.offsetTop - 80 <= el.scrollTop) cur = s.id;
      }
      if (el.scrollTop + el.clientHeight >= el.scrollHeight - 4) cur = SECTIONS[SECTIONS.length - 1].id;
      useSpy.setState({ active: cur });
    };
    el.addEventListener('scroll', spy, { passive: true });
    spy();
    return () => el.removeEventListener('scroll', spy);
  }, [tab]);

  return (
    <div className="page page-settings" data-page="settings" ref={page}>
      <div className="settings">
        <header className="settings-hero">
          <h1>设置</h1>
          {/* 按下面区块的顺序写 */}
          <p>
            账号、界面的显示方式{canHost ? '、让这台电脑接任务' : ''}{canUpdate ? '、桌面端更新' : ''}，以及快捷键。agent 的接入、访问令牌、共用配置在「Agent」里。
          </p>
        </header>

        <section className="set-card" id="set-account" style={{ ['--i' as string]: 1 }}>
          <h2>
            <Icon name="user" />
            账号
          </h2>
          <Account />
          {/* 桌面端：换了服务端，账号、白板都在新的服务端上 */}
          {canSwitchServer && <SwitchServer />}
        </section>

        <section className="set-card" id="set-appearance" style={{ ['--i' as string]: 2 }}>
          <h2>
            <Icon name="sun" />
            外观
          </h2>
          <Row label="主题" hint="默认跟随系统">
            <Segmented
              value={theme}
              options={[
                ['auto', '跟随系统'],
                ['light', '浅色'],
                ['dark', '深色'],
              ]}
              onChange={v => {
                setTheme(v);
                setT(v);
              }}
            />
          </Row>
          <Row label="卡片密度" hint="紧凑模式下一屏能看到更多卡片">
            <Segmented
              value={density}
              options={[
                ['comfy', '舒适'],
                ['compact', '紧凑'],
              ]}
              onChange={v => {
                setDensity(v);
                setD(v);
              }}
            />
          </Row>
          <Row label="动效" hint="减弱后关闭大部分过渡动画">
            <Segmented
              value={motion}
              options={[
                ['full', '完整'],
                ['reduced', '减弱'],
              ]}
              onChange={v => {
                setMotion(v);
                setM(v);
              }}
            />
          </Row>
        </section>

        {canHost && <LocalHostCard />}

        {/* 桌面端更新：单写在一张卡里，和「本机」分开（本机里是 board host 的设置） */}
        <section className="set-card" id="set-update" style={{ ['--i' as string]: canHost ? 4 : 3 }}>
          <h2>
            <Icon name="renew" />
            桌面端更新
          </h2>
          <p className="set-p">
            {canUpdate
              ? '发版后会发布新的桌面端：检查 GitHub Release 上最新的版本，下载的安装包先校验签名再安装。装好后重启就是新版本，正在做的事不会丢（卡片和白板都在服务端）。'
              : '在网页上打开时的版本由服务器决定，不用更新；装了桌面端之后，可以在这里检查并安装新版本。'}
          </p>
          {canUpdate ? <UpdateCard /> : <Row label="桌面端" hint={shellVersion ? `当前 ${shellVersion}` : '没有检测到桌面端外壳'}><span className="ro">{shellVersion ?? '—'}</span></Row>}
        </section>

        <section className="set-card" id="set-shortcuts" style={{ ['--i' as string]: canHost ? 5 : 4 }}>
          <h2>
            <Icon name="keyboard" />
            快捷键
          </h2>
          <div className="keys">
            {(
              [
                [`${accel('K')}  /`, '搜索 / 命令面板'],
                ['P', '打开项目 / 白板切换器'],
                [accel('1~8'), '切换到白板 / 未整理 / 里程碑 / 大盘 / 项目 / 机器 / Agent / 设置'],
                [accel('B'), '收起 / 展开侧栏'],
                ['N', '新建卡片'],
                ['I', '记一条未整理'],
                ['Esc', '关闭弹窗、面板'],
                ['↑ ↓ ← → ↵', '在卡片间移动、打开卡片'],
                ['↑ ↓  K J', '详情里：同状态上一张 / 下一张'],
                ['J  K', '里程碑里：更早的版本 / 新一点的版本'],
                ['C', '详情里：复制为提示词'],
                ['L', '详情里：加标签'],
              ] as [string, string][]
            ).map(([k, v]) => (
              <div key={v}>
                <span>
                  {k.split(/\s+/).map((x, i) => (
                    <kbd key={i}>{x}</kbd>
                  ))}
                </span>
                {v}
              </div>
            ))}
          </div>
        </section>

        <footer className="settings-foot">
          <span>{server ? `服务端 ${server.version} · ${server.commit}` : serverFailed ? '服务端版本暂不可用' : '服务端版本读取中…'}</span>
          {shellVersion && <span>桌面端 {shellVersion}</span>}
        </footer>
      </div>
    </div>
  );
}
