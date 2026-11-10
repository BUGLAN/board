/* 机器页共用的小部件：系统图标、状态、agent 标记、心跳；轮询机器列表；改机器设置的补丁 */
import { useEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { agentMark, hostState, osKind, since } from '../../lib/hosts';
import type { HostAgent, HostInfo } from '../../lib/types';
import { cx, hhmm } from '../../lib/util';
import { hostKey, refreshHosts, type Patch } from '../../store/data';

const OS_GLYPH = { mac: '⌘', linux: '$_', win: '⊞' } as const;

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
export function AgentChip({ a, def, small, env }: { a: HostAgent; def?: boolean; small?: boolean; env?: string | null }) {
  const m = agentMark(a.id);
  return (
    <span className={cx('hp-agent', !a.found && 'missing', def && 'def', small && 'sm')} style={{ ['--ac' as string]: m.color }}>
      <AgentIcon id={a.id} />
      {a.name}
      {env && <i className="hp-agent-env">· {env}</i>}
      {!small && <em>{a.found ? a.version || '找到了' : '没找到'}</em>}
      {def && <small>默认</small>}
    </span>
  );
}

export function Beat({ at, now }: { at: number | null; now: number }) {
  if (at == null) return <span className="hp-beat off">还没有心跳</span>;
  return (
    <span className="hp-beat" title={`最近一次心跳 ${hhmm(at)}`}>
      <i />
      {since(at, now)}
    </span>
  );
}

export const lastSeen = (h: HostInfo, now: number) => (h.online ? `心跳 ${since(h.beatAt, now)}` : `${since(h.offAt ?? h.beatAt, now)}在线`);

/** 机器页打开（或机器详情打开）时每 10 秒刷新一次机器和派发：机器心跳不推送，靠它更新「最近心跳」 */
export function usePollHosts(on: boolean) {
  useEffect(() => {
    if (!on) return;
    const t = window.setInterval(() => void refreshHosts().catch(() => {}), 10000);
    return () => clearInterval(t);
  }, [on]);
}

/** 改一台机器（工作区里的 hosts） */
export const patchHost = (name: string, fn: (h: HostInfo) => HostInfo): Patch => ({
  ws: ws => ({ ...ws, hosts: ws.hosts?.map(h => (h.name === name ? fn(h) : h)) }),
  keys: [hostKey(name)],
});

/** 派发日志：最后 500 行；在跑时每 3 秒取一次 */
export function LogView({ id, live }: { id: number; live?: boolean }) {
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
      {lines === null ? '正在加载…' : lines.length ? lines.join('\n') : '还没有输出（host 每 5 秒上传一次新增的行）'}
      {live && lines !== null && (
        <>
          {'\n'}
          <span className="hp-caret">▍</span>
        </>
      )}
    </pre>
  );
}
