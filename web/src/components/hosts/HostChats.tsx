/* ==========================================================================
   机器详情「对话」页签（PRD §9.1「机器对话」）：左栏对话列表，右栏选中的对话或新对话
   - 左栏：最上面「新对话」（机器离线时禁用），下面是这台机器上的对话（一直留着，最近有动静的在前）：
     第一条消息、角色或 agent、回答中 / 多久前
   - 右栏：选中的对话直接在这里聊（不弹窗）。头部角色、模型、启动环境、开始时间、回答中 / 空闲，过程 / 原始输出；
     输出在底部时跟着新内容，往上翻时不动；底部输入框（Enter 发送、Shift+Enter 换行）
   - 发消息走乐观更新（AGENTS.md「界面写操作」）：消息当帧画在输出里、标「发送中」，这一轮当帧变成「回答中」；
     失败时撤回、消息退回输入框并提示原因。这一轮在跑时不能发，旁边「停止这一轮」（要等服务端返回，按钮转圈）
   - 机器离线：输入框禁用，写「<机器> 离线，上线后才能接着发」；对话不结束
   - 还没有对话、或者点了「新对话」：右栏是 NewChatForm
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { hostsOf, since } from '../../lib/hosts';
import { mergeEvents, groupEvents, runningTool } from '../../lib/runEvents';
import type { Chat, ChatView, HostInfo, Workspace } from '../../lib/types';
import { cx, hhmm } from '../../lib/util';
import { chatPromptPatch, onChatRuns, optimistic, useData } from '../../store/data';
import { toastError } from '../../store/ui';
import { Segmented } from '../bits';
import { Icon } from '../Icon';
import { Mark } from './bits';
import { NewChatForm } from './NewChat';
import { EventRow, PromptRow, rowKey, SegBodySkeleton } from './Session';

export type ChatSel = number | 'new' | null;

const NO_CHATS: Chat[] = [];

/** 一台机器上的对话（工作区里带着，最近有动静的在前） */
export function chatsOf(ws: Pick<Workspace, 'chats'> | null | undefined, hostId: number): Chat[] {
  const all = ws?.chats;
  if (!all?.length) return NO_CHATS;
  return all.filter(c => c.hostId === hostId).sort((a, b) => b.lastAt - a.lastAt || b.id - a.id);
}

/** 对话用什么跑：角色名，没有角色时 agent 名 */
export const chatWho = (c: Pick<Chat, 'profileName' | 'agentLabel' | 'agent'>) => c.profileName || c.agentLabel || c.agent;

export function HostChats({ h, chats, sel, setSel, now }: { h: HostInfo; chats: Chat[]; sel: ChatSel; setSel: (s: ChatSel) => void; now: number }) {
  // 没选时：有对话就选最近的，没有就是新对话
  const cur = sel === 'new' ? null : (chats.find(c => c.id === sel) ?? (sel == null ? chats[0] : undefined));
  return (
    <div className="hc">
      <nav className="hc-list" aria-label="这台机器上的对话">
        <button type="button" className={cx('hc-new', !cur && 'on')} disabled={!h.online} onClick={() => setSel('new')} title={h.online ? undefined : '机器离线'}>
          <Icon name="plus" />
          新对话
        </button>
        <div className="d-label hc-label">
          对话 <span className="count">{chats.length || ''}</span>
        </div>
        {chats.length === 0 && <p className="hp-note hc-empty">还没有对话</p>}
        {chats.map(c => {
          const busy = c.turn === 'busy';
          return (
            <button key={c.id} type="button" className={cx('hc-item', cur?.id === c.id && 'on')} aria-current={cur?.id === c.id || undefined} onClick={() => setSel(c.id)}>
              <Mark id={c.agent} title={c.agentLabel} />
              <span className="hc-item-text">
                <b>{c.title}</b>
                <small>
                  {chatWho(c)} · <em className={cx(busy && 'on')}>{busy ? '回答中' : since(c.lastAt, now)}</em>
                </small>
              </span>
            </button>
          );
        })}
      </nav>
      <div className="hc-main">{cur ? <ChatPane key={cur.id} c={cur} /> : <NewChatForm key="new" h={h} onStarted={id => setSel(id)} />}</div>
    </div>
  );
}

