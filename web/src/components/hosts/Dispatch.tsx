/* 卡片的派发：白板卡片上的状态行、卡片详情里的派发说明、「派发」对话框、取消派发 */
import { useEffect, useRef, useState } from 'react';
import { openBoard, openCard } from '../../lib/actions';
import { api } from '../../lib/api';
import { agentMark, boardOf, cardDispatch, envName, hostLoad, hostsOf, inSession, queueReason, dirReadiness, readyOk, readyWhy, sessionRunning, skillMiss, SOURCE } from '../../lib/hosts';
import type { Dispatch, HostInfo } from '../../lib/types';
import { cx, durWords, hhmm } from '../../lib/util';
import { hold, optimistic, refresh, useData, type Patch } from '../../store/data';
import { confirm, openDispatch, openHost, toast, toastError, useUI } from '../../store/ui';
import { useNow } from '../bits';
import { Icon } from '../Icon';
import { usePresence } from '../overlays/presence';
import { AgentChip, Dot, LogView, Mark } from './bits';

/** 打开卡片：切到它的白板，打开详情 */
export function goCard(d: Dispatch) {
  openHost(null);
  openBoard(d.board);
  setTimeout(() => openCard(d.card), 120);
}

export async function cancel(d: Dispatch) {
  const running = d.state === 'running';
  if (running) {
    const yes = await confirm({
      title: `停止 #${d.card}？`,
      text: `${d.host} 会结束 agent 进程（先 SIGTERM，10 秒后强制结束），卡片移到需要人处理的状态，评论写明是你停止的。`,
      ok: '停止',
      icon: 'trash',
    });
    if (!yes) return;
  }
  // 当帧生效：排队的从队列里消失，在跑的标成「正在停止」（停止按钮随之隐藏，不会重复提交）；被拒绝时改回来并提示
  const patch: Patch = {
    ws: ws => ({
      ...ws,
      dispatches: running
        ? ws.dispatches?.map(x => (x.id === d.id ? { ...x, stopBy: x.stopBy ?? '你' } : x))
        : ws.dispatches?.filter(x => x.id !== d.id),
    }),
    keys: [`dispatch:${d.id}`],
  };
  try {
    await optimistic(patch, () => api.cancelDispatch(d.id));
    toast({ type: 'success', title: running ? `已通知 ${d.host} 停止 #${d.card}` : `已取消 #${d.card} 的派发` });
  } catch (e) {
    toastError(e);
  }
}

/** 卡片上：排队时写「排队 → <机器> · <agent>（原因）」；会话里排着的原因是「会话第 N/M 张」 */
export function CardDispatchLine({ card }: { card: number }) {
  const d = useData(s => cardDispatch(s.ws, s.path, card));
  const h = useData(s => (d ? hostsOf(s.ws).find(x => x.id === d.hostId) : undefined));
  if (!d || d.state !== 'queued') return null;
  return (
    <div className="card-dispatch" title={`派发于 ${hhmm(d.createdAt)}（${SOURCE[d.source]}）`}>
      <Icon name="dispatch" />
      <span>
        排队 → {d.host} · {d.agentLabel}
        <em>（{queueReason(h, d)}）</em>
      </span>
    </div>
  );
}

/** 卡片详情里：排队或正在跑的派发，以及「取消派发」/「停止」。会话里排着的，下面一行写会话的名字和它正在做的卡片 */
export function DispatchNote({ board, card }: { board: string; card: number }) {
  const d = useData(s => cardDispatch(s.ws, board, card));
  const h = useData(s => (d ? hostsOf(s.ws).find(x => x.id === d.hostId) : undefined));
  const lead = useData(s => (d && inSession(d) ? sessionRunning(s.ws, d) : undefined));
  const now = useNow(1000);
  const [log, setLog] = useState(false);
  if (!d) return null;
  const queued = d.state === 'queued';
  return (
    <section className="d-section" data-part="dispatch" style={{ ['--i' as string]: 1 }}>
      <div className={cx('dispatch-note', queued && 'queued')} style={{ ['--ac' as string]: agentMark(d.agent).color }}>
        <Mark id={d.agent} title={d.agentLabel} />
        <div className="dn-text">
          <b>
            {queued ? '排队' : '在跑'} → {d.host} · {d.agentLabel}
          </b>
          <span>
            {queued ? (
              <>
                {queueReason(h, d)} · {SOURCE[d.source]}派发 · 排了 {durWords(now - d.createdAt)}
              </>
            ) : (
              <>
                {d.name}
                {d.envName && ` · ${d.envName}`}
                {d.model && ` · ${d.model}`}
                {d.effort && ` · 强度 ${d.effort}`} · {SOURCE[d.source]}派发 · 已跑 {d.startedAt != null ? durWords(now - d.startedAt) : '0 秒'}
                {d.stopBy && ' · 正在停止'}
              </>
            )}
          </span>
          {lead && (
            <span className="dn-lead">
              {lead.name} 正在做 #{lead.card}
            </span>
          )}
        </div>
        {!queued && (
          <button className="btn sm ghost" type="button" onClick={() => setLog(!log)}>
            <Icon name="log" />
            {log ? '收起日志' : '日志'}
          </button>
        )}
        {!d.stopBy && (
          <button className="btn sm danger" type="button" onClick={() => cancel(d)}>
            <Icon name={queued ? 'close' : 'stop'} />
            {queued ? '取消派发' : '停止'}
          </button>
        )}
      </div>
      {log && !queued && <LogView id={d.id} live />}
    </section>
  );
}

/** 卡片详情底部「派发」对话框：选机器、选 agent */
export function DispatchDialog() {
  const card = useUI(s => s.dispatchOf);
  const { mounted, on } = usePresence(card != null, 220);
  const last = useRef(card);
  if (card != null) last.current = card;
  if (!mounted || last.current == null) return null;
  return <DispatchBody key={last.current} card={last.current} on={on && card != null} />;
}

