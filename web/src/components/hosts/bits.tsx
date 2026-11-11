/* 机器页共用的小部件：系统图标、状态、agent 标记、心跳、派发的运行状态；轮询机器列表；改机器设置的补丁 */
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { api } from '../../lib/api';
import { agentMark, hostState, osKind, runFor, runInfo, since, type OsKind } from '../../lib/hosts';
import type { Dispatch, HostAgent, HostInfo } from '../../lib/types';
import { cx, hhmm } from '../../lib/util';
import { hostKey, refreshHosts, type Patch } from '../../store/data';
import { useNow } from '../bits';
import { Icon } from '../Icon';

/** Apple 品牌图标：单色跟随文字颜色，深色、浅色主题都清晰 */
const AppleMark = () => (
  <svg viewBox="0 0 384 512" fill="currentColor" aria-hidden="true">
    <path d="M318.7 268.7c-.2-36.7 16.4-64.4 50-84.8-18.8-26.9-47.2-41.7-84.7-44.6-35.5-2.8-74.3 20.7-88.5 20.7-15 0-49.4-19.7-76.4-19.7C63.3 141.2 4 184.8 4 273.5q0 39.3 14.4 81.2c12.8 36.7 59 126.7 107.2 125.2 25.2-.6 43-17.9 75.8-17.9 31.8 0 48.3 17.9 76.4 17.9 48.6-.7 90.4-82.5 102.6-119.3-65.2-30.7-61.7-90-61.7-91.9zm-56.6-164.2c27.3-32.4 24.8-61.9 24-72.5-24.1 1.4-52 16.4-67.9 34.9-17.5 19.8-27.8 44.3-25.6 71.9 26.1 2 49.9-11.4 69.5-34.3z" />
  </svg>
);

/** Microsoft Windows 品牌图标：官方四色四窗格，深色、浅色背景都清晰 */
const WindowsMark = () => (
  <svg viewBox="0 0 88 88" aria-hidden="true">
    <path fill="#F25022" d="M0 12.5 35.7 7.6v34.5H0Z" />
    <path fill="#7FBA00" d="M40 6.9 88 0v41.9H40Z" />
    <path fill="#00A4EF" d="M0 50.5h35.7v33.9L0 79.4Z" />
    <path fill="#FFB900" d="M40 50.5H88V88L40 81.2Z" />
  </svg>
);

/* 认识的大系统用品牌图标，Linux 及其他保持字符回退 */
const OS_GLYPH: Record<OsKind, ReactNode> = { mac: <AppleMark />, linux: '$_', win: <WindowsMark /> };

export function OsGlyph({ os, small }: { os: string; small?: boolean }) {
  const k = osKind(os);
  return (
    <span className={cx('hp-os', `os-${k}`, small && 'sm')} title={os}>
      {OS_GLYPH[k]}
    </span>
  );
}

export function StatePill({ h, running }: { h: HostInfo; running: number }) {
  const s = hostState(h, running);
  return (
    <span className="hp-pill" style={{ ['--c' as string]: s.color }}>
      <i className={cx('hp-dot', `st-${s.key}`)} />
      {s.label}
    </span>
  );
}

export function Dot({ h, running }: { h: HostInfo; running: number }) {
  const s = hostState(h, running);
  return <i className={cx('hp-dot', `st-${s.key}`)} style={{ ['--c' as string]: s.color }} />;
}

/** agent 标记：认识的 agent 用它自己的图标，其他用两个字母 */
export function Mark({ id, title }: { id: string; title?: string }) {
  const m = agentMark(id);
  if (m.icon) return <img className={cx('hp-mini hp-logo', m.light && 'ring')} src={m.icon} alt={title ?? id} title={title ?? id} draggable={false} />;
  return (
    <b className="hp-mini" style={{ ['--ac' as string]: m.color }} title={title ?? id}>
      {m.short}
    </b>
  );
}

/** agent 自己的图标；不认识的 agent 用两个字母 */
export function AgentIcon({ id }: { id: string }) {
  const m = agentMark(id);
  return m.icon ? <img className={cx('hp-logo', m.light && 'ring')} src={m.icon} alt="" draggable={false} /> : <b style={{ ['--ac' as string]: m.color }}>{m.short}</b>;
}

