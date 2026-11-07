/* ==========================================================================
   设置 Tab（PRD §10.6）：外观 / 白板规则 / Agent 接入 / 快捷键
   侧栏是目录，点击跳到对应部分；滚动时目录跟随
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { create } from 'zustand';
import { api } from '../lib/api';
import { getTheme, setDensity, setMotion, setTheme, shellVersion, type Density, type Motion, type ThemePref } from '../lib/platform';
import { boardFlow, ERRORS } from '../lib/schema';
import type { TokenInfo } from '../lib/types';
import { ago, copyText, cx, MOD, pref, reduceMotion, replay } from '../lib/util';
import skill from '../../../.agents/skills/board-skills/SKILL.md?raw';
import { logout, useAuth } from '../store/auth';
import { useData } from '../store/data';
import { confirm, toast, toastError, useUI } from '../store/ui';
import { Icon, type IconName } from './Icon';

const SECTIONS: { id: string; title: string; icon: IconName }[] = [
  { id: 'appearance', title: '外观', icon: 'sun' },
  { id: 'board', title: '白板规则', icon: 'lock' },
  { id: 'agents', title: 'Agent 接入', icon: 'terminal' },
  { id: 'account', title: '账号', icon: 'user' },
  { id: 'shortcuts', title: '快捷键', icon: 'keyboard' },
];

const useSpy = create<{ active: string; lock: boolean }>(() => ({ active: SECTIONS[0].id, lock: false }));

export function SettingsNav() {
  const active = useSpy(s => s.active);
  return (
    <nav className="settings-nav scroll">
      {SECTIONS.map(s => (
        <a
          key={s.id}
          href={`#set-${s.id}`}
          className={cx('sn-item', active === s.id && 'on')}
          onClick={e => {
            e.preventDefault();
            const page = document.querySelector<HTMLElement>('.page-settings');
            const target = document.getElementById(`set-${s.id}`);
            if (!page || !target) return;
            useSpy.setState({ active: s.id, lock: true });
            setTimeout(() => useSpy.setState({ lock: false }), 900);
            page.scrollTo({ top: target.offsetTop - 20, behavior: reduceMotion() ? 'auto' : 'smooth' });
            replay(target, 'flash');
          }}
        >
          <Icon name={s.icon} />
          <span>{s.title}</span>
        </a>
      ))}
    </nav>
  );
}

function Segmented<T extends string>({ value, options, onChange }: { value: T; options: [T, string][]; onChange: (v: T) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pill, setPill] = useState({ w: 0, x: 0 });
  useLayoutEffect(() => {
    const place = () => {
      const on = ref.current?.querySelector<HTMLElement>('button.on');
      if (on) setPill({ w: on.offsetWidth, x: on.offsetLeft - 3 });
    };
    place();
    document.fonts?.ready.then(place);
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [value]);
  return (
    <div className="segmented" role="radiogroup" ref={ref}>
      {options.map(([v, label]) => (
        <button key={v} type="button" role="radio" aria-checked={v === value} className={cx(v === value && 'on')} onClick={() => onChange(v)}>
          {label}
        </button>
      ))}
      <i className="seg-pill" style={{ width: pill.w, transform: `translateX(${pill.x}px)` }} />
    </div>
  );
}

const Row = ({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) => (
  <div className="set-row">
    <div className="set-label">
      <b>{label}</b>
      {hint && <span>{hint}</span>}
    </div>
    <div className="set-control">{children}</div>
  </div>
);

function Code({ cmd, note, plain }: { cmd: string; note?: string; plain?: boolean }) {
  const [done, setDone] = useState(false);
  return (
    <div className={cx('code-line', plain && 'plain')}>
      <code>{cmd}</code>
      {note && <span>{note}</span>}
      <button
        type="button"
        className={cx('copy', done && 'done')}
        aria-label="复制"
        onClick={async () => {
          await copyText(cmd);
          setDone(true);
          setTimeout(() => setDone(false), 1400);
        }}
      >
        <Icon name={done ? 'check' : 'copy'} />
      </button>
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
    try {
      await api.revokeToken(t.id);
      await load();
    } catch (err) {
      toastError(err);
    }
  };

  const now = Date.now();
  return (
    <div className="tokens">
      {fresh && (
        <div className="token-fresh" role="status">
          <b>令牌「{fresh.name}」已新建。原文只显示这一次，请现在复制保存。</b>
          <div className="code-block">
            <Code cmd={`export BOARD_TOKEN=${fresh.token}`} />
          </div>
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
        <button type="submit" className="btn sm primary" disabled={!name.trim() || busy}>
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

export function SettingsPage() {
  const path = useData(s => s.path);
  const flow = boardFlow(useData(s => s.board));
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
    <div className="page page-settings scroll" data-page="settings" ref={page}>
      <div className="settings">
        <header className="settings-hero">
          <h1>设置</h1>
          <p>界面的显示方式、agent 如何通过命令行接入，以及账号。</p>
        </header>

        <section className="set-card" id="set-appearance" style={{ ['--i' as string]: 1 }}>
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

        <section className="set-card" id="set-board" style={{ ['--i' as string]: 1 }}>
          <h2>
            <Icon name="lock" />
            白板规则
          </h2>
          <p className="set-p">每块白板的工作流可以在白板页顶部的工作流条里自定义。当前白板：</p>
          <div className="flow">
            {[{ key: 'inbox', name: '未整理', who: '人记下想法' }, ...flow.list.map(s => ({ key: s.key, name: s.name, who: s.claim ? '领取起点' : s.lock ? '锁定 · agent 领取' : s.final ? '终点 · 人确认' : '' }))].map(
              (s, i) => (
                <span key={s.key} style={{ display: 'contents' }}>
                  {i > 0 && <i className="flow-arrow" />}
                  <span className="flow-node" style={{ ['--c' as string]: flow.color(s.key) }}>
                    <i />
                    {s.name}
                    {s.who && <em>{s.who}</em>}
                  </span>
                </span>
              ),
            )}
          </div>
          <Row label="领取时长" hint="持有者评论、编辑或再次 claim 都会重新计时">
            <span className="ro mono">30 分钟</span>
          </Row>
          <Row label="超时" hint="到期未续期：回到领取起点最上面，记录「领取超时，自动释放」">
            <span className="ro">自动释放</span>
          </Row>
          <ul className="rules">
            <li>只有领取起点里的卡片能被领取；领取后卡片进入它之后的第一个锁定状态；多个 agent 同时领取同一张，只有一个成功</li>
            <li>一个 agent 在一块白板上同时只持有一张卡片</li>
            <li>进入锁定状态只能靠领取；持有者可以把自己的卡片在锁定状态之间移动，领取保持</li>
            <li>被持有的卡片只有持有者能编辑和移动，其他人只能评论、加或去标签；人可以强制释放</li>
            <li>卡片离开锁定状态时，领取一起释放；强制释放和超时释放的卡片回到领取起点最上面</li>
            <li>终点状态只归人管；删除只能在界面上做</li>
            <li>「未整理」只在界面上新建，agent 不会领取；整理后再放进领取起点</li>
          </ul>
        </section>

        <section className="set-card" id="set-agents" style={{ ['--i' as string]: 2 }}>
          <h2>
            <Icon name="terminal" />
            Agent 接入
          </h2>
          <p className="set-p">
            给 agent 装上 <code>board-skills</code>，之后只需要发一句话：领取、按描述干活、记进度、交审、规则和错误码都写在 skill 里，agent 自己用{' '}
            <code>board</code> 命令行读取卡片。卡片详情里点「复制为提示词」（或按 <kbd>C</kbd>）得到的就是这一句。
          </p>
          <div className="code-block">
            <Code plain cmd={`/board-skills ${path}#12`} note="处理一张卡片" />
            <Code plain cmd={`/board-skills ${path}`} note={`领完这块白板的「${flow.claim?.name ?? '待办'}」`} />
            <Code plain cmd="/board-skills README 加一个英文版" note={`一句话需求：先记到「${flow.claim?.name ?? '待办'}」再做`} />
          </div>
          <Row label="安装 skill" hint="保存为 ~/.agents/skills/board-skills/SKILL.md（Claude Code 放在 ~/.claude/skills/board-skills/）">
            <CopySkill />
          </Row>
          <div className="code-block">
            <Code cmd={'ln -s "$PWD/.agents/skills/board-skills" ~/.agents/skills/board-skills'} note="在 Board 仓库里执行：链接到 skills 目录" />
            <Code cmd="cargo install --path crates/board-cli" note="安装 board 命令行" />
          </div>
          <h3 className="set-sub">
            <Icon name="key" />
            访问令牌
          </h3>
          <p className="set-p">
            agent 用 <code>board</code> 命令行连接这个服务端时，用访问令牌认证；<code>--as</code> / <code>BOARD_AS</code> 只是 agent 的名字。不用的令牌随时吊销。
          </p>
          <div className="code-block">
            <Code cmd={`export BOARD_URL=${location.origin}`} note="服务端地址" />
            <Code cmd="export BOARD_TOKEN=<访问令牌>" note="在下面新建" />
          </div>
          <Tokens />
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

        <section className="set-card" id="set-account" style={{ ['--i' as string]: 3 }}>
          <h2>
            <Icon name="user" />
            账号
          </h2>
          <Account />
        </section>

        <section className="set-card" id="set-shortcuts" style={{ ['--i' as string]: 3 }}>
          <h2>
            <Icon name="keyboard" />
            快捷键
          </h2>
          <div className="keys">
            {(
              [
                [`${MOD} K  /`, '搜索 / 命令面板'],
                ['P', '打开项目 / 白板切换器'],
                [`${MOD} 1~4`, '切换到白板 / 项目 / 未整理 / 设置'],
                [`${MOD} B`, '收起 / 展开侧栏'],
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
