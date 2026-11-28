/* 卡片的派发：白板卡片上的状态行、卡片详情里的派发说明、「派发」对话框（单卡和批量）、取消派发。
   「未整理」里的条目派出去是拆解（kind = split）：不领取、不移动它，拆出的卡记回未整理（PRD §9.1） */
import { profilesOf } from '../../lib/profiles';
import { DispatchFlow } from '../profiles/DispatchFlow';
import { useEffect, useRef, useState } from 'react';
import { openBoard, openCard } from '../../lib/actions';
import { api } from '../../lib/api';
import { bulkDispatchToast, dispatchPlan, runBulkDispatch } from '../../lib/boardSelect';
import { agentMark, agentUsable, agentWhy, blockOf, cardDispatch, dispatchRoute, envName, hostLoad, hostWords, hostsOf, inSession, isSplit, queueReason, sessionRunning, skillMiss, sourceOf } from '../../lib/hosts';
import type { Dispatch, HostInfo } from '../../lib/types';
import { cx, durWords, hhmm } from '../../lib/util';
import { joinPatch, optimistic, unqueueCard, useData, type Patch } from '../../store/data';
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

/** 白板卡片右上角：派发还在排队时的角标「[agent 图标] → <机器>」（和自动领取的去向角标同一种样子，见 profiles/Next.tsx CardRoute）；
    整句「排队 → <机器> · <agent>（原因）」和派发时间在悬停提示里，会话里排着的原因是「会话第 N/M 张」 */
export function CardDispatchBadge({ card }: { card: number }) {
  const d = useData(s => cardDispatch(s.ws, s.path, card));
  const h = useData(s => (d ? hostsOf(s.ws).find(x => x.id === d.hostId) : undefined));
  const block = useData(s => (d && d.host ? blockOf(s.ws, d.host, d.board) : undefined));
  if (!d || d.state !== 'queued') return null;
  const say = `排队 → ${hostWords(d)} · ${d.agentLabel}（${queueReason(h, d, block)}）· 派发于 ${hhmm(d.createdAt)}（${sourceOf(d)}）`;
  return (
    <span className="card-where t-sent" title={say} aria-label={say}>
      <Mark id={d.agent} title={say} />
      <span>→ {hostWords(d)}</span>
    </span>
  );
}