function DispatchBody({ card, on }: { card: number; on: boolean }) {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const title = useData(s => s.board?.cards[card]?.title ?? '');
  const hosts = hostsOf(ws);
  const rc = boardOf(ws, path)?.readyCheck;
  const hasDir = (h: HostInfo) => h.dirs.some(d => d.board === path);
  // 环境检查没通过（或还没检查）的机器不能选（PRD §9.1）
  const ready = (h: HostInfo) => dirReadiness(rc, h, h.dirs.find(d => d.board === path));
  const usable = (h: HostInfo) => hasDir(h) && readyOk(ready(h));
  const busy = (h: HostInfo) => !h.online || h.paused || hostLoad(ws, h).running.length >= h.max;
  // 默认选中：有这块白板目录的机器里，在线、没满、没暂停的优先
  const first = hosts.filter(usable).sort((a, b) => Number(busy(a)) - Number(busy(b)))[0];
  const [host, setHost] = useState<string | null>(first?.name ?? null);
  const h = hosts.find(x => x.name === host) ?? null;
  const [agent, setAgent] = useState<string | null>(h?.agent ?? null);
  const chosen = h?.agents.find(x => x.id === agent);
  // 看不到 board-skills 的 agent 照样能派，只提醒
  const miss = h && chosen ? skillMiss(h, chosen) : null;
  const [sending, setSending] = useState(false);
  const close = () => openDispatch(null);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        e.preventDefault();
        close();
      }
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, []);

  const pick = (x: HostInfo) => {
    setHost(x.name);
    setAgent(x.agent);
  };

  const submit = async () => {
    if (!h || !agent) return;
    setSending(true);
    try {
      // 派发要服务端排进队列才有记录：按钮进行中（转圈、禁用）直到返回；返回后当即在卡片和机器页上画出这条派发，不等刷新
      const d = await api.dispatch(path, card, h.name, agent);
      close();
      toast({ type: 'success', title: `#${card} 已派发到 ${d.host} · ${d.agentLabel}`, text: busy(h) ? `排队：${queueReason(h)}` : '机器有空位，马上开始' });
      void hold({ ws: ws => ({ ...ws, dispatches: [...(ws.dispatches ?? []).filter(x => x.id !== d.id), d] }) }, refresh());
    } catch (e) {
      toastError(e);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog dp-dialog" role="dialog" aria-modal="true" aria-label={`派发 #${card}`}>
        <h3>派发 #{card}</h3>
        <p className="dp-sub">{title}</p>
        {hosts.length === 0 ? (
          <div className="dp-empty">
            <p>还没有机器。在要干活的机器上运行 board host，它会出现在这里。</p>
            <button
              className="btn sm"
              type="button"
              onClick={() => {
                close();
                openHost('new');
              }}
            >
              <Icon name="plus" />
              添加机器
            </button>
          </div>
        ) : (
          <>
            <div className="dp-label">机器</div>
            <div className="dp-hosts" role="radiogroup" aria-label="机器">
              {hosts.map(x => {
                const ok = usable(x);
                const run = hostLoad(ws, x).running.length;
                const why = !x.online ? '离线' : x.paused ? '暂停接单' : run >= x.max ? '已满' : '';
                return (
                  <button
                    key={x.id}
                    type="button"
                    role="radio"
                    aria-checked={host === x.name}
                    className={cx('dp-host', host === x.name && 'on')}
                    disabled={!ok}
                    onClick={() => pick(x)}
                  >
                    <Dot h={x} running={run} />
                    <b>{x.name}</b>
                    <span className="dp-load">
                      {run}/{x.max}
                    </span>
                    {x.paused && <span className="dp-tag">暂停接单</span>}
                    {!x.online && <span className="dp-tag">离线</span>}
                    <span className="hp-sp" />
                    {!hasDir(x) ? (
                      <em>没有这块白板的目录</em>
                    ) : !ok ? (
                      <em className="bad" title={readyWhy(ready(x))}>
                        {readyWhy(ready(x))}
                      </em>
                    ) : why ? (
                      <em>会排队</em>
                    ) : (
                      <em className="ok">有空位</em>
                    )}
                  </button>
                );
              })}
            </div>
            {h && (
              <>
                <div className="dp-label">agent</div>
                <div className="hp-agents">
                  {h.agents.map(a => (
                    <button
                      key={a.id}
                      type="button"
                      className={cx('hp-agent-btn', agent === a.id && 'on')}
                      disabled={!a.found}
                      onClick={() => setAgent(a.id)}
                      data-tip={a.found ? undefined : a.env ? `在${envName(h, a.env)}里没找到命令` : '这台机器上没找到命令'}
                    >
                      <AgentChip a={a} def={a.id === h.agent} env={envName(h, a.env)} skill={skillMiss(h, a)} />
                    </button>
                  ))}
                </div>
                {miss && (
                  <p className="dp-note" role="status">
                    {miss}
                  </p>
                )}
                {busy(h) && <p className="dp-note">{h.name} {!h.online ? '离线' : h.paused ? '暂停接单' : '没有空位'}：派发会排队，{queueReason(h)}后开始。</p>}
              </>
            )}
          </>
        )}
        <div className="dialog-actions">
          <button className="btn" type="button" onClick={close}>
            取消
          </button>
          <button className={cx('btn brand', sending && 'is-busy')} type="button" disabled={!h || !agent || sending || !usable(h)} aria-busy={sending || undefined} onClick={submit}>
            <Icon name="dispatch" />
            派发
          </button>
        </div>
      </div>
    </div>
  );
}
