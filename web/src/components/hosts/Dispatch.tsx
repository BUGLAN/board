/* 卡片的派发：白板卡片上的状态行、卡片详情里的派发说明、「派发」对话框、取消派发。
   「未整理」里的条目派出去是拆解（kind = split）：不领取、不移动它，拆出的卡记回未整理（PRD §9.1） */
import { useEffect, useRef, useState } from 'react';
import { openBoard, openCard } from '../../lib/actions';
import { api } from '../../lib/api';
import { judge } from '../../lib/granularity';
import { agentMark, boardOf, cardDispatch, envName, hostDirFor, hostLoad, hostsOf, inSession, isSplit, queueReason, dirReadiness, readyOk, readyWhy, sessionRunning, skillMiss, sourceOf } from '../../lib/hosts';
import type { Dispatch, HostInfo } from '../../lib/types';
import { cx, durWords, hhmm } from '../../lib/util';
import { hold, joinPatch, optimistic, refresh, unqueueCard, useData, type Patch } from '../../store/data';
import { confirm, openDispatch, openHost, openSession, toast, toastError, useUI } from '../../store/ui';
import { Tip, useNow } from '../bits';
import { Icon } from '../Icon';
import { usePresence } from '../overlays/presence';
import { AgentChip, Dot, Mark } from './bits';
import { partState, partTook } from './Session';

/** 打开卡片：切到它的白板，打开详情 */
export function goCard(d: Dispatch) {
  openHost(null);
  openBoard(d.board);
  setTimeout(() => openCard(d.card), 120);
}

