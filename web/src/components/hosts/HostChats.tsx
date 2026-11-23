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
   - 右上角一个图标按钮「放大 / 还原」（ChatFull）：放大时桌面盖住整个机器详情弹窗，手机盖住整个屏幕（挂在 body 上、高度跟
     visualViewport），Esc 先还原；选择记在 localStorage，手机默认放大、桌面默认还原
   - 手机（COMPACT：窄屏或横屏矮屏）：列表和聊天二选一，没选时是列表；聊天头部 ← 回列表（default/main#623）
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { hostsOf, since } from '../../lib/hosts';
import { mergeEvents, groupEvents, runningTool } from '../../lib/runEvents';
import type { Chat, ChatView, HostInfo, Workspace } from '../../lib/types';
import { cx, hhmm } from '../../lib/util';
import { chatPromptPatch, onChatRuns, optimistic, useData } from '../../store/data';
import { toastError, useUI } from '../../store/ui';
import { Segmented, Tip } from '../bits';
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

/** 对话按手机来排：窄屏，或者手机横屏（矮屏）。和 hosts.css 对话那段的媒体查询一致 */
const COMPACT = '(max-width: 760px), (max-height: 520px)';
function useCompact() {
  const [m, setM] = useState(() => window.matchMedia(COMPACT).matches);
  useEffect(() => {
    const mq = window.matchMedia(COMPACT);
    const on = () => setM(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return m;
}

/** 「放大 / 还原」记在本机：手机和桌面各记各的，手机默认放大（#621 原型） */
function useChatFull(mobile: boolean): [boolean, () => void] {
  const key = mobile ? 'board.chatFull.m' : 'board.chatFull.d';
  const read = () => {
    const v = localStorage.getItem(key);
    return v == null ? mobile : v === '1';
  };
  const [full, set] = useState(read);
  useEffect(() => set(read()), [key]);
  return [
    full,
    () =>
      set(f => {
        localStorage.setItem(key, f ? '0' : '1');
        return !f;
      }),
  ];
}

/** 放大时的那一层：手机上盖住整个屏幕（挂到 body：底部 sheet 带 transform，里面的 fixed 盖不住整屏），
 *  高度跟着 visualViewport（iOS 键盘弹起时不改 dvh）；桌面上盖住整个机器详情弹窗 */
function ChatFull({ mobile, label, restore, children }: { mobile: boolean; label: string; restore: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const anchor = useRef<HTMLSpanElement>(null);
  const [target, setTarget] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => {
    setTarget(mobile ? document.body : (anchor.current?.closest('.drawer') as HTMLElement | null) ?? document.body);
  }, [mobile]);
  useLayoutEffect(() => {
    const vv = window.visualViewport;
    const el = ref.current;
    if (!mobile || !vv || !el) return;
    const set = () => {
      el.style.setProperty('--vv-h', `${vv.height}px`);
      el.style.setProperty('--vv-top', `${vv.offsetTop}px`);
      el.classList.toggle('kb', vv.height < window.innerHeight - 120);
    };
    set();
    vv.addEventListener('resize', set);
    vv.addEventListener('scroll', set);
    return () => {
      vv.removeEventListener('resize', set);
      vv.removeEventListener('scroll', set);
    };
  }, [mobile, target]);
  // Esc 先退整屏（机器详情的 Esc 看到 .hc-full 时不处理）
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      if (useUI.getState().ctx || useUI.getState().confirm) return;
      e.preventDefault();
      restore();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });
  return (
    <>
      <span ref={anchor} hidden />
      {target &&
        createPortal(
          <div ref={ref} className={cx('hc-full', mobile ? 'on-screen' : 'in-drawer')} role="dialog" aria-modal="true" aria-label={label}>
            {children}
          </div>,
          target,
        )}
    </>
  );
}

export function HostChats({ h, chats, sel, setSel, now }: { h: HostInfo; chats: Chat[]; sel: ChatSel; setSel: (s: ChatSel) => void; now: number }) {
  const mobile = useCompact();
  const [full, toggle] = useChatFull(mobile);
  // 没选时：桌面上有对话就选最近的，没有就是新对话；手机上是列表
  const cur = sel === 'new' ? null : (chats.find(c => c.id === sel) ?? (sel == null && !mobile ? chats[0] : undefined));
  const view: 'list' | 'chat' | 'new' = cur ? 'chat' : sel === 'new' || !mobile ? 'new' : 'list';
  const back = mobile ? () => setSel(null) : undefined;
  const nav = { back, full, toggle };
  const pane =
    view === 'chat' && cur ? (
      <ChatPane key={cur.id} c={cur} mobile={mobile} nav={nav} />
    ) : view === 'new' ? (
      <>
        <div className="hc-head">
          <PaneNav nav={nav} side="back" />
          <span className="hc-title static">
            <b>新对话</b>
            <small>
              {h.name} · {h.online ? '在线' : '离线'}
            </small>
          </span>
          <PaneNav nav={nav} side="zoom" />
        </div>
        <NewChatForm key="new" h={h} onStarted={id => setSel(id)} />
      </>
    ) : null;
  return (
    <div className="hc" data-view={view}>
      <nav className="hc-list" aria-label="这台机器上的对话">
        <button type="button" className={cx('hc-new', view === 'new' && 'on')} disabled={!h.online} onClick={() => setSel('new')} title={h.online ? undefined : '机器离线'}>
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
                  {chatWho(c)} · {chatDirShort(c)} · <em className={cx(busy && 'on')}>{busy ? '回答中' : since(c.lastAt, now)}</em>
                </small>
              </span>
              <Icon name="chevRight" />
            </button>
          );
        })}
      </nav>
      {pane &&
        (full ? (
          <ChatFull mobile={mobile} label={cur ? `${h.name}上的对话` : `在${h.name}上新建对话`} restore={toggle}>
            {pane}
          </ChatFull>
        ) : (
          <div className="hc-main">{pane}</div>
        ))}
      {pane && full && !mobile && <div className="hc-main hc-main-ph" />}
    </div>
  );
}

