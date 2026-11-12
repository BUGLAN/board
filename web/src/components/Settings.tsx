/* ==========================================================================
   设置 Tab（PRD §10.6）：账号 / 外观 / 白板规则 / Agent 接入 / 访问令牌 / 本机 / 桌面端更新 / 快捷键
   侧栏是目录，点击跳到对应部分；滚动时目录跟随
   ========================================================================== */
import { useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { api } from '../lib/api';
import {
  boardCli, canHost, canSetEnv, canSwitchServer, canUpdate, checkServer, connectServer, currentServer, enableLocalHost, getTheme, isMac, isWindows, localHost, pickFolder,
  saveLocalHost, setAutostart, setBoardEnv, setDensity, setMotion, setTheme, shellVersion, type Density, type LocalHost, type Motion, type ServerInfo, type ThemePref,
} from '../lib/platform';
import { hostsOf } from '../lib/hosts';
import { hostConfirmText, serverConfirmText } from '../lib/localHost';
import { cardPrompt } from '../lib/prompt';
import { boardFlow, ERRORS } from '../lib/schema';
import type { TokenInfo } from '../lib/types';
import { accel, ago, copyText, cx, pref } from '../lib/util';
import skill from '../../../.agents/skills/board-skills/SKILL.md?raw';
import { logout, useAuth } from '../store/auth';
import { allBoards, colCount, refresh, useCurrent, useData } from '../store/data';
import { confirm, openHost, setTab, toast, toastError, useUI } from '../store/ui';
import { quickPrompts } from '../lib/prompt';
import { Segmented, Select } from './bits';
import { FlowBar } from './FlowBar';
import { Icon } from './Icon';
import { SECTIONS, useSpy } from './SettingsNav';
import { UpdateCard } from './update-card';

const Row = ({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) => (
  <div className="set-row">
    <div className="set-label">
      <b>{label}</b>
      {hint && <span>{hint}</span>}
    </div>
    <div className="set-control">{children}</div>
  </div>
);

function Code({ cmd, note, plain, ps }: { cmd: string; note?: string; plain?: boolean; ps?: boolean }) {
  return (
    <div className={cx('code-line', plain && 'plain', ps && 'ps')}>
      <code>{cmd}</code>
      {note && <span>{note}</span>}
      <CopyBtn text={cmd} />
    </div>
  );
}

function CopyBtn({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className={cx('copy', done && 'done')}
      aria-label="复制"
      onClick={async () => {
        await copyText(text);
        setDone(true);
        setTimeout(() => setDone(false), 1400);
      }}
    >
      <Icon name={done ? 'check' : 'copy'} />
    </button>
  );
}

/** 「做某状态里带某标签的任务」：一行，行里选标签（当前项目的标签）后复制 */
function LabelPrompt({ path, status, names }: { path: string; status: string; names: string[] }) {
  const labels = useCurrent()?.project.labels ?? [];
  const [pick, setPick] = useState<string | null>(null);
  const label = pick ?? labels.find(l => l.name === 'claude')?.name ?? labels[0]?.name ?? 'claude';
  const q = quickPrompts(path, names, label).find(x => x.kind === 'label' && x.status === status);
  if (!q) return null;
  return (
    <div className="code-line plain has-pick">
      <code>{q.text}</code>
      <Select
        className="code-pick"
        value={label}
        options={(labels.length ? labels.map(l => l.name) : [label]).map(n => ({ value: n, label: n }))}
        onChange={setPick}
        aria-label={`「${status}」里按哪个标签`}
      />
      <CopyBtn text={q.text} />
    </div>
  );
}

function CopySkill() {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className="btn sm"
      onClick={async () => {
        const ok = await copyText(skill);
        setDone(ok);
        toast({ type: ok ? 'success' : 'error', title: ok ? '已复制 SKILL.md' : '没能写入剪贴板', duration: 1800 });
        setTimeout(() => setDone(false), 1600);
      }}
    >
      <Icon name={done ? 'check' : 'copy'} />
      复制 SKILL.md
    </button>
  );
}

/** 新建令牌后给出的那一条命令：校验令牌后把服务端地址和令牌写进 ~/.board/config.toml（各平台相同） */
const loginCommand = (url: string, token: string) => `board login --url ${url} --token ${token}`;

/** 桌面端外壳注入的 board 命令行完整路径 → 它所在的目录 */
const cliDir = () => (boardCli ? boardCli.replace(/[\\/][^\\/]*$/, '') : null);

/** PATH 里加上 board 所在目录：只写用户级配置、重复执行不会重复追加，命令里不出现任何令牌 */
function pathCommand(kind: 'powershell' | 'sh', rc: string): string {
  const dir = cliDir();
  if (kind === 'powershell') {
    const find = dir
      ? `$d = '${dir.replace(/'/g, "''")}'`
      : `$d = Split-Path (Get-Command board -ErrorAction SilentlyContinue).Source; if (-not $d) { throw '先找到 board.exe：在桌面端打开这个页面会直接给出完整路径' }`;
    return (
      find +
      `; $p = [Environment]::GetEnvironmentVariable('Path','User'); $parts = @($p -split ';' | Where-Object { $_ }); ` +
      `if ($parts -notcontains $d) { [Environment]::SetEnvironmentVariable('Path', (($parts + $d) -join ';'), 'User'); "已把 $d 写进用户 PATH" } ` +
      `else { "$d 已经在用户 PATH 里" }; $env:Path = "$d;$env:Path"`
    );
  }
  const body =
    `f=${rc}; touch "$f"; if grep -qxF "export PATH=\\"$d:\\$PATH\\"" "$f" 2>/dev/null; then echo "已经在 $f 里，不重复追加（新开终端生效）"; ` +
    `else printf 'export PATH="%s:$PATH"\\n' "$d" >> "$f"; echo "已写入 $f（新开终端或 source $f 后生效）"; fi`;
  return dir
    ? `d="${dir}"; ${body}`
    : `d=$(dirname "$(command -v board)" 2>/dev/null); if [ -z "$d" ]; then echo '先找到 board：在桌面端打开本页会直接给出完整路径'; else ${body}; fi`;
}

function LoginCommand({ token }: { token: string }) {
  const [state, setState] = useState<{ busy?: boolean; done?: string; error?: string }>({});
  const apply = async () => {
    setState({ busy: true });
    try {
      setState({ done: await setBoardEnv(token) });
    } catch (e) {
      setState({ error: e === 'unsupported' ? '这个版本的桌面端还不支持，请更新桌面端，或复制下面的命令运行。' : `没能写入：${e}` });
    }
  };
  return (
    <div className="env-cmd">
      {canSetEnv ? (
        <div className="env-apply">
          <button type="button" className="btn sm primary" disabled={state.busy || !!state.done} onClick={apply}>
            <Icon name={state.done ? 'check' : 'terminal'} />
            {state.done ? '已写入' : '一键写入配置文件'}
          </button>
          <span role="status">
            {state.done
              ? `已写入 ${state.done}：服务端地址 ${location.origin} 和这个令牌。board 命令马上就能用，不用新开终端。`
              : state.error ?? '把服务端地址和这个令牌写进 ~/.board/config.toml；也可以复制下面的命令自己运行。'}
          </span>
        </div>
      ) : (
        <p className="env-cmd-note">网页不能直接修改你电脑上的配置，复制下面的命令在终端里运行；在 Board 桌面端可以一键写入。</p>
      )}
      <div className="code-block">
        <Code cmd={loginCommand(location.origin, token)} />
      </div>
      <p className="env-cmd-note">
        校验令牌后写进 <code>~/.board/config.toml</code>（Windows：<code>%USERPROFILE%\.board\config.toml</code>），所有终端马上能用。环境变量{' '}
        <code>BOARD_URL</code>、<code>BOARD_TOKEN</code> 会覆盖配置文件；以前写进 shell 配置文件的 <code>export</code> 要删掉，<code>board doctor</code> 会指出来。
      </p>
    </div>
  );
}

/** 访问令牌：agent 用 board 命令行连接服务端时认证。原文只在新建时显示一次 */
function Tokens() {
  const tab = useUI(s => s.tab);
  const [list, setList] = useState<TokenInfo[] | null>(null);
  const [name, setName] = useState('');
  const [fresh, setFresh] = useState<{ token: string; name: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const load = () =>
    api
      .tokens()
      .then(setList)
      .catch(() => {});
  useEffect(() => {
    if (tab === 'settings') load();
  }, [tab]);

  const create = async (e: FormEvent) => {
    e.preventDefault();
    const n = name.trim();
    if (!n || busy) return;
    setBusy(true);
    try {
      const r = await api.createToken(n);
      setFresh({ token: r.token, name: r.info.name });
      setName('');
      await load();
    } catch (err) {
      toastError(err);
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (t: TokenInfo) => {
    const ok = await confirm({
      title: `吊销令牌「${t.name}」？`,
      text: '使用这个令牌的 agent 之后会得到 INVALID_TOKEN，需要换成新的令牌。',
      ok: '吊销',
      icon: 'key',
    });
    if (!ok) return;
    // 当帧从列表里拿掉，被拒绝时重新取列表并提示
    setList(l => l?.filter(x => x.id !== t.id) ?? l);
    try {
      await api.revokeToken(t.id);
    } catch (err) {
      toastError(err);
    }
    await load();
  };

  const now = Date.now();
  return (
    <div className="tokens">
      {fresh && (
        <div className="token-fresh" role="status">
          <b>令牌「{fresh.name}」已新建。原文只显示这一次，请现在复制保存。</b>
          <LoginCommand token={fresh.token} />
          <button type="button" className="btn sm" onClick={() => setFresh(null)}>
            <Icon name="check" />
            已保存
          </button>
        </div>
      )}
      <ul className="token-list">
        {list?.length === 0 && <li className="token-empty">还没有访问令牌</li>}
        {list?.map(t => (
          <li key={t.id}>
            <Icon name="key" />
            <div className="token-main">
              <b>{t.name}</b>
              <span>
                <code>{t.hint}</code> · 创建于 {ago(t.createdAt, now)} · {t.lastUsedAt ? `最近使用 ${ago(t.lastUsedAt, now)}` : '还没用过'}
              </span>
            </div>
            <button type="button" className="btn sm danger" onClick={() => revoke(t)}>
              吊销
            </button>
          </li>
        ))}
      </ul>
      <form className="token-new" onSubmit={create}>
        <input
          className="input"
          value={name}
          onChange={e => setName(e.target.value)}
          placeholder="令牌用途，例如 macbook、ci"
          maxLength={40}
          aria-label="令牌用途"
        />
        <button type="submit" className={cx('btn sm primary', busy && 'is-busy')} disabled={!name.trim() || busy} aria-busy={busy || undefined}>
          <Icon name="plus" />
          新建令牌
        </button>
      </form>
    </div>
  );
}

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

type Dir = { board: string; path: string };

function LocalHostCard() {
  const tab = useUI(s => s.tab);
  const ws = useData(s => s.ws);
  const [st, setSt] = useState<LocalHost | null>(null);
  const [old, setOld] = useState(false);
  const [name, setName] = useState('');
  const [dirs, setDirs] = useState<Dir[]>([]);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  /** 外壳不支持选择文件夹（旧版桌面端）：点过一次后不再显示按钮 */
  const [noPick, setNoPick] = useState(false);
  /** 正在弹文件夹选择框的那一行 */
  const [picking, setPicking] = useState<number | null>(null);
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
      setDirs(s.dirs);
    }
  };
  // 设置页打开时每 4 秒看一次 board host 还在不在跑
  useEffect(() => {
    if (tab !== 'settings') return;
    void load();
    const t = window.setInterval(load, 4000);
    return () => clearInterval(t);
  }, [tab]); // eslint-disable-line react-hooks/exhaustive-deps

  const boards = allBoards(ws).map(b => b.path);
  const me = st ? (st.name || st.defaultName) : '';
  const online = hostsOf(ws).find(h => h.name === me);
  const edit = (f: (d: Dir[]) => Dir[]) => {
    setDirs(f);
    setDirty(true);
  };
  /** 表格里填的白板与目录（只留下填了一半的行，保存时也这样交给外壳） */
  const filled = () => dirs.filter(d => d.board || d.path);
  const sameDirs = (a: Dir[], b: Dir[]) =>
    a.length === b.length && a.every((d, i) => d.board.trim() === b[i].board.trim() && d.path.trim() === b[i].path.trim());

  // 打开开关 / 改了白板与目录要保存：先用应用内的确认框说清机器名、目录和免审批运行的风险，
  // 人确认之后再调外壳（确认过就不再弹系统确认框）。取消时什么都不做：不保存
  const ask = async (title: string, ok: string, ds: Dir[]) =>
    confirm({
      title,
      text: hostConfirmText(name, st?.defaultName ?? '', ds),
      ok,
      icon: 'alert',
      tone: 'warn',
    });

  // 弹系统的文件夹选择框，选中后填进这一行；取消保持原值
  const pick = async (i: number) => {
    setPicking(i);
    try {
      const p = await pickFolder(dirs[i]?.path ?? '');
      if (p) edit(v => v.map((x, j) => (j === i ? { ...x, path: p } : x)));
    } catch (e) {
      if (e === 'unsupported') {
        setNoPick(true);
        toast({ type: 'info', title: '这个版本的桌面端不支持选择文件夹', text: '升级桌面端后再用；现在可以直接在输入框里填目录' });
      } else toast({ type: 'error', title: '没有打开文件夹选择框', text: String(e) });
    } finally {
      setPicking(null);
    }
  };

  const save = async () => {
    // 正在接任务时改了白板与目录：先确认（agent 会去新目录干活），取消就不保存。
    // 保存不重启 board host：它自己读到新配置，正在跑的 agent 不受影响
    const next = filled();
    if (st?.enabled && !sameDirs(st.dirs, next) && !(await ask('修改这台电脑接任务的白板目录？', '保存', next))) return;
    setBusy(true);
    try {
      await saveLocalHost(name.trim(), filled());
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
    // 打开前确认：机器名、白板与目录一起写进配置，派到这台电脑的卡片会在这些目录里免审批运行
    // 打开时表格里改过还没保存的先确认（关掉不用确认）
    if (!st.enabled && !(await ask('让这台电脑开始接任务？', '开始接任务', filled()))) return;
    setBusy(true);
    try {
      const next = filled();
      await enableLocalHost(!st.enabled, name.trim(), next);
      // 打开时把表格里还没保存的机器名、白板与目录一起存了（和确认框里看到的一致），不用再点保存
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
  // 表格里有没保存的改动：确认框里列的是表格里的（和打开开关时一起写进配置的一致）
  const unsaved = [name.trim() !== (st?.name ?? ''), !sameDirs(st?.dirs ?? [], filled())].some(Boolean);

  return (
    <section className="set-card" id="set-host" style={{ ['--i' as string]: 6 }}>
      <h2>
        <Icon name="host" />
        本机
      </h2>
      <p className="set-p">
        让这台电脑接任务：桌面端在后台用自带的 <code>board</code> 运行 <code>board host</code>，派给这台电脑的卡片在对应目录里启动本机的 agent（提示词仍是「复制为提示词」那一行）。关掉窗口也继续运行，退出桌面端时结束。
      </p>
      {old ? (
        <p className="set-p">这个版本的桌面端还不支持，升级桌面端后再打开。</p>
      ) : !st ? (
        <p className="set-p">正在读取…</p>
      ) : (
        <>
          {!st.loggedIn && <p className="set-p lh-warn">还没有服务端地址和令牌：先在上面「Agent 接入」新建令牌并写入配置文件，board host 才连得上。</p>}
          {!st.cli && <p className="set-p lh-warn">没找到桌面端自带的 board 命令行，重新安装桌面端试试。</p>}
          <Row label="让这台电脑接任务" hint={unsaved ? `${state} · 表格里有还没保存的改动，打开时会一起保存` : state}>
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
          <div className="set-row lh-dirs-row">
            <div className="set-label">
              <b>白板与目录</b>
              <span>派到这台电脑的卡片，agent 在对应目录（这块白板的代码仓库）里启动；没有目录的白板不接</span>
            </div>
          </div>
          <div className="lh-dirs">
            {dirs.map((d, i) => (
              <div className="lh-dir" key={i}>
                <Select
                  value={d.board}
                  options={[{ value: '', label: '选白板…' }, ...[...new Set([...boards, ...(d.board ? [d.board] : [])])].map(b => ({ value: b, label: b }))]}
                  onChange={val => edit(v => v.map((x, j) => (j === i ? { ...x, board: val } : x)))}
                  aria-label="白板"
                />
                <Icon name="move" />
                <input
                  className="input"
                  value={d.path}
                  placeholder={isWindows ? 'C:\\repo\\board' : '~/repo/board'}
                  spellCheck={false}
                  onChange={e => edit(v => v.map((x, j) => (j === i ? { ...x, path: e.target.value } : x)))}
                  aria-label="目录"
                />
                {!noPick && (
                  <button
                    className={cx('icon-btn sm', picking === i && 'is-busy')}
                    type="button"
                    aria-label="选择文件夹"
                    title="选择文件夹"
                    disabled={picking !== null}
                    onClick={() => void pick(i)}
                  >
                    <Icon name="folder" />
                  </button>
                )}
                <button className="icon-btn sm" type="button" aria-label="去掉这一行" onClick={() => edit(v => v.filter((_, j) => j !== i))}>
                  <Icon name="close" />
                </button>
              </div>
            ))}
            <div className="lh-actions">
              <button className="btn sm" type="button" onClick={() => edit(v => [...v, { board: boards.find(b => !v.some(x => x.board === b)) ?? '', path: '' }])}>
                <Icon name="plus" />
                加一块白板
              </button>
              <span className="lh-sp" />
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
              <button className="btn sm brand" type="button" disabled={!dirty || busy} onClick={save}>
                保存
              </button>
            </div>
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
  const path = useData(s => s.path);
  const board = useData(s => s.board);
  const flow = boardFlow(board);
  // 和白板页工作流条一样的卡片数（终点状态含快照里省略的）
  const counts = useMemo(() => (board ? Object.fromEntries(Object.keys(board.cols).map(k => [k, colCount(board, k)])) : {}), [board]);
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
          <p>账号、界面的显示方式、白板规则、agent 如何通过命令行接入、访问令牌{canHost ? '、让这台电脑接任务' : ''}{canUpdate ? '、桌面端更新' : ''}，以及快捷键。</p>
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

        <section className="set-card" id="set-board" style={{ ['--i' as string]: 3 }}>
          <h2>
            <Icon name="lock" />
            白板规则
          </h2>
          <p className="set-p">每块白板的工作流可以在白板页顶部的工作流条里自定义。当前白板（只读，和工作流条是同一张图）：</p>
          {flow.list.length > 0 && <FlowBar counts={counts} path={path} f={flow} readOnly />}
          <Row label="心跳" hint="agent 会话 check 之后由 skill 里的脚本发送，一路心跳同时维持名字在线和它持有的卡片；领取后的第一次心跳开始计时，final 结束，每一次的耗时记进动态">
            <span className="ro mono">每 10 秒</span>
          </Row>
          <Row label="心跳中断" hint="连续 5 次没收到：名字离线；卡片（从领取或最近一次心跳算起）回到领取它时所在的可领取状态最上面，进行中的这一次记为异常退出">
            <span className="ro">50 秒后释放</span>
          </Row>
          <ul className="rules">
            <li>只有可领取状态里的卡片能被领取；领取后卡片沿连线进入锁定状态；不带 ID 时按工作流顺序先领排在前面的可领取状态；多个 agent 同时领取同一张，只有一个成功</li>
            <li>agent 只能沿工作流条上的连线移动卡片；人移动卡片不受连线限制</li>
            <li>一个 agent 在一块白板上同时只持有一张卡片</li>
            <li>进入锁定状态只能靠领取；持有者可以沿连线把自己的卡片在锁定状态之间移动，领取保持</li>
            <li>被持有的卡片只有持有者能编辑和移动，其他人只能评论、加或去标签；人可以强制释放</li>
            <li>卡片离开锁定状态时，领取一起释放；强制释放和超时释放的卡片回到领取它时所在的可领取状态最上面</li>
            <li>终点状态只归人管；删除只能在界面上做</li>
            <li>错误节点（状态设置里打开）：agent 异常退出、做不下去时沿连线移进来，不会再被领走；只有人能移出</li>
            <li>「未整理」可以由界面或 agent/CLI 新建，但只有人能整理进/出工作流：agent 不会领取，也不能把已有卡片移入或移出；整理后放进「{flow.home?.name ?? '待办'}」</li>
          </ul>
        </section>

        <section className="set-card" id="set-agents" style={{ ['--i' as string]: 4 }}>
          <h2>
            <Icon name="terminal" />
            Agent 接入
          </h2>
          <p className="set-p">
            给 agent 装上 <code>board-skills</code>，之后只需要发一句话：领取、按描述干活、记进度、交审、规则和错误码都写在 skill 里，agent 自己用{' '}
            <code>board</code> 命令行读取卡片。卡片详情里点「复制为提示词」（或按 <kbd>C</kbd>）得到的就是这一句。
          </p>
          <div className="code-block">
            <Code plain cmd={cardPrompt(path, 12)} note="处理一张卡片" />
            <Code plain cmd={`/board-skills ${path}`} note={`领完这块白板可领取状态里的卡片（${flow.claimables.map(s => `「${s.name}」`).join('') || '待办'}）`} />
            <Code plain cmd="/board-skills README 加一个英文版" note={`一句话需求：先记到「${flow.home?.name ?? '待办'}」再做`} />
          </div>
          <p className="set-p">快捷提示词：白板页头点「提示词」一键复制。</p>
          <div className="code-block">
            {quickPrompts(path, flow.claimables.map(s => s.name)).map(q => (
              <Code key={q.id} plain cmd={q.text} note={q.title} />
            ))}
            {/* 带标签的：每个可领取状态一行，在行里选标签后复制 */}
            {flow.claimables.map(s => (
              <LabelPrompt key={s.key} path={path} status={s.name} names={flow.claimables.map(x => x.name)} />
            ))}
          </div>
          <Row
            label="安装 skill"
            hint="skill 是 ~/.agents/skills/board-skills/ 整个目录（Claude Code 放在 ~/.claude/skills/board-skills/）：入口 SKILL.md、按场景的 work.md、plan.md、host.md、reference.md，自带的 board 在 scripts/、bin/ 里。只复制 SKILL.md 不够，解压发版附件 board-skills.zip，或用下面的命令链接整个目录"
          >
            <CopySkill />
          </Row>
          <div className="code-block">
            <Code cmd={'ln -s "$PWD/.agents/skills/board-skills" ~/.agents/skills/board-skills'} note="在 Board 仓库里执行：链接到 skills 目录" />
          </div>
          <h3 className="set-sub">
            <Icon name="terminal" />
            安装 board 命令行
          </h3>
          <p className="set-p">
            {isWindows
              ? '装了 Windows 桌面端就不需要 Rust：安装包自带的 board.exe 会写进用户 PATH。如果新开的终端还是 command not found，把下面这条命令粘进 PowerShell 执行一次。'
              : isMac
                ? '装了 macOS 桌面端就不需要 Rust：安装包自带 board。下面这条命令把它的目录写进 shell 配置，新开终端就能用。'
                : '从源码安装需要 Rust；桌面端的安装包里已经带了 board。下面这条命令把它所在目录写进 shell 配置。'}{' '}
            命令只写用户级配置，重复执行不会把同一个目录追加两次，也不动系统级 PATH。
          </p>
          <div className="code-block">
            {isWindows ? (
              <Code ps cmd={pathCommand('powershell', '')} note="写进用户 Path；重复运行不会重复追加" />
            ) : (
              <Code cmd={pathCommand('sh', isMac ? '~/.zshrc' : '~/.bashrc')} note={`写进 ${isMac ? '~/.zshrc' : '~/.bashrc'}；重复运行不会重复追加`} />
            )}
            <Code plain cmd="board help" note="新开一个终端后验证：能打印帮助就说明 PATH 生效了" />
            <Code cmd="cargo install --path crates/board-cli" note="还没装桌面端：从源码安装（需要 Rust）" />
          </div>
          <p className="set-p">
            配好之后先跑一次 <code>board doctor</code>：board 在哪、PATH 里能不能找到、服务端地址和令牌来自环境变量还是配置文件、服务端能不能连、令牌是否有效、名字是否在线（带 <code>--as</code> 时），一次说清，有问题还会给出下一步。
            把下面这条粘给 agent，它自己就能先自查。
          </p>
          <div className="code-block">
            <Code plain cmd="board doctor" note="环境自检；全部通过时退出码 0" />
            <Code plain cmd="board doctor --json" note="结构化结果，方便 agent 解析" />
          </div>
          <details className="errcodes">
            <summary>错误码</summary>
            <div className="err-grid">
              {ERRORS.map(([k, v]) => (
                <span key={k} style={{ display: 'contents' }}>
                  <code>{k}</code>
                  <span>{v}</span>
                </span>
              ))}
            </div>
          </details>
        </section>

        {/* 访问令牌单独一节：放在 Agent 接入里时在一千多像素以下，侧栏目录跳不到 */}
        <section className="set-card" id="set-tokens" style={{ ['--i' as string]: 5 }}>
          <h2>
            <Icon name="key" />
            访问令牌
          </h2>
          <p className="set-p">
            agent 用 <code>board</code> 命令行连接这个服务端时，用访问令牌认证；<code>--as</code> / <code>BOARD_AS</code> 是 agent 的名字，会话开始时用{' '}
            <code>board check</code> 占用，同一时间一个名字只属于一个在线会话。不用的令牌随时吊销。
          </p>
          <div className="code-block">
            <Code cmd={`board login --url ${location.origin} --token <访问令牌>`} note="令牌在下面新建" />
            <Code plain cmd="board check --as claude/opus-5.5" note="会话开始时占用名字（skill 自动做）" />
          </div>
          <Tokens />
        </section>

        {canHost && <LocalHostCard />}

        {/* 桌面端更新：单写在一张卡里，和「本机」分开（本机里是 board host 的设置） */}
        <section className="set-card" id="set-update" style={{ ['--i' as string]: canUpdate ? 7 : 6 }}>
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

        <section className="set-card" id="set-shortcuts" style={{ ['--i' as string]: canUpdate ? 8 : canHost ? 7 : 6 }}>
          <h2>
            <Icon name="keyboard" />
            快捷键
          </h2>
          <div className="keys">
            {(
              [
                [`${accel('K')}  /`, '搜索 / 命令面板'],
                ['P', '打开项目 / 白板切换器'],
                [accel('1~6'), '切换到白板 / 未整理 / 大盘 / 项目 / 机器 / 设置'],
                [accel('B'), '收起 / 展开侧栏'],
                ['N', '新建卡片'],
                ['I', '记一条未整理'],
                ['Esc', '关闭弹窗、面板'],
                ['↑ ↓ ← → ↵', '在卡片间移动、打开卡片'],
                ['↑ ↓  K J', '详情里：同状态上一张 / 下一张'],
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
