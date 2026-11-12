/* ==========================================================================
   复盘台（PRD §10.14，卡片 #194）：交审的卡逐张过，通过、打回一步完成；需要人处理的卡回复后放回
   - 入口：白板上交审状态块标题的「复盘」、需要人处理的状态块标题的「处理」；大盘「现在」的待复盘
   - 居中大弹窗（手机上全屏），叠在白板上。卡片按进入当前状态的时间从早到晚，一次一张，顶部「还剩 N 张」
   - 一张卡：标题、标签；agent 最近一次评论放最上面（提交、集成分支、检查几行高亮，截图直接显示）；描述（验收标准能直接勾）；
     派发记录（每条打开会话）
   - 操作：通过（移到连向的终点状态）、打回（必须写意见：评论和移动一个命令，移到连向的可领取状态）、跳过；
     连向的终点 / 可领取状态不止一个时按钮旁边选去哪个。处理模式只有「回复并放回」（回复、移到首个可领取状态）
   - 快捷键：A 通过、R 打回（光标进意见框）、J / K 下一张 / 上一张、Esc 关闭（在意见框里先离开意见框）
   - 乐观更新（AGENTS.md「界面写操作」）：movePatch 当帧把卡片移走，下一张马上顶上来；失败时提示原因、回到那一张，意见还在
   ========================================================================== */
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { openCard } from '../lib/actions';
import { api } from '../lib/api';
import { repoWeb } from '../lib/repo';
import { approveTargets, deskQueue, homeOf, latestAgentComment, rejectTargets, splitFacts, taskCount } from '../lib/review';
import type { CardDetail, Status } from '../lib/types';
import { cx, isMobile } from '../lib/util';
import { exec, movePatch, useCardBusy, useCurrent, useData } from '../store/data';
import { openReview, toastError, useUI } from '../store/ui';
import { Avatar, LabelChips, Markdown, Select, useNow } from './bits';
import { Icon } from './Icon';
import { DispatchHistory } from './hosts/Dispatch';
import { usePresence } from './overlays/presence';
import { TaskBody } from './TaskBody';

type Req = { cols: string[]; mode: 'review' | 'human' };

export function ReviewDesk() {
  const req = useUI(s => s.review);
  const { mounted, on } = usePresence(!!req, 260);
  const last = useRef(req);
  if (req) last.current = req;
  if (!mounted || !last.current) return null;
  return createPortal(<DeskShell req={last.current} on={on && !!req} />, document.body);
}

function DeskShell({ req, on }: { req: Req; on: boolean }) {
  const close = () => openReview(null);
  return (
    <>
      <div className={cx('scrim rv-scrim', on && 'on')} onClick={close} />
      <aside className={cx('drawer rv-modal', on && 'on')} role="dialog" aria-modal="true" aria-label={req.mode === 'review' ? '复盘台' : '处理需要人处理的卡片'} aria-hidden={!on}>
        <DeskBody req={req} on={on} close={close} />
      </aside>
    </>
  );
}

/** 输入框、选择框里按的键不当快捷键 */
const typing = (t: EventTarget | null) => {
  const el = t as HTMLElement | null;
  return !!el && (el.tagName === 'TEXTAREA' || el.tagName === 'INPUT' || el.tagName === 'SELECT' || el.isContentEditable);
};