export async function cancel(d: Dispatch) {
  const running = d.state === 'running';
  const split = isSplit(d);
  if (running) {
    const yes = await confirm({
      title: split ? `停止拆解 #${d.card}？` : `停止 #${d.card}？`,
      text: split
        ? `${d.host} 会结束 agent 进程（先 SIGTERM，10 秒后强制结束）。这一条留在未整理，已经拆出的卡不动，评论写明是你停止的。`
        : `${d.host} 会结束 agent 进程（先 SIGTERM，10 秒后强制结束），卡片移到需要人处理的状态，评论写明是你停止的。`,
      ok: '停止',
      icon: 'trash',
    });
    if (!yes) return;
  }
  // 当帧生效：排队的从队列里消失、卡片回到原来的可领取状态最上面，在跑的标成「正在停止」（停止按钮随之隐藏，不会重复提交）；
  // 被拒绝时改回来并提示
  const mark: Patch = {
    ws: ws => ({
      ...ws,
      dispatches: running
        ? ws.dispatches?.map(x => (x.id === d.id ? { ...x, stopBy: x.stopBy ?? '你' } : x))
        : ws.dispatches?.filter(x => x.id !== d.id),
    }),
    keys: [`dispatch:${d.id}`],
  };
  const patch = running ? mark : joinPatch(mark, unqueueCard(d.board, d.card));
  try {
    await optimistic(patch, () => api.cancelDispatch(d.id));
    toast({ type: 'success', title: running ? `已通知 ${d.host} 停止 #${d.card}` : split ? `已取消 #${d.card} 的拆解` : `已取消 #${d.card} 的派发` });
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
    <div className="card-dispatch" title={`派发于 ${hhmm(d.createdAt)}（${sourceOf(d)}）`}>
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
  if (!d) return null;
  const queued = d.state === 'queued';
  const split = isSplit(d);
  return (
    <section className="d-section" data-part="dispatch" style={{ ['--i' as string]: 1 }}>
      <div className={cx('dispatch-note', queued && 'queued')} style={{ ['--ac' as string]: agentMark(d.agent).color }}>
        <Mark id={d.agent} title={d.agentLabel} />
        <div className="dn-text">
          <b>
            {split ? (queued ? '排队拆解' : '拆解中') : queued ? '排队' : '在跑'} → {d.host} · {d.agentLabel}
          </b>
          <span>
            {queued ? (
              <>
                {queueReason(h, d)} · {sourceOf(d)}{split ? '拆解' : '派发'} · 排了 {durWords(now - d.createdAt)}
              </>
            ) : (
              <>
                {d.name}
                {d.envName && ` · ${d.envName}`}
                {d.model && ` · ${d.model}`}
                {d.effort && ` · 强度 ${d.effort}`} · {sourceOf(d)}{split ? '拆解' : '派发'} · 已跑 {d.startedAt != null ? durWords(now - d.startedAt) : '0 秒'}
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
          <button className="btn sm ghost" type="button" onClick={() => openSession({ id: d.session || d.id, card: d.card, known: [d] })}>
            <Icon name="log" />
            会话
          </button>
        )}
        {!d.stopBy && (
          <button className="btn sm danger" type="button" onClick={() => cancel(d)}>
            <Icon name={queued ? 'close' : 'stop'} />
            {queued ? (split ? '取消拆解' : '取消派发') : '停止'}
          </button>
        )}
      </div>
    </section>
  );
}

/** 卡片详情「派发记录」：这张卡结束过的每一次派发，新的在前（时间、机器、agent、结果、用时）；点一行打开那个会话、定位到这张卡。
 *  打开时取一次，派发有变化（推送）时再取 */
export function DispatchHistory({ board, card }: { board: string; card: number }) {
  const [list, setList] = useState<Dispatch[] | null>(null);
  const rev = useData(s => s.ws?.rev);
  const active = useData(s => cardDispatch(s.ws, board, card)?.state);
  useEffect(() => {
    let alive = true;
    api
      .cardDispatches(board, card)
      .then(r => alive && setList(r))
      .catch(() => alive && setList(l => l ?? []));
    return () => {
      alive = false;
    };
  }, [board, card, active, rev]);
  const done = (list ?? []).filter(d => d.state === 'done');
  if (!done.length) return null;
  return (
    <section className="d-section" data-part="history" style={{ ['--i' as string]: 1 }}>
      <div className="d-label">
        派发记录 <span className="count">{done.length}</span>
      </div>
      <div className="dh-list">
        {done.map(d => {
          const st = partState(d);
          const took = partTook(d, d.endedAt ?? 0);
          return (
            <button key={d.id} type="button" className="dh-row" onClick={() => openSession({ id: d.session || d.id, card: d.card, known: [d] })} title="打开这个会话">
              <Mark id={d.agent} title={d.agentLabel} />
              <span className="dh-when">{d.endedAt != null ? new Date(d.endedAt).toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : ''}</span>
              <span className="dh-where">
                {d.host} · {d.agentLabel}
              </span>
              <em style={{ ['--c' as string]: st.color }}>{st.label}</em>
              <span className="dh-took">{took != null ? durWords(took) : ''}</span>
              {(d.total ?? 1) > 1 && <span className="dh-seq">会话第 {d.seq}/{d.total} 张</span>}
            </button>
          );
        })}
      </div>
    </section>
  );
}

/** 卡片详情底部「派发」对话框：选机器、选 agent */
export function DispatchDialog() {
  const req = useUI(s => s.dispatchOf);
  const { mounted, on } = usePresence(req != null, 220);
  const last = useRef(req);
  if (req != null) last.current = req;
  if (!mounted || last.current == null) return null;
  const { card, force } = last.current;
  return <DispatchBody key={`${card}-${force}`} card={card} force={force} on={on && req != null} />;
}

/** force：粒度预检判偏小，人点了「强制拆解」仍要拆（default/main#306） */
function DispatchBody({ card, force, on }: { card: number; force: boolean; on: boolean }) {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const entry = useData(s => s.board?.cards[card]);
  const title = entry?.title ?? '';
  // 「未整理」里的条目：派给 agent 拆解
  const split = entry?.column === 'inbox';
  // 粒度预检说这一条已经够细（已经拆出来的子项不再判）
  const small = split && !force && entry?.splitFrom == null ? judge(entry.title, entry.body) : null;
  const hosts = hostsOf(ws);
  const rc = boardOf(ws, path)?.readyCheck;
  // 有工作空间的机器就视为有这块白板的目录（目录状态只在机器详情里提示，不再拒绝派发）
  const hasDir = (h: HostInfo) => !!hostDirFor(h, path);
  // 环境检查没通过（或还没检查）的机器不能选（PRD §9.1）
  const ready = (h: HostInfo) => dirReadiness(rc, h, hostDirFor(h, path));
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
      toast({
        type: 'success',
        title: split ? `#${card} 已派给 ${d.host} · ${d.agentLabel} 拆解` : `#${card} 已派发到 ${d.host} · ${d.agentLabel}`,
        text: busy(h) ? `排队：${queueReason(h)}` : split ? '机器有空位，马上开始；拆出的卡会记到未整理' : '机器有空位，马上开始',
      });
      void hold({ ws: ws => ({ ...ws, dispatches: [...(ws.dispatches ?? []).filter(x => x.id !== d.id), d] }) }, refresh());
    } catch (e) {
      toastError(e);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog dp-dialog" role="dialog" aria-modal="true" aria-label={split ? `派给 agent 拆解 #${card}` : `派发 #${card}`}>
        <h3>{split ? `${force ? '强制' : ''}派给 agent 拆解 #${card}` : `派发 #${card}`}</h3>
        <p className="dp-sub">{title}</p>
        {split && <p className="dp-note dp-split">agent 读这一条，把它拆成几张卡记到未整理、挂在这条下面；这一条不领取、不移动，看过后再整理进工作流。</p>}
        {/* 粒度预检（default/main#306）：够细的一条先别拆，说清楚为什么；强制拆解时提醒 agent 只切真的有先后依赖的部分 */}
        {(small || (split && force)) && (
          <p className="dp-note dp-size" role="status">
            {small ? (
              <>
                <b>任务粒度合适，不用拆</b>
                （{small.why}）：这一条自己就能做完、单独复盘。点「强制拆解」仍可派给 agent，提示词里会告诉它只在真的有先后依赖时才切。
              </>
            ) : (
              <>强制拆解：这条预检判过够细，agent 会再判一遍，只在真的有先后依赖时才切；能整个做就保留原样、不建卡。</>
            )}
          </p>
        )}
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
                      <em>还没有配工作空间</em>
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
                    <Tip key={a.id} text={a.found ? '' : a.env ? `在${envName(h, a.env)}里没找到命令` : '这台机器上没找到命令'}>
                      <button type="button" className={cx('hp-agent-btn', agent === a.id && 'on')} disabled={!a.found} onClick={() => setAgent(a.id)}>
                        <AgentChip a={a} def={a.id === h.agent} env={envName(h, a.env)} skill={skillMiss(h, a)} />
                      </button>
                    </Tip>
                  ))}
                </div>
                {miss && (
                  <p className="dp-note" role="status">
                    {miss}
                  </p>
                )}
                {busy(h) && <p className="dp-note">{h.name} {!h.online ? '离线' : h.paused ? '暂停接单' : '没有空位'}：{split ? '拆解' : '派发'}会排队，{queueReason(h)}后开始。</p>}
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
            {split ? (force ? '强制拆解' : '派给 agent 拆解') : '派发'}
          </button>
        </div>
      </div>
    </div>
  );
}
