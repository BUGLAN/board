/* ==========================================================================
   设置 Tab（PRD §10.6）：账号 / 外观 / 桌面端更新 / 快捷键（agent 的接入、访问令牌在 Agent tab；让这台电脑接任务在机器 Tab「添加机器」）
   侧栏是目录，点击跳到对应部分；滚动时目录跟随
   ========================================================================== */
import { useEffect, useRef, useState, type FormEvent } from 'react';
import { api } from '../lib/api';
import {
  canSwitchServer, canUpdate, checkServer, connectServer, currentServer, getTheme, localHost, setDensity, setMotion, setTheme, shellVersion,
  type Density, type LocalHost, type Motion, type ServerInfo, type ThemePref,
} from '../lib/platform';
import { serverConfirmText } from '../lib/localHost';
import { accel, cx, pref } from '../lib/util';
import { logout, useAuth } from '../store/auth';
import { confirm, useUI } from '../store/ui';
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
            账号、界面的显示方式{canUpdate ? '、桌面端更新' : ''}，以及快捷键。agent 的接入、访问令牌、共用配置在「Agent」里；让这台电脑接任务在「机器 → 添加机器」。
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


        {/* 桌面端更新：单写在一张卡里 */}
        <section className="set-card" id="set-update" style={{ ['--i' as string]: 3 }}>
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

        <section className="set-card" id="set-shortcuts" style={{ ['--i' as string]: canUpdate ? 4 : 3 }}>
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
