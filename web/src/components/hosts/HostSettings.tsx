/* ==========================================================================
   机器详情「设置」页
   0. 资源：CPU、内存、GPU、存储四块磁贴（Resources.tsx）
   1. 接单：一行摘要（时段、自动领 / 只做派给它的 / 暂停、最多几个会话、一个会话几张）+「在接单里改」（机器 Tab 的「接单」视图）
   2. agent 更新：自动更新开关
   3. skills、MCP、Git 凭据：各一个开关（用服务器的 / 不改）、旁边一行结果；替换了本机同名的、远端不是 https 的仓库写一行；规则一行；「从这台机器导入 skill（N）」
   4. 工作空间：路径字段 +「更改…」（分栏选文件夹，本机在桌面端里弹系统的选择框）；只列例外；每个项目「能推送」
   5. 维护：重新检查、升级到 x.y.z（落后时才有）、重启 board host、令牌、移除这台机器
   开关、工作空间是设置项（当帧生效，旁边转圈）；导入、维护、移除要等服务端（按钮转圈、禁用直到返回）
   ========================================================================== */
import { useEffect, useState } from 'react';
import { api } from '../../lib/api';
import { gitLine, laterProjects, mcpLine, pushRows, sepOf, skillsLine, workspaceExceptions, type LineTone, type WsException } from '../../lib/hostDetail';
import { acceptSummary, isLocalHost, since } from '../../lib/hosts';
import { localHost, pickFolder, shell } from '../../lib/platform';
import { importRows } from '../../lib/skills';
import type { HostInfo, McpServer, SkillImportPick, SkillInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { hostKey, optimistic, useData } from '../../store/data';
import { confirm, confirmCheck, setHostsView, setTab, toast, toastError } from '../../store/ui';
import { Segmented, Sync, Tip, type SyncState } from '../bits';
import { Icon } from '../Icon';
import { patchHost } from './bits';
import { FolderPick } from './FolderPick';
import { askState } from './Manage';
import { Resources } from './Resources';
import type { Save } from './Saver';
import { useRemoteList } from './useRemoteList';

type SyncMap = Record<string, SyncState | undefined>;

export function HostSettings({
  h,
  save,
  sync,
  now,
  server,
  behind,
  running,
  close,
}: {
  h: HostInfo;
  save: Save;
  sync: SyncMap;
  now: number;
  server: string | null;
  behind: boolean;
  running: number;
  close: () => void;
}) {
  const push = pushRows(h);
  return (
    <div className="hd-set">
      <Resources h={h} now={now} server={server} behind={behind} />
      <section>
        <h3>接单</h3>
        <p className="hd-line">
          {acceptSummary(h, running)}
          <button
            type="button"
            className="hp-link hd-go"
            onClick={() => {
              // 从 Agent tab 的机器胶囊点进来时也回到机器页
              setTab('hosts');
              setHostsView('sched');
              close();
            }}
          >
            在接单里改
            <Icon name="chevRight" />
          </button>
        </p>
      </section>
      <section>
        <h3>agent 更新</h3>
        <div className="hd-sw">
          <div className="hd-sw-text">
            <b>自动更新 agent</b>
            <span className="t-muted">默认关。开着时，board host 发现有新版本、并且这台机器没有在跑的派发，就按它实际的安装方式更新；安装方式不支持的只在 agent 页提示。</span>
          </div>
          <span className="hp-ctl">
            <Sync s={sync.agentAuto} />
            <Segmented<AutoMode>
              value={h.agentAuto ? 'on' : 'off'}
              options={AUTO_MODES}
              onChange={v => v !== (h.agentAuto ? 'on' : 'off') && save({ agentAuto: v === 'on' })}
            />
          </span>
        </div>
      </section>
      <section>
        <h3>skills、MCP 和 Git 凭据</h3>
        <SyncBlock h={h} save={save} sync={sync} now={now} />
      </section>
      <section>
        <h3>工作空间</h3>
        <WorkspaceBlock h={h} save={save} sync={sync.workspace} />
        {push.length > 0 && (
          <ul className="hd-push">
            {push.map(r => (
              <li key={r.key} className={r.ok ? 'ok' : 'bad'}>
                <Icon name={r.ok ? 'checkCircle' : 'alert'} />
                <b>{r.label}</b>
                <span>{r.ok ? `自检 ${since(r.at, now)}` : `${r.why ?? '没通过'} · 自检 ${since(r.at, now)}`}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
      <section>
        <h3>维护</h3>
        <Maintenance h={h} server={server} behind={behind} running={running} close={close} />
      </section>
    </div>
  );
}

/* ------------------------------ skills、MCP 和 Git 凭据 ------------------------------ */

type Mode = 'server' | 'off';
type AutoMode = 'on' | 'off';
const AUTO_MODES: [AutoMode, string][] = [
  ['off', '关闭'],
  ['on', '自动更新'],
];
const MODES: [Mode, string][] = [
  ['server', '用服务器的'],
  ['off', '不改'],
];
const LINE_ICON: Record<LineTone, Parameters<typeof Icon>[0]['name']> = { ok: 'checkCircle', busy: 'renew', bad: 'alert', muted: 'info' };

function SyncBlock({ h, save, sync, now }: { h: HostInfo; save: Save; sync: SyncMap; now: number }) {
  const ws = useData(s => s.ws);
  const { list: lib } = useRemoteList<SkillInfo[]>(() => api.skills(), ws?.hosts);
  const { list: mcp } = useRemoteList<McpServer[]>(() => api.mcp(), ws?.hosts);
  const skills = skillsLine(h, lib?.length ?? null, now);
  const mcps = mcpLine(h, mcp?.map(m => m.name) ?? null, now);
  const { list: git } = useRemoteList(() => api.git(), ws?.hosts);
  const gits = gitLine(h, git ? git.cred : undefined, now);
  const skipped = h.sync?.last?.git.skipped ?? [];
  const r = h.sync?.last?.skills;
  const busy = !!h.sync?.ask;
  const old = !h.sync?.able;
  return (
    <div className="hd-sync">
      {old && (
        <p className="hp-note hd-sync-old">
          <Icon name="alert" />
          board {h.version || '?'} 是旧版：按老办法同步，开关升级后才生效
        </p>
      )}
      <div className={cx('hd-sw', sync.syncSkills === 'failed' && 'hp-rollback')}>
        <div className="hd-sw-text">
          <b>skills</b>
          <span className={cx(`t-${skills.tone}`)}>
            <Icon name={LINE_ICON[skills.tone]} />
            {skills.text}
          </span>
        </div>
        <span className="hp-ctl">
          <Sync s={sync.syncSkills} />
          <Segmented<Mode> value={h.syncSkills ? 'server' : 'off'} options={MODES} onChange={v => v !== (h.syncSkills ? 'server' : 'off') && save({ syncSkills: v === 'server' })} />
        </span>
      </div>
      {h.syncSkills && !busy && !!r?.replaced.length && (
        <p className="hd-sync-note">
          <Icon name="info" />
          <span>
            替换了本机同名的 {r.replaced.join('、')}：旧的挪进了 <code>~/.board/host/skills-backup/</code>
          </span>
        </p>
      )}
      {h.syncSkills && !busy && !!r?.links.length && (
        <p className="hd-sync-note">
          <Icon name="info" />
          <span>本机的 {r.links.join('、')} 是软链接，没有覆盖</span>
        </p>
      )}
      <div className={cx('hd-sw', sync.syncMcp === 'failed' && 'hp-rollback')}>
        <div className="hd-sw-text">
          <b>MCP</b>
          <span className={cx(`t-${mcps.tone}`)}>
            <Icon name={LINE_ICON[mcps.tone]} />
            {mcps.text}
          </span>
        </div>
        <span className="hp-ctl">
          <Sync s={sync.syncMcp} />
          <Segmented<Mode> value={h.syncMcp ? 'server' : 'off'} options={MODES} onChange={v => v !== (h.syncMcp ? 'server' : 'off') && save({ syncMcp: v === 'server' })} />
        </span>
      </div>
      {h.syncMcp && !busy && !!h.sync?.last?.mcp.replaced.length && (
        <p className="hd-sync-note">
          <Icon name="info" />
          <span>
            换掉了本机同名的 {h.sync.last.mcp.replaced.join('、')}：原来的值存进了 <code>~/.board/host/</code> 下的备份文件
          </span>
        </p>
      )}
      <div className={cx('hd-sw', sync.syncGit === 'failed' && 'hp-rollback')}>
        <div className="hd-sw-text">
          <b>Git 凭据</b>
          <span className={cx(`t-${gits.tone}`)}>
            <Icon name={LINE_ICON[gits.tone]} />
            {gits.text}
          </span>
        </div>
        <span className="hp-ctl">
          <Sync s={sync.syncGit} />
          <Segmented<Mode> value={h.syncGit ? 'server' : 'off'} options={MODES} onChange={v => v !== (h.syncGit ? 'server' : 'off') && save({ syncGit: v === 'server' })} />
        </span>
      </div>
      {h.syncGit && !busy && skipped.length > 0 && (
        <p className="hd-sync-note">
          <Icon name="info" />
          <span>没配的仓库：{skipped.join('；')}</span>
        </p>
      )}
      {(skills.tone === 'bad' || mcps.tone === 'bad' || gits.tone === 'bad') && <RetrySync h={h} />}
      <p className="hp-note hd-sync-rule">board-skills 不管选哪个都装：没有它派来的卡做不了。本机有同名的、不是 Board 装的，换成库里的，旧的挪进备份；软链接（开发中的 skill）不碰。</p>
      <SkillImportInline h={h} lib={lib} />
    </div>
  );
}

function RetrySync({ h }: { h: HostInfo }) {
  const [busy, setBusy] = useState(false);
  return (
    <div className="hd-retry">
      <button
        className={cx('btn sm', busy && 'is-busy')}
        type="button"
        disabled={busy || !h.online}
        onClick={async () => {
          setBusy(true);
          try {
            await optimistic(null, () => api.syncHost(h.name));
          } catch (e) {
            toastError(e);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Icon name="renew" />
        重试
      </button>
    </div>
  );
}

/** 「从这台机器导入 skill（N）」：展开列出这台机器 agent 目录里不是 Board 装的 skill，勾选后「导入到库」 */
function SkillImportInline({ h, lib }: { h: HostInfo; lib: SkillInfo[] | null }) {
  const [open, setOpen] = useState(false);
  const [picked, setPicked] = useState<SkillImportPick[]>([]);
  const [busy, setBusy] = useState(false);
  const [sent, setSent] = useState<number | null>(null);
  const rows = importRows(h.skillImport?.local ?? [], lib ?? []).filter(r => r.kind === 'new' || r.kind === 'differs');
  const last = h.skillImport?.last;
  const done = sent != null && last?.id === sent;
  useEffect(() => setPicked([]), [h.name]);
  if (!h.skillImport?.able) return null;
  const go = async () => {
    setBusy(true);
    try {
      const out = await optimistic(null, () => api.importSkills(h.name, picked));
      if (out.skillImport?.ask) setSent(out.skillImport.ask.id);
      setOpen(false);
      setPicked([]);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="hd-imp">
      <button type="button" className={cx('btn sm', open && 'on')} aria-expanded={open} onClick={() => setOpen(!open)} disabled={!rows.length || !h.online}>
        <Icon name="download" />
        从这台机器导入 skill{rows.length ? `（${rows.length}）` : ''}
      </button>
      {!rows.length && <span className="hp-note">这台机器上没有可以导入的 skill（不是 Board 装的、库里没有或内容不一样的）</span>}
      {sent != null && (
        <span className="hp-note">
          {done
            ? [last?.ok.length ? `已导入 ${last.ok.join('、')}` : '', last?.failed.length ? `没导入：${last.failed.map(f => `${f.name}（${f.why}）`).join('、')}` : ''].filter(Boolean).join('；')
            : `正在等 ${h.name} 把它们传进库…`}
        </span>
      )}
      {open && (
        <div className="hd-imp-list">
          <p className="hp-note">这台机器的 agent 目录里、不是 Board 装的 skill。勾选的传进共用库，之后同步给选了「用服务器的」的机器；库里有同名的换成这份。这台机器上的目录不动。</p>
          {rows.map(r => {
            const dir = r.skill.dirs[0];
            const on = picked.some(p => p.name === r.skill.name);
            return (
              <label key={`${r.skill.name}:${dir}`} className="hd-imp-row">
                <input
                  type="checkbox"
                  checked={on}
                  onChange={e => setPicked(e.target.checked ? [...picked, { name: r.skill.name, dir }] : picked.filter(p => p.name !== r.skill.name))}
                />
                <b>{r.skill.name}</b>
                <span>{dir}</span>
                {r.kind === 'differs' && <em>会换掉库里的</em>}
              </label>
            );
          })}
          <div className="hd-imp-acts">
            <button type="button" className={cx('btn sm brand', busy && 'is-busy')} disabled={!picked.length || busy} onClick={() => void go()}>
              导入到库
            </button>
            <button type="button" className="btn sm ghost" disabled={busy} onClick={() => setOpen(false)}>
              取消
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/* ------------------------------ 工作空间 ------------------------------ */

function WorkspaceBlock({ h, save, sync }: { h: HostInfo; save: Save; sync?: SyncState }) {
  const path = h.workspace?.path ?? '';
  const [picking, setPicking] = useState(false);
  const [local, setLocal] = useState(false);
  const [busy, setBusy] = useState('');
  const ex = workspaceExceptions(h);
  const later = laterProjects(h);
  useEffect(() => {
    if (shell !== 'desktop') return;
    let alive = true;
    void localHost().then(x => alive && setLocal(isLocalHost(h, x)));
    return () => {
      alive = false;
    };
  }, [h]); // eslint-disable-line react-hooks/exhaustive-deps

  /** 存工作空间（`--dir` 单独指定的原样带着）；本机改之前先确认 */
  const apply = async (next: string) => {
    if (next.trim() === path) return;
    if (
      local &&
      !(await confirm({
        title: '修改这台电脑的工作空间？',
        text: '保存后 board host 会换上新的工作空间：每个项目的目录是 <工作空间>/<项目>，正在跑的 agent 不受影响。',
        ok: '保存',
        icon: 'alert',
        tone: 'warn',
      }))
    )
      return;
    save({ workspace: { path: next.trim(), overrides: h.workspace?.overrides ?? [] } });
  };
  /** 本机在桌面端里：弹系统的文件夹选择框；别的机器：分栏浏览那台机器的文件夹 */
  const change = async () => {
    if (!(shell === 'desktop' && local)) return setPicking(true);
    try {
      const p = await pickFolder(path);
      if (p) await apply(p);
    } catch (e) {
      if (e === 'unsupported') setPicking(true);
      else toast({ type: 'error', title: '没有打开文件夹选择框', text: String(e) });
    }
  };
  const reclone = async (x: WsException) => {
    if (!x.reclone) return;
    const { board, url } = x.reclone;
    setBusy(x.key);
    try {
      await optimistic(
        {
          ...patchHost(h.name, y => ({
            ...y,
            dirs: y.dirs.map(d => (d.board === board ? { ...d, clone: { state: 'queued', url, message: '等待 board host 开始克隆', updatedAt: Date.now(), by: '' } } : d)),
          })),
          keys: [hostKey(h.name)],
        },
        () => api.cloneHostDir(h.name, board, url),
      );
    } catch (e) {
      toastError(e);
    } finally {
      setBusy('');
    }
  };
  const sep = sepOf(h);
  return (
    <div className="hd-ws">
      <div className={cx('hd-ws-field', sync === 'failed' && 'hp-rollback')}>
        <span className="hd-ws-ico">
          <Icon name="folder" />
        </span>
        <div className="hd-ws-main">
          <code title={path}>{path || '还没选工作空间：派不了卡'}</code>
          <small>
            每个项目一个目录：
            <code>
              {(path || '<工作空间>').replace(/[\\/]+$/, '')}
              {sep}&lt;项目&gt;
            </code>
            {later.length > 0 && `。${later.join('、')} 还没有，第一次派来时建好或克隆`}
          </small>
        </div>
        {/* 保存状态贴在「更改…」左边：包在 .hp-ctl 里，不然定位落到弹窗上、画到可视区外面（default/main#391） */}
        <span className="hp-ctl">
          <Sync s={sync} />
          <button type="button" className="btn sm" onClick={() => void change()}>
            更改…
          </button>
        </span>
      </div>
      {picking && (
        <FolderPick
          h={h}
          current={path}
          onPick={p => {
            setPicking(false);
            void apply(p);
          }}
          onClose={() => setPicking(false)}
        />
      )}
      {ex.length > 0 && (
        <ul className="hd-ex">
          {ex.map(x => (
            <li key={x.key} className={x.tone}>
              <Icon name={x.tone === 'bad' ? 'alert' : 'info'} />
              <b>{x.label}</b>
              <span>{x.text}</span>
              {x.reclone && (
                <button type="button" className={cx('btn sm', busy === x.key && 'is-busy')} disabled={!!busy || !h.online} onClick={() => void reclone(x)}>
                  重新克隆
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/* ------------------------------ 维护 ------------------------------ */

function Maintenance({ h, server, behind, running, close }: { h: HostInfo; server: string | null; behind: boolean; running: number; close: () => void }) {
  const [busy, setBusy] = useState<string | null>(null);
  const offline = !h.online;
  const pending = !!h.ask;
  const state = askState(h, running);
  const send = async (kind: 'recheck' | 'restart' | 'upgrade' | 'cancel', now = false, done?: string) => {
    if (busy) return;
    setBusy(kind);
    try {
      await optimistic(null, () => api.askHost(h.name, kind, now));
      if (done) toast({ type: 'success', title: done });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const restart = async () => {
    const yes = await confirm({
      title: `重启 ${h.name} 上的 board host？`,
      text: running
        ? `正在跑的 ${running} 个派发会按「人停止」结束（卡片移到需要人处理的状态）。board host 重新启动后重新读配置、重新检查，一般 30 秒内重新上线。`
        : 'board host 重新启动后重新读配置、重新检查，一般 30 秒内重新上线。',
      ok: '重启',
      icon: 'alert',
      tone: 'warn',
    });
    if (yes) void send('restart', false, `已通知 ${h.name} 重启 board host`);
  };
  const upgradeNow = async () => {
    const yes = await confirm({
      title: `立即升级 ${h.name}？`,
      text: `正在跑的 ${running} 个派发会按「人停止」结束（卡片移到需要人处理的状态），然后马上升级到 ${server}。`,
      ok: '立即升级',
      icon: 'alert',
      tone: 'warn',
    });
    if (yes) void send('upgrade', true);
  };
  const remove = async () => {
    const t = h.token;
    const { ok: yes, checked: revoke } = await confirmCheck({
      title: `移除 ${h.name}？`,
      text: '取消它所有排队中的派发，停止正在跑的派发，删除机器设置；派发记录保留。那台机器的 board host 再连上来时按新机器处理。',
      ok: '移除',
      check: {
        label: t ? `同时吊销它用的令牌「${t.name}」（${t.hint}）` : '同时吊销它用的令牌',
        why: !t ? '看不到这台机器用的令牌（还没上报过，或已经吊销）' : t.shared?.length ? `机器 ${t.shared.join('、')} 也在用这个令牌，不能一起吊销` : undefined,
      },
    });
    if (!yes) return;
    // 当帧关闭弹窗、从列表里拿掉（连同它的派发），被拒绝时回来并提示
    close();
    try {
      await optimistic(
        {
          ws: ws => ({ ...ws, hosts: ws.hosts?.filter(x => x.name !== h.name), dispatches: ws.dispatches?.filter(x => x.host !== h.name) }),
          keys: [hostKey(h.name)],
        },
        () => api.removeHost(h.name, revoke),
      );
      toast({ type: 'success', title: `已移除 ${h.name}`, text: revoke && t ? `令牌「${t.name}」已吊销` : undefined });
    } catch (e) {
      toastError(e);
    }
  };
  const t = h.token;
  const row = (key: string, icon: Parameters<typeof Icon>[0]['name'], label: string, sub: string, act?: { label: string; run: () => void; disabled?: boolean; tip?: string; danger?: boolean }) => (
    <div key={key} className={cx('hd-mrow', act?.danger && 'danger')}>
      <Icon name={icon} />
      <div>
        <b>{label}</b>
        <span>{sub}</span>
      </div>
      {act && (
        <Tip text={act.tip ?? ''}>
          <span>
            <button className={cx('btn sm', act.danger && 'danger', busy === key && 'is-busy')} type="button" disabled={!!busy || act.disabled} onClick={act.run}>
              {act.label}
            </button>
          </span>
        </Tip>
      )}
    </div>
  );
  return (
    <>
      <div className="hd-maint">
        {row('recheck', 'renew', '重新检查', h.recheckedAt ? `重新找启动环境、agent、board-skills · 上次 ${since(h.recheckedAt, Date.now())}` : '重新找启动环境、agent、board-skills', {
          label: '检查',
          run: () => void send('recheck', false, `已通知 ${h.name} 重新检查`),
          disabled: offline || pending,
          tip: offline ? '机器离线' : '在跑的派发不受影响',
        })}
        {behind &&
          row('upgrade', 'download', `升级到 ${server}`, '先暂停接单，等在跑的派发做完再升级；升级完恢复接单', {
            label: '升级',
            run: () => void send('upgrade', false, `${h.name} 会在在跑的派发结束后升级`),
            disabled: offline || pending,
            tip: offline ? '机器离线' : '',
          })}
        {row('restart', 'replay', '重启 board host', '在跑的派发按「人停止」结束', { label: '重启', run: () => void restart(), disabled: offline || pending, tip: offline ? '机器离线' : '' })}
        {row(
          'token',
          'key',
          t ? `令牌「${t.name}」` : '令牌',
          t ? `${t.hint} · ${t.lastUsedAt ? `最近使用 ${since(t.lastUsedAt, Date.now())}` : '还没用过'}${t.shared?.length ? ` · 机器 ${t.shared.join('、')} 也在用` : ''}` : '看不到：还没上报过，或令牌已经吊销',
        )}
        {row('remove', 'trash', '移除这台机器', '排队的取消，在跑的停止；派发记录保留', { label: '移除', run: () => void remove(), danger: true })}
      </div>
      {state && (
        <div className={cx('hp-ask', state.tone)} role="status">
          {state.tone === 'busy' && <i className="hp-ask-spin" />}
          {state.tone === 'ok' && <Icon name="check" />}
          <span>
            {state.text}
            {h.ask && (
              <em>
                {' '}
                · {h.ask.by} {since(h.ask.at, Date.now())}请求
              </em>
            )}
          </span>
          {h.ask?.kind === 'upgrade' && !h.ask.now && running > 0 && (
            <button className="btn sm" type="button" disabled={!!busy} onClick={() => void upgradeNow()}>
              立即升级
            </button>
          )}
          {h.ask && (
            <button className={cx('btn sm ghost', busy === 'cancel' && 'is-busy')} type="button" disabled={!!busy} onClick={() => void send('cancel')}>
              取消
            </button>
          )}
        </div>
      )}
    </>
  );
}