type Nav = { back?: () => void; full: boolean; toggle: () => void };

/** 聊天头部两边：左边返回列表（只有手机上有），右上角唯一的图标按钮：放大 / 还原 */
function PaneNav({ nav, side }: { nav: Nav; side: 'back' | 'zoom' }) {
  if (side === 'back')
    return nav.back ? (
      <button type="button" className="icon-btn hc-back" aria-label="返回对话列表" onClick={nav.back}>
        <Icon name="chevLeft" />
      </button>
    ) : null;
  const label = nav.full ? '还原' : '放大';
  return (
    <Tip text={nav.full ? '还原  esc' : '放大'}>
      <button type="button" className="icon-btn hc-zoom" aria-label={label} aria-pressed={nav.full} data-tip-align="end" onClick={nav.toggle}>
        <Icon name={nav.full ? 'shrink' : 'fit'} />
      </button>
    </Tip>
  );
}

/** 对话在哪个目录里跑（#624）：列表写项目标识，头部写实际路径；没选项目的是主目录 */
const chatDirShort = (c: Chat) => c.project || '~';
const chatDir = (c: Chat) => c.cwd || '主目录';

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
function ChatPane({ c, mobile, nav }: { c: Chat; mobile: boolean; nav: Nav }) {
  const busy = c.turn === 'busy';
  const [more, setMore] = useState(false);
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
  // 上一次量到的可见高度：高度变了（键盘弹起、放大还原）引起的 scroll 不算人往上翻
  const seenH = useRef(0);
  const size = events.length + events.filter(e => e.ok != null).length + (view?.log.length ?? 0) + sent.length;
  useLayoutEffect(() => {
    const el = out.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [size, mode, !!view]);
  // 键盘弹起、放大还原时输出区变矮：在底部时仍贴着底部
  useEffect(() => {
    const el = out.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    seenH.current = el.clientHeight;
    const ro = new ResizeObserver(() => {
      seenH.current = el.clientHeight;
      if (stick.current) el.scrollTop = el.scrollHeight;
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const rows = groupEvents(events);
  const now = runningTool(events, busy);
  const modes = (
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
  );
  return (
    <>
      <div className={cx('hc-head', more && 'more')}>
        <PaneNav nav={nav} side="back" />
        <Mark id={c.agent} title={c.agentLabel} />
        <button type="button" className="hc-title" aria-expanded={more} title={more ? '收起细节' : '展开细节'} onClick={() => setMore(m => !m)}>
          <b>{c.title}</b>
          <small>
            <span className="hc-dot" style={{ ['--c' as string]: busy ? 'var(--c-doing)' : 'var(--text-3)' }} />
            <span>{busy ? '回答中' : '空闲'}</span>
            <span>{chatWho(c)}</span>
            <span className="hc-path">{chatDir(c)}</span>
            {(c.model || c.effort) && <span>{[c.model, c.effort && `强度 ${c.effort}`].filter(Boolean).join(' · ')}</span>}
            {c.envName && <span>{c.envName}</span>}
            <span>开始于 {hhmm(c.createdAt)}</span>
            <span>{c.host}</span>
          </small>
        </button>
        {!mobile && modes}
        <PaneNav nav={nav} side="zoom" />
      </div>
      {mobile && <div className="hc-sub">{modes}</div>}
      <div
        className="hc-out"
        ref={out}
        aria-busy={!view || undefined}
        onScroll={e => {
          const el = e.currentTarget;
          if (el.clientHeight !== seenH.current) return;
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
          <button className={cx('btn sm ghost hc-stop', stopping && 'is-busy')} type="button" aria-label="停止这一轮" disabled={stopping || sent.length > 0} onClick={() => void stop()}>
            <Icon name="stop" />
            <span className="hc-btn-label">停止这一轮</span>
          </button>
        )}
        <button className="btn sm brand hc-send" type="button" aria-label="发送" disabled={busy || !online || !draft.trim()} onClick={() => void send()}>
          <Icon name="send" />
          <span className="hc-btn-label">发送</span>
        </button>
      </div>
    </div>
  );
}