/** 取一个对话的内容：第一次全取，之后带 since 只取新增、改过的事件。推送里有这个对话的新事件、kick() 时马上再取；
 *  这一轮在跑时另外每 5 秒取一次。同一时刻只有一个请求 */
export function useChatView(id: number, busy: boolean, turnKey: string) {
  const [view, setView] = useState<ChatView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [kick, setKick] = useState(0);
  const runs = useRef(0);
  const ref = useRef<ChatView | null>(null);
  useEffect(() => {
    let alive = true;
    let inflight = false;
    let again = false;
    const load = async () => {
      if (inflight) {
        again = true;
        return;
      }
      inflight = true;
      try {
        do {
          again = false;
          const prev = ref.current;
          const since = prev ? runs.current : 0;
          try {
            const v = await api.chat(id, since);
            if (!alive) return;
            const next = prev && since > 0 ? { ...v, events: mergeEvents(prev.events, v.events, v.eventsFrom) } : v;
            ref.current = next;
            runs.current = v.runs ?? 0;
            setView(next);
            setError(null);
          } catch (e) {
            if (alive) setError(e instanceof Error ? e.message : String(e));
          }
        } while (again && alive);
      } finally {
        inflight = false;
      }
    };
    void load();
    const t = busy ? window.setInterval(load, 5000) : 0;
    const off = onChatRuns(ids => {
      if (ids.includes(id)) void load();
    });
    return () => {
      alive = false;
      clearInterval(t);
      off();
    };
  }, [id, busy, turnKey, kick]);
  return { view, error, kick: () => setKick(k => k + 1) };
}

/** 发出去、还没在取回来的事件里出现的消息 */
interface Sent {
  key: number;
  text: string;
  at: number;
  /** 发送时已经有几条人发的消息 */
  before: number;
}