function DeskBody({ req, on, close }: { req: Req; on: boolean; close: () => void }) {
  const board = useData(s => s.board);
  const path = useData(s => s.path);
  const rev = useData(s => s.ws?.rev);
  const statuses = board?.statuses ?? [];
  const labels = useCurrent()?.project.labels ?? [];
  const queue = useMemo(() => deskQueue(board, req.cols), [board, req.cols]);
  // 正在看的卡片：还在队列里就是它；被移走了（通过、打回、别处改了）就看同一个位置上的下一张
  const [cur, setCur] = useState<number | null>(null);
  const pos = useRef(0);
  const id = cur != null && queue.includes(cur) ? cur : (queue[Math.min(pos.current, queue.length - 1)] ?? null);
  useEffect(() => {
    if (id != null) pos.current = queue.indexOf(id);
  }, [id, queue]);
  const c = id != null ? board?.cards[id] ?? null : null;
  const busy = useCardBusy(id ?? 0);
  const index = id != null ? queue.indexOf(id) : -1;
  const go = (d: number) => {
    if (!queue.length) return;
    const i = Math.max(0, Math.min(queue.length - 1, index + d));
    setCur(queue[i]);
  };

  // 详情（动态、附件）：换卡片时、有推送变化时取
  const [detail, setDetail] = useState<CardDetail | null>(null);
  useEffect(() => {
    if (!on || id == null) return;
    let alive = true;
    api
      .card(path, id)
      .then(d => alive && setDetail(d))
      .catch(() => alive && setDetail(null));
    return () => {
      alive = false;
    };
  }, [on, path, id, rev]);
  const d = detail && detail.id === id ? detail : null;

  // 每张卡各自的意见草稿：失败回到那一张时还在
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const draft = id != null ? (drafts[id] ?? '') : '';
  const setDraft = (v: string) => id != null && setDrafts(m => ({ ...m, [id]: v }));
  const [need, setNeed] = useState(0);
  const box = useRef<HTMLTextAreaElement>(null);

  const col = c?.column ?? req.cols[0];
  const approve = approveTargets(statuses, col);
  const reject = rejectTargets(statuses, col);
  const home = homeOf(statuses);
  const [approveTo, setApproveTo] = useState<string | null>(null);
  const [rejectTo, setRejectTo] = useState<string | null>(null);
  const toApprove = approve.find(s => s.key === approveTo) ?? approve[0] ?? null;
  const toReject = (req.mode === 'human' ? (home ? [home] : []) : reject).find(s => s.key === rejectTo) ?? (req.mode === 'human' ? home : reject[0]) ?? null;

  /** 移到 to；有意见时评论和移动一个命令（服务端一个事务）。补丁当帧生效，下一张马上顶上来；失败时回到这一张，意见放回去 */
  const send = (to: Status, text: string) => {
    if (id == null) return;
    const card = id;
    const t = text.trim();
    const p = t ? exec({ op: 'reply', id: card, text: t, column: to.key }, movePatch(card, to.key)) : exec({ op: 'move', id: card, column: to.key }, movePatch(card, to.key));
    setDrafts(m => {
      const n = { ...m };
      delete n[card];
      return n;
    });
    p.catch(e => {
      toastError(e);
      setDrafts(m => ({ ...m, [card]: text }));
      setCur(card);
    });
  };
  const doApprove = () => {
    if (!toApprove || id == null || busy) return;
    send(toApprove, draft);
  };
  const doReject = () => {
    if (!toReject || id == null || busy) return;
    if (!draft.trim()) {
      setNeed(n => n + 1);
      box.current?.focus();
      return;
    }
    send(toReject, draft);
  };

  // 快捷键：A 通过、R 打回（进意见框）、J / K、Esc。Esc 用捕获阶段，先于卡片详情
  const keys = useRef({ doApprove, doReject, go, close, mode: req.mode });
  keys.current = { doApprove, doReject, go, close, mode: req.mode };
  useEffect(() => {
    if (!on) return;
    const down = (e: KeyboardEvent) => {
      const ui = useUI.getState();
      if (ui.confirm || ui.ctx || ui.session) return;
      const k = keys.current;
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        if (typing(e.target)) (e.target as HTMLElement).blur();
        else k.close();
        return;
      }
      if (typing(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      const key = e.key.toLowerCase();
      if (key === 'a' && k.mode === 'review') {
        e.preventDefault();
        k.doApprove();
      } else if (key === 'r') {
        e.preventDefault();
        box.current?.focus();
      } else if (key === 'j' || e.key === 'ArrowDown') {
        e.preventDefault();
        k.go(1);
      } else if (key === 'k' || e.key === 'ArrowUp') {
        e.preventDefault();
        k.go(-1);
      }
    };
    window.addEventListener('keydown', down, true);
    return () => window.removeEventListener('keydown', down, true);
  }, [on]);

  const names = req.cols.map(k => statuses.find(s => s.key === k)?.name ?? k).join('、');
  const review = req.mode === 'review';
  return (
    <div className="drawer-body rv-body">
      <div className="d-head rv-head">
        <span className="rv-title">
          <Icon name={review ? 'checkCircle' : 'alert'} />
          {review ? '复盘台' : '处理'}
          <em>{names}</em>
        </span>
        <span className="rv-left">{queue.length ? `还剩 ${queue.length} 张` : ''}</span>
        <span className="spacer" />
        {queue.length > 0 && (
          <span className="d-nav">
            <button className="icon-btn" type="button" aria-label="上一张" data-tip="上一张  K" disabled={index <= 0} onClick={() => go(-1)}>
              <Icon name="chevUp" />
            </button>
            <span className="rv-pos">
              {index + 1} / {queue.length}
            </span>
            <button className="icon-btn" type="button" aria-label="下一张" data-tip="下一张 / 跳过  J" disabled={index >= queue.length - 1} onClick={() => go(1)}>
              <Icon name="chevDown" />
            </button>
          </span>
        )}
        <button aria-label="关闭" className="icon-btn" type="button" data-tip="关闭  esc" data-tip-align="end" onClick={close}>
          <Icon name="close" />
        </button>
      </div>
      {!c ? (
        <div className="rv-empty">
          <Icon name="sparkle" />
          <b>{review ? '都复盘完了' : '没有要处理的卡片'}</b>
          <span>「{names}」里现在没有卡片。</span>
          <button className="btn sm" type="button" onClick={close}>
            关闭
          </button>
        </div>
      ) : (
        <>
          <div className="rv-main" key={c.id}>
            <div className="rv-card-head">
              <button type="button" className="rv-id" data-tip="在卡片详情里打开" onClick={() => {
                close();
                openCard(c.id);
              }}>
                #{c.id}
              </button>
              <h2 className="rv-card-title">{c.title}</h2>
              {busy && <span className="rv-saving">保存中</span>}
            </div>
            {c.labels.length > 0 && (
              <div className="rv-labels">
                <LabelChips ids={c.labels} labels={labels} />
              </div>
            )}
            <AgentComment detail={d} repo={board?.repo} mode={req.mode} />
            <section className="rv-sec">
              <div className="d-label">
                描述
                {(() => {
                  const t = taskCount(c.body);
                  return t.total ? <span className="count">验收标准 {t.done}/{t.total}</span> : null;
                })()}
              </div>
              {c.body ? <TaskBody card={c} repo={board?.repo} locked={!!c.holder || busy} className="rv-desc" /> : <p className="hp-note">没有描述</p>}
            </section>
            <DispatchHistory board={path} card={c.id} />
          </div>
          <div className="rv-foot">
            <textarea
              ref={box}
              key={`box-${c.id}`}
              className={cx('textarea rv-box', need > 0 && 'need')}
              style={need ? { ['--n' as string]: need } : undefined}
              rows={isMobile() ? 2 : 3}
              placeholder={review ? '意见：打回时必须写，作为评论发出（R）。通过时写了也会一起发出' : '回复 agent 的问题，作为评论发出，再把卡片放回去'}
              value={draft}
              onChange={e => {
                setDraft(e.target.value);
                setNeed(0);
              }}
              onKeyDown={e => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  doReject();
                }
              }}
            />
            {need > 0 && <span className="rv-need">{review ? '打回要写意见' : '要写回复'}</span>}
            <div className="rv-acts">
              <button className="btn sm ghost" type="button" disabled={index >= queue.length - 1} onClick={() => go(1)} data-tip="看下一张  J">
                跳过
              </button>
              <span className="spacer" />
              {review ? (
                <>
                  <Target list={reject} value={toReject?.key ?? null} onChange={setRejectTo} label="打回到" />
                  <button className="btn sm rv-reject" type="button" disabled={busy || !toReject} onClick={doReject} data-tip={`意见作为评论发出，卡片移到「${toReject?.name ?? ''}」  R 写意见 · ⌘↵ 打回`}>
                    <Icon name="replay" />
                    打回
                  </button>
                  <Target list={approve} value={toApprove?.key ?? null} onChange={setApproveTo} label="通过到" />
                  <button className="btn sm brand rv-approve" type="button" disabled={busy || !toApprove} onClick={doApprove} data-tip={`移到「${toApprove?.name ?? ''}」  A`}>
                    <Icon name="check" />
                    通过
                  </button>
                </>
              ) : (
                <button className="btn sm brand" type="button" disabled={busy || !toReject} onClick={doReject} data-tip={`回复作为评论发出，卡片移到「${toReject?.name ?? ''}」  ⌘↵`}>
                  <Icon name="send" />
                  回复并放回{toReject ? `「${toReject.name}」` : ''}
                </button>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

/** 去向不止一个时选去哪个；只有一个时不显示 */
function Target({ list, value, onChange, label }: { list: Status[]; value: string | null; onChange: (k: string) => void; label: string }) {
  if (list.length < 2) return null;
  return <Select className="rv-target" value={value ?? list[0].key} options={list.map(s => ({ value: s.key, label: s.name }))} onChange={onChange} aria-label={label} />;
}

/** agent 最近一次评论：交审评论（复盘）或者它留下的问题（处理）。提交、集成分支、检查几行高亮，截图直接显示 */
function AgentComment({ detail, repo, mode }: { detail: CardDetail | null; repo?: string | null; mode: 'review' | 'human' }) {
  const now = useNow(30000);
  if (!detail) return <div className="rv-agent loading" aria-hidden="true" />;
  const a = latestAgentComment(detail.activity);
  if (!a) return <p className="hp-note rv-none">{mode === 'review' ? 'agent 没有留交审评论' : 'agent 没有留评论'}</p>;
  const { body, facts } = splitFacts(a.text);
  return (
    <section className="rv-agent">
      <div className="rv-agent-head">
        <Avatar actor={a.actor} />
        <b>{a.actor.name}</b>
        <span>{mode === 'review' ? '交审评论' : '最近的评论'}</span>
        <time>{ago(a.at, now)}</time>
      </div>
      {facts.length > 0 && (
        <div className="rv-facts">
          {facts.map(f => (
            <div key={f.key} className={cx('rv-fact', f.key === '检查' && 'check')}>
              <span>{f.key}</span>
              {f.key === '提交' ? <CommitLink value={f.value} repo={repo} /> : <Markdown src={f.value} repo={repo} className="rv-fact-v" />}
            </div>
          ))}
        </div>
      )}
      {body && <Markdown src={body} repo={repo} className="rv-agent-body" />}
    </section>
  );
}

/** 提交号：白板设了仓库地址时链到提交页面（PRD §4），否则只显示 */
function CommitLink({ value, repo }: { value: string; repo?: string | null }) {
  const web = repoWeb(repo);
  const m = value.match(/^([0-9a-f]{7,40})\b(.*)$/);
  if (!m) return <span className="rv-fact-v mono">{value}</span>;
  return (
    <span className="rv-fact-v mono">
      {web ? (
        <a href={web.commit(m[1])} target="_blank" rel="noreferrer">
          {m[1]}
        </a>
      ) : (
        m[1]
      )}
      {m[2]}
    </span>
  );
}

function ago(at: number, now: number): string {
  const m = Math.max(0, Math.round((now - at) / 60000));
  if (m < 1) return '刚刚';
  if (m < 60) return `${m} 分钟前`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} 小时前`;
  return `${Math.round(h / 24)} 天前`;
}