/** 卡片详情里：排队或正在跑的派发，以及「取消派发」/「停止」。会话里排着的，下面一行写会话的名字和它正在做的卡片 */
export function DispatchNote({ board, card }: { board: string; card: number }) {
  const d = useData(s => cardDispatch(s.ws, board, card));
  const h = useData(s => (d ? hostsOf(s.ws).find(x => x.id === d.hostId) : undefined));
  const lead = useData(s => (d && inSession(d) ? sessionRunning(s.ws, d) : undefined));
  const block = useData(s => (d && d.host && d.state === 'queued' ? blockOf(s.ws, d.host, d.board) : undefined));
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
            {split ? (queued ? '排队拆解' : '拆解中') : queued ? '排队' : '在跑'} → {hostWords(d)} · {d.agentLabel}
          </b>
          <span>
            {queued ? (
              <>
                {queueReason(h, d, block)} · {sourceOf(d)}{split ? '拆解' : '派发'} · 排了 {durWords(now - d.createdAt)}
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
        {queued && !inSession(d) && (
          <button className="btn sm ghost" type="button" onClick={() => openDispatch(d.card, { move: d.id })}>
            <Icon name="move" />
            改派
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
                {hostWords(d)} · {d.agentLabel}
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

/** 「派发」对话框（卡片详情的单卡、白板批量条的多张）：有角色时选角色和机器（profiles/DispatchFlow.tsx），没有时选机器、选 agent */
export function DispatchDialog() {
  const req = useUI(s => s.dispatchOf);
  const { mounted, on } = usePresence(req != null, 220);
  const last = useRef(req);
  if (req != null) last.current = req;
  if (!mounted || last.current == null) return null;
  const { cards, move } = last.current;
  const key = `${cards.join(',')}-${move ?? ''}`;
  // 有角色时单卡、批量都按角色派（分工页定的那一套）；还没有角色时选机器和 agent
  if (profilesOf(useData.getState().ws).length) return <DispatchFlow key={key} cards={cards} move={move} on={on && req != null} />;
  return <DispatchBody key={key} cards={cards} move={move} on={on && req != null} />;
}

function DispatchBody({ cards, move, on }: { cards: number[]; move?: number; on: boolean }) {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const board = useData(s => s.board);
  const single = cards.length === 1 ? cards[0] : null;
  const entry = single != null ? board?.cards[single] : undefined;
  const title = entry?.title ?? '';
  // 「未整理」里的条目：派给 agent 拆解
  const split = entry?.column === 'inbox';
  // 不在可领取状态（待审核、错误节点……）：排队时一步移进锁定状态（#498），这里写明会从哪里移出
  const statuses = board?.statuses;
  const route = entry && statuses && move == null ? dispatchRoute(statuses, entry.column, entry.holder) : null;
  const active = new Set((ws?.dispatches ?? []).filter(d => d.board === path && d.state !== 'done').map(d => d.card));
  const plan = single == null ? dispatchPlan(statuses ?? [], board?.cards ?? {}, cards, active) : null;
  const skipped = plan?.skipped ?? [];
  const hosts = hostsOf(ws);
  // 接不了这块白板的机器不能选（PRD §9.1「能不能接」）：没有工作空间、目录不对。
  // 原因是服务端给的（工作区数据里的 blocks），这里不自己再判断
  const block = (h: HostInfo) => blockOf(ws, h.name, path);
  const usable = (h: HostInfo) => !block(h);
  // 预先选中的 agent：机器的默认 agent 能用就是它，否则第一个能用的
  const agentOf = (h: HostInfo | null | undefined) => h?.agents.find(a => a.id === h.agent && agentUsable(a))?.id ?? h?.agents.find(agentUsable)?.id ?? null;
  const busy = (h: HostInfo) => !h.online || h.paused || hostLoad(ws, h).running.length >= h.max;
  // 能接这块白板、又有能用的 agent 的机器：「任意机器」就在它们里面挑
  const takers = hosts.filter(x => usable(x) && x.agents.some(agentUsable));
  // 默认选中：两台以上能接时是「任意机器」（服务端挑空位最多的）；只有一台时就是它，在线、没满、没暂停的优先
  const first = hosts.filter(usable).sort((a, b) => Number(busy(a)) - Number(busy(b)))[0];
  // host：机器名；'' = 任意机器；null = 还没有能选的
  const [host, setHost] = useState<string | null>(takers.length >= 2 ? '' : (first?.name ?? null));
  const any = host === '';
  const h = hosts.find(x => x.name === host) ?? null;
  // agent：标识；任意机器时 '' = 挑中那台的默认 agent
  const [agent, setAgent] = useState<string | null>(any ? '' : agentOf(h));
  // 任意机器时能选的 agent：至少在一台能接的机器上能用的
  const anyAgents = takers.flatMap(x => x.agents.filter(agentUsable)).filter((a, i, all) => all.findIndex(b => b.id === a.id) === i);
  const idle = takers.filter(x => !busy(x)).length;
  const chosen = h?.agents.find(x => x.id === agent);
  // 看不到 board-skills 的 agent 照样能派，只提醒
  const miss = h && chosen ? skillMiss(h, chosen) : null;
  const [sending, setSending] = useState(false);
  const close = () => openDispatch(null);
  const heading = move != null ? `改派 #${single}` : split ? `拆解 #${single}` : single != null ? `派发 #${single}` : `派发 ${cards.length} 张卡片`;

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
    setAgent(agentOf(x));
  };

  const submit = async () => {
    if (any ? agent == null : !h || !agent) return;
    if (plan && !plan.ids.length) return;
    setSending(true);
    try {
      // 派发要服务端排进队列才有记录：按钮进行中（转圈、禁用）直到请求返回、随后的刷新也完成，机器页的旧轮询不会把派发画回去
      const to = any ? '' : h!.name;
      if (single != null) {
        const d = await optimistic(null, () =>
          move != null ? api.moveDispatch(move, to, agent || undefined) : api.dispatch(path, single, to, agent || undefined),
        );
        close();
        const got = hosts.find(x => x.name === d.host);
        toast({
          type: 'success',
          title: move != null ? `#${single} 已改派到 ${hostWords(d)} · ${d.agentLabel}` : split ? `#${single} 已派给 ${hostWords(d)} · ${d.agentLabel} 拆解` : `#${single} 已派发到 ${hostWords(d)} · ${d.agentLabel}`,
          text: !got ? '排队：等一台能接的机器' : busy(got) ? `排队：${queueReason(got)}` : split ? '机器有空位，马上开始；拆出的卡会记到未整理' : '机器有空位，马上开始',
        });
        return;
      }

      // 批量：按选中顺序逐张发；单张失败只记下来，后面的照常发。全部返回后才关对话框
      const ids = plan!.ids;
      const result = await optimistic(null, () => runBulkDispatch(ids, id => api.dispatch(path, id, to, agent || undefined)));
      close();
      const where = !any && result.ok[0] ? `派给 ${hostWords(result.ok[0])} · ${result.ok[0].agentLabel}` : '';
      toast(bulkDispatchToast(result, skipped, where));
    } catch (e) {
      toastError(e);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog dp-dialog" role="dialog" aria-modal="true" aria-label={heading}>
        <h3>{heading}</h3>
        <p className="dp-sub">{single != null ? title : `共 ${cards.length} 张卡片，逐张排队派发`}</p>
        {route?.from && route.into && (
          <p className="dp-note dp-split">
            会从「{route.from.name}」移出，直接在「{route.into.name}」排队，不经过可领取状态；排队时取消派发，回到「{route.from.name}」。
          </p>
        )}
        {split && <p className="dp-note dp-split">派给一台机器上的 agent：它读这一条，拆成几张能直接做的卡（够小的整理成一张）记到未整理、挂在这条下面；这一条不领取、不移动，看过后再整理进工作流。</p>}
        {plan && (
          <p className={cx('dp-note', 'dp-plan', !plan.ids.length && 'bad')} role="status">
            {plan.ids.length ? `可派发 ${plan.ids.length} 张` : '没有可派发的卡片'}
            {skipped.length ? `；跳过 ${skipped.length} 张：${skipped.map(x => `#${x.id} ${x.why}`).join('；')}` : ''}
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
              <button
                type="button"
                role="radio"
                aria-checked={any}
                className={cx('dp-host', any && 'on')}
                disabled={takers.length === 0}
                onClick={() => {
                  setHost('');
                  setAgent('');
                }}
              >
                <Icon name="dispatch" />
                <b>任意机器</b>
                <span className="hp-sp" />
                {takers.length === 0 ? <em>没有能接这块白板的机器</em> : idle > 0 ? <em className="ok">{idle} 台有空位，挑最空的</em> : <em>会排队，谁先空出来给谁</em>}
              </button>
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
                    {!ok ? (
                      <em className="bad" title={block(x)?.why}>
                        {block(x)?.why}
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
            {any && (
              <>
                <div className="dp-label">agent</div>
                <div className="hp-agents">
                  {/* 怎么挑机器的说明不再常驻一行，悬浮 / 聚焦「机器的默认 agent」时提示（default/main#496） */}
                  <Tip text={`服务端在能接这块白板的 ${takers.length} 台机器里挑空位最多的一台，一样多时轮流；现在都没有空位就排着，分到的机器离线、暂停接单时自动换一台。`}>
                    <button type="button" className={cx('hp-agent-btn', agent === '' && 'on')} onClick={() => setAgent('')}>
                      <span className="hp-agent dp-agent-def">机器的默认 agent</span>
                    </button>
                  </Tip>
                  {anyAgents.map(a => (
                    <button key={a.id} type="button" className={cx('hp-agent-btn', agent === a.id && 'on')} onClick={() => setAgent(a.id)}>
                      <AgentChip a={a} />
                    </button>
                  ))}
                </div>
              </>
            )}
            {h && (
              <>
                <div className="dp-label">agent</div>
                <div className="hp-agents">
                  {h.agents.map(a => (
                    <Tip key={a.id} text={agentWhy(h, a)}>
                      <button type="button" className={cx('hp-agent-btn', agent === a.id && 'on')} disabled={!agentUsable(a)} onClick={() => setAgent(a.id)}>
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
          <button
            className={cx('btn brand', sending && 'is-busy')}
            type="button"
            disabled={sending || (any ? takers.length === 0 : !h || !agent || !usable(h)) || (plan != null && !plan.ids.length)}
            aria-busy={sending || undefined}
            onClick={submit}
          >
            <Icon name="dispatch" />
            {move != null ? '改派' : split ? '拆解' : single != null ? '派发' : `派送 ${plan?.ids.length ?? cards.length} 张`}
          </button>
        </div>
      </div>
    </div>
  );
}