/** agent 胶囊：图标、名称、启动环境（「pi · Git Bash」）、版本号、「默认」 */
export function AgentChip({ a, def, small, env, skill }: { a: HostAgent; def?: boolean; small?: boolean; env?: string | null; skill?: string | null }) {
  const m = agentMark(a.id);
  return (
    <span className={cx('hp-agent', !a.found && 'missing', def && 'def', small && 'sm', skill && 'noskill')} style={{ ['--ac' as string]: m.color }}>
      <AgentIcon id={a.id} />
      {a.name}
      {env && <i className="hp-agent-env">· {env}</i>}
      {!small && <em>{a.found ? a.version || '找到了' : '没找到'}</em>}
      {def && <small>默认</small>}
      {skill && (
        <span className="hp-agent-warn" role="img" aria-label={skill} data-tip={skill}>
          <Icon name="alert" />
        </span>
      )}
    </span>
  );
}

/** 心跳。tip：派发的心跳写明「心跳不等于干活」 */
export function Beat({ at, now, tip }: { at: number | null; now: number; tip?: (at: number) => string }) {
  if (at == null) return <span className="hp-beat off">还没有心跳</span>;
  return (
    <span className="hp-beat" title={tip ? tip(at) : `最近一次心跳 ${hhmm(at)}`}>
      <i />
      {since(at, now)}
    </span>
  );
}

export const lastSeen = (h: HostInfo, now: number) => (h.online ? `心跳 ${since(h.beatAt, now)}` : `${since(h.offAt ?? h.beatAt, now)}在线`);

/** 机器页打开（或机器详情打开）时立即刷新一次机器和派发，之后每 10 秒一次：机器心跳不推送，靠它更新「最近心跳」 */
export function usePollHosts(on: boolean) {
  useEffect(() => {
    if (!on) return;
    const load = () => void refreshHosts().catch(() => {});
    load();
    const t = window.setInterval(load, 10000);
    return () => clearInterval(t);
  }, [on]);
}

/** 改一台机器（工作区里的 hosts） */
export const patchHost = (name: string, fn: (h: HostInfo) => HostInfo): Patch => ({
  ws: ws => ({ ...ws, hosts: ws.hosts?.map(h => (h.name === name ? fn(h) : h)) }),
  keys: [hostKey(name)],
});

/** 派发的运行状态：状态词（工作中 / 等待输出 / 可能空转）和依据，悬停看判断口径。
    short：只画状态词（机器页的车道），依据放进悬停提示。排队、已结束、正在停止、旧版服务端不画 */
export function RunStatus({ d, now, short }: { d: Dispatch; now: number; short?: boolean }) {
  const s = runInfo(d, now);
  if (!s || (short && !s.key)) return null;
  const tip = short ? `${s.basis}${s.note ? `（${s.note}）` : ''}。${s.tip}` : s.tip;
  return (
    <div className={cx('hp-rs', s.key && `rs-${s.key}`, short && 'short')} data-tip={tip} aria-label={short ? `${s.label}：${s.basis}` : undefined}>
      {s.label && <b>{s.label}</b>}
      {!short && <i>{s.basis}</i>}
      {!short && s.note && <em>{s.note}</em>}
    </div>
  );
}

/** 日志为空时的一句话：agent 启动多久了、还没有输出，带上运行状态。旧版服务端（没有 outputAt）、已经有过输出时用原来的文案 */
function emptyLog(d: Dispatch | undefined, now: number) {
  const old = '还没有输出（host 每 5 秒上传一次新增的行）';
  if (!d || d.state !== 'running' || d.outputAt !== null || d.startedAt == null) return old;
  const head = `agent 已启动 ${runFor(now - d.startedAt)}，还没有输出`;
  const s = runInfo(d, now);
  if (!s?.key) return `${head}（host 每 5 秒上传一次新增的行）`;
  return `${head}\n${s.label} · CPU ${d.cpu}%${s.note ? ` · ${s.note}` : ''}`;
}

/** 派发日志：最后 500 行；在跑时每 3 秒取一次。d：日志为空时按运行状态说明 */
export function LogView({ id, live, d }: { id: number; live?: boolean; d?: Dispatch }) {
  const now = useNow(d ? 1000 : null);
  const [lines, setLines] = useState<string[] | null>(null);
  const ref = useRef<HTMLPreElement>(null);
  const stick = useRef(true);
  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .dispatchLog(id)
        .then(r => alive && setLines(r.lines))
        .catch(() => alive && setLines(l => l ?? []));
    void load();
    const t = live ? window.setInterval(load, 3000) : 0;
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [id, live]);
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  return (
    <pre
      className="hp-log"
      ref={ref}
      onScroll={e => {
        const el = e.currentTarget;
        stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 8;
      }}
    >
      {lines === null ? '正在加载…' : lines.length ? lines.join('\n') : emptyLog(d, now)}
      {live && lines !== null && (
        <>
          {'\n'}
          <span className="hp-caret">▍</span>
        </>
      )}
    </pre>
  );
}