/** 右栏的一个对话：头部、输出、输入框 */
function ChatPane({ c }: { c: Chat }) {
  const busy = c.turn === 'busy';
  const { view, error, kick } = useChatView(c.id, busy, `${c.turn}${c.lastAt}`);
  const [mode, setMode] = useState<'events' | 'raw'>('events');
  const [sent, setSent] = useState<Sent[]>([]);
  const events = view?.events ?? [];
  const prompts = events.filter(e => e.kind === 'prompt').length;
  // 取回来的事件里已经有了：不再画「发送中」
  useEffect(() => {
    if (sent.some(x => prompts > x.before)) setSent(s => s.filter(x => prompts <= x.before));
  }, [prompts, sent]);
  // 在底部时跟着新内容；往上翻了就不动
  const out = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const size = events.length + events.filter(e => e.ok != null).length + (view?.log.length ?? 0) + sent.length;
  useLayoutEffect(() => {
    const el = out.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [size, mode, !!view]);
  const rows = groupEvents(events);
  const now = runningTool(events, busy);
  return (
    <>
      <div className="hc-head">
        <Mark id={c.agent} title={c.agentLabel} />
        <b>{chatWho(c)}</b>
        <span className="hc-meta">
          {(c.model || c.effort) && <span>{[c.model, c.effort && `强度 ${c.effort}`].filter(Boolean).join(' · ')}</span>}
          {c.envName && <span>{c.envName}</span>}
          <span>主目录</span>
          <span>开始于 {hhmm(c.createdAt)}</span>
          <span className="ss-state" style={{ ['--c' as string]: busy ? 'var(--c-doing)' : 'var(--text-3)' }}>
            {busy ? '回答中' : '空闲'}
          </span>
        </span>
        <span className="ss-modes">
          <Segmented
            value={mode}
            options={[
              ['events', '过程'],
              ['raw', '原始输出'],
            ]}
            onChange={setMode}
          />
        </span>
      </div>
      <div
        className="hc-out"
        ref={out}
        aria-busy={!view || undefined}
        onScroll={e => {
          const el = e.currentTarget;
          stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 12;
        }}
      >
        {error && <p className="hp-note ss-err">{error}</p>}
        {!view && !error ? (
          <SegBodySkeleton i={0} />
        ) : mode === 'raw' ? (
          <pre className="hp-log ss-log">
            {view?.log.length ? view.log.join('\n') : '还没有输出'}
            {busy && (
              <>
                {'\n'}
                <span className="hp-caret">▍</span>
              </>
            )}
          </pre>
        ) : (
          <div className="ev-list ss-chat">
            {rows.map(r => (
              <EventRow key={rowKey(r)} r={r} events={events} running={busy} now={now} chat />
            ))}
            {sent.map(x => (
              <PromptRow key={`sent${x.key}`} text={x.text} at={x.at} sending />
            ))}
            {(busy || sent.length > 0) && <span className="hp-caret ev-caret">▍</span>}
          </div>
        )}
      </div>
      <Composer c={c} prompts={prompts} sent={sent} setSent={setSent} kick={kick} />
    </>
  );
}

/** 对话的底部：输入框（Enter 发送、Shift+Enter 换行）。这一轮在跑、或者上一条还没确认时不能发，旁边「停止这一轮」；
 *  机器离线时禁用 */
function Composer({ c, prompts, sent, setSent, kick }: { c: Chat; prompts: number; sent: Sent[]; setSent: (f: (s: Sent[]) => Sent[]) => void; kick: () => void }) {
  const [draft, setDraft] = useState('');
  const [stopping, setStopping] = useState(false);
  const online = useData(s => !!hostsOf(s.ws).find(h => h.id === c.hostId)?.online);
  const box = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [draft]);
  const answering = c.turn === 'busy';
  const busy = answering || sent.length > 0;
  const send = async () => {
    const text = draft.trim();
    if (!text || busy || !online) return;
    const at = Date.now();
    const item: Sent = { key: at, text, at, before: prompts };
    setSent(s => [...s, item]);
    setDraft('');
    try {
      await optimistic(chatPromptPatch(c.id, at), () => api.chatPrompt(c.id, text));
      kick();
    } catch (e) {
      setSent(s => s.filter(x => x.key !== item.key));
      setDraft(cur => cur || text);
      toastError(e);
    }
  };
  const stop = async () => {
    setStopping(true);
    try {
      await optimistic(null, () => api.chatCancel(c.id));
      kick();
    } catch (e) {
      toastError(e);
    } finally {
      setStopping(false);
    }
  };
  return (
    <div className={cx('ss-compose', busy && 'busy', !online && 'off')}>
      <textarea
        ref={box}
        className="ss-input"
        rows={1}
        value={draft}
        disabled={!online}
        placeholder={!online ? `${c.host} 离线，上线后才能接着发` : busy ? '等这一轮结束再发' : `发给 ${chatWho(c)}`}
        aria-label="消息"
        onChange={e => setDraft(e.target.value)}
        onKeyDown={e => {
          if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            void send();
          }
        }}
      />
      <div className="ss-compose-bar">
        <span className="ss-hint">{!online ? '机器离线' : sent.length ? '发送中…' : answering ? `${chatWho(c)} 正在回答` : 'Enter 发送 · Shift+Enter 换行'}</span>
        {answering && online && (
          <button className={cx('btn sm ghost', stopping && 'is-busy')} type="button" disabled={stopping || sent.length > 0} onClick={() => void stop()}>
            <Icon name="stop" />
            停止这一轮
          </button>
        )}
        <button className="btn sm brand" type="button" disabled={busy || !online || !draft.trim()} onClick={() => void send()}>
          <Icon name="send" />
          发送
        </button>
      </div>
    </div>
  );
}
