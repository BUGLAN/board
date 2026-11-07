/* ==========================================================================
   卡片详情（PRD §10.8）
   桌面：居中弹窗 920×640，左内容 / 右动态；手机：底部 sheet，可下拉关闭
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { copyCardPrompt, triage } from '../lib/actions';
import { fileSize, stillUploading, useAttach } from '../lib/attach';
import { backTo, BEAT_MISSES, BEAT_MS, boardFlow, timing } from '../lib/schema';
import type { Activity, Attachment, Card, CardDetail as Detail } from '../lib/types';
import { accel, actorName, agentColor, ago, autosize, clock, cx, hhmm, isMobile, replay } from '../lib/util';
import { closeDetail, exec, openDetail, useCurrent, useData } from '../store/data';
import { confirm, openImage, openPicker, toast, toastError, useUI } from '../store/ui';
import { Avatar, LabelChip, Markdown, agent, useNow } from './bits';
import { RunPill } from './Card';
import { Icon, type IconName } from './Icon';

const ACT_ICON: Record<string, IconName> = {
  create: 'sparkle', edit: 'edit', move: 'move', reorder: 'reorder', claim: 'claim', renew: 'renew', timeout: 'timer', release: 'unlock', label: 'tag',
  beat: 'timer', lost: 'alert',
};

export function CardDetail() {
  const id = useData(s => s.detailId);
  const detail = useData(s => s.detail);
  const board = useData(s => s.board);
  const summary = id != null ? board?.cards[id] : undefined;
  // 详情还没取回来时先用白板上的摘要
  const live: Detail | null = detail && detail.id === id ? detail : summary ? { ...summary, activity: [] } : null;
  const last = useRef<Detail | null>(null);
  if (live) last.current = live;
  const card = live ?? last.current;
  const on = id != null && !!live;
  const drawer = useRef<HTMLElement>(null);
  const [sheetY, setSheetY] = useState<number | null>(null);

  // 卡片被删除（例如在另一个窗口）：关闭
  useEffect(() => {
    if (id != null && board && !board.cards[id]) closeDetail();
  }, [id, board]);

  useEffect(() => {
    if (!on) setSheetY(null);
  }, [on]);

  // 手机：下拉关闭
  const grab = useRef<{ y0: number; t0: number; dy: number } | null>(null);
  const onPointerDown = (e: React.PointerEvent) => {
    if (!isMobile()) return;
    const t = e.target as HTMLElement;
    if (!t.closest('.drawer-grab, .d-head') || t.closest('button')) return;
    grab.current = { y0: e.clientY, t0: performance.now(), dy: 0 };
    drawer.current?.setPointerCapture(e.pointerId);
    drawer.current?.classList.add('dragging');
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (!grab.current) return;
    grab.current.dy = Math.max(0, e.clientY - grab.current.y0);
    setSheetY(grab.current.dy);
  };
  const onPointerUp = () => {
    const g = grab.current;
    if (!g) return;
    grab.current = null;
    drawer.current?.classList.remove('dragging');
    const v = g.dy / Math.max(1, performance.now() - g.t0);
    if (g.dy > 140 || v > 0.6) closeDetail();
    else setSheetY(0);
  };

  return (
    <>
      <div className={cx('scrim', on && 'on')} onClick={closeDetail} />
      <aside
        ref={drawer}
        className={cx('drawer', on && 'on')}
        role="dialog"
        aria-modal="true"
        aria-label={card ? `#${card.id} ${card.title}` : '卡片详情'}
        aria-hidden={!on}
        style={sheetY != null ? { ['--sheet-y' as string]: `${sheetY}px` } : undefined}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        <div className="drawer-grab">
          <i />
        </div>
        {card && <Body key={card.id} card={card} on={on} />}
      </aside>
    </>
  );
}

function Body({ card: c, on }: { card: Detail; on: boolean }) {
  const cur = useCurrent();
  const board = useData(s => s.board);
  const now = useNow(1000);
  const bodyRef = useRef<HTMLDivElement>(null);
  const locked = !!c.holder;
  const f = boardFlow(board);
  const labels = cur?.project.labels ?? [];
  const list = board?.cols[c.column] ?? [];
  const i = list.indexOf(c.id);

  // 打开时把焦点放到关闭按钮，避免误触快捷键输入
  useEffect(() => {
    if (!on) return;
    const t = setTimeout(() => bodyRef.current?.querySelector<HTMLElement>('[data-act="close"]')?.focus({ preventScroll: true }), 60);
    return () => clearTimeout(t);
  }, [on]);

  const step = (d: number) => {
    const next = list[i + d];
    if (next == null) {
      bodyRef.current?.querySelector('.d-nav')?.animate([{ transform: 'none' }, { transform: `translateY(${d * 3}px)` }, { transform: 'none' }], { duration: 220 });
      return;
    }
    (document.activeElement as HTMLElement | null)?.blur?.(); // 先提交正在编辑的标题
    navDir = d;
    openDetail(next);
  };

  // 详情里的快捷键：↑↓ / K J 切换，C 复制为提示词，L 加标签
  useEffect(() => {
    if (!on) return;
    const key = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const t = e.target as HTMLElement;
      if (t.closest?.('input, textarea, [contenteditable]')) return;
      const ui = useUI.getState();
      if (ui.confirm || ui.palette.open || ui.creator.open || ui.picker || ui.form || ui.labelsOf || ui.switcher) return;
      const k = e.key.toLowerCase();
      if (k === 'c') {
        e.preventDefault();
        copyCardPrompt(c);
      } else if (k === 'l') {
        e.preventDefault();
        bodyRef.current?.querySelector<HTMLElement>('[data-label-picker]')?.click();
      } else if (e.key === 'ArrowUp' || k === 'k') {
        e.preventDefault();
        step(-1);
      } else if (e.key === 'ArrowDown' || k === 'j') {
        e.preventDefault();
        step(1);
      }
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  });

  useLayoutEffect(() => {
    if (navDir && bodyRef.current)
      bodyRef.current.animate([{ transform: `translateY(${navDir * 10}px)`, opacity: 0.3 }, { transform: 'none', opacity: 1 }], {
        duration: 280,
        easing: 'cubic-bezier(.16,1,.3,1)',
      });
    navDir = 0;
  }, []);

  // 列变化时列标签闪一下
  const pill = useRef<HTMLSpanElement>(null);
  const prevCol = useRef(c.column);
  useEffect(() => {
    if (prevCol.current !== c.column) replay(pill.current, 'changed');
    prevCol.current = c.column;
  }, [c.column]);

  const onLabels = (anchor: HTMLElement) =>
    cur &&
    openPicker({
      rect: anchor.getBoundingClientRect(),
      project: cur.project.key,
      selected: c.labels,
      onToggle: async (id, add) => {
        try {
          await exec({ op: 'label', id: c.id, ...(add ? { add: [id] } : { remove: [id] }) });
        } catch (e) {
          toastError(e);
        }
      },
    });

  const creator = c.createdBy;
  return (
    <div className="drawer-body scroll" ref={bodyRef}>
      <div className="d-head">
        <span className="d-id">#{c.id}</span>
        <span className="d-col" ref={pill} data-col={c.column} style={{ ['--c' as string]: f.color(c.column) }}>
          <i />
          <span>{f.name(c.column)}</span>
        </span>
        <span className="d-nav">
          <button className="icon-btn sm" type="button" data-tip="上一张  ↑ / K" disabled={i <= 0} onClick={() => step(-1)}>
            <Icon name="chevUp" />
          </button>
          <span className="d-pos mono">{list.length ? `${i + 1} / ${list.length}` : ''}</span>
          <button className="icon-btn sm" type="button" data-tip="下一张  ↓ / J" disabled={i < 0 || i >= list.length - 1} onClick={() => step(1)}>
            <Icon name="chevDown" />
          </button>
        </span>
        <span className="spacer" />
        <button className="btn sm ghost d-prompt" type="button" data-tip={`复制 /board-skills ${useData.getState().path}#${c.id}  C`} data-tip-align="end" onClick={() => copyCardPrompt(c)}>
          <Icon name="prompt" />
          复制为提示词
        </button>
        <button className="icon-btn" type="button" data-act="close" data-tip="关闭  esc" data-tip-align="end" onClick={closeDetail}>
          <Icon name="close" />
        </button>
      </div>
      <div className="d-grid">
        <div className="d-main scroll">
          <section className="d-section" style={{ ['--i' as string]: 0 }}>
            <TitleEditor card={c} locked={locked} />
            <div className="d-meta">
              <span>
                由 <b>{actorName(creator)}</b> 创建于 <b>{ago(c.createdAt, now)}</b>
              </span>
              <span>
                最近更新 <b>{ago(c.updatedAt, now)}</b>
              </span>
            </div>
            <div className="d-labels">
              {c.labels
                .map(id => labels.find(l => l.id === id))
                .filter(Boolean)
                .map(l => (
                  <LabelChip
                    key={l!.id}
                    label={l!}
                    onRemove={async () => {
                      try {
                        await exec({ op: 'label', id: c.id, remove: [l!.id] });
                      } catch (e) {
                        toastError(e);
                      }
                    }}
                  />
                ))}
              <button type="button" className="lb-add" data-label-picker onClick={e => onLabels(e.currentTarget)}>
                <Icon name="tag" />
                {c.labels.length ? '' : '添加标签'}
              </button>
            </div>
          </section>
          <ClaimPart card={c} now={now} />
          <section className="d-section" data-part="desc" style={{ ['--i' as string]: 2 }}>
            <DescEditor card={c} locked={locked} />
          </section>
          {!!c.attachments?.length && <Attachments list={c.attachments} />}
          <div className="d-danger">
            <span>删除后无法恢复</span>
            <button
              className="btn sm danger"
              type="button"
              onClick={async () => {
                const yes = await confirm({ title: `删除 #${c.id}？`, text: `「${c.title}」及其全部动态会被永久删除，无法恢复。`, ok: '删除' });
                if (!yes) return;
                closeDetail();
                try {
                  await exec({ op: 'delete', id: c.id });
                  toast({ type: 'success', title: `已删除 #${c.id}`, text: c.title });
                } catch (e) {
                  toastError(e);
                }
              }}
            >
              <Icon name="trash" />
              删除卡片
            </button>
          </div>
        </div>
        <ActivityPart card={c} now={now} />
      </div>
    </div>
  );
}
let navDir = 0;

function TitleEditor({ card: c, locked }: { card: Card; locked: boolean }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [v, setV] = useState(c.title);
  const editing = useRef(false);
  useEffect(() => {
    if (!editing.current) setV(c.title);
  }, [c.title]);
  useLayoutEffect(() => autosize(ref.current), [v]);
  useEffect(() => {
    const t = requestAnimationFrame(() => autosize(ref.current));
    return () => cancelAnimationFrame(t);
  }, []);
  const commit = async () => {
    editing.current = false;
    if (locked || v.trim() === c.title) return;
    try {
      await exec({ op: 'edit', id: c.id, title: v });
    } catch (e) {
      toastError(e);
      replay(ref.current, 'invalid');
      setV(c.title);
    }
  };
  return (
    <textarea
      ref={ref}
      className="d-title"
      rows={1}
      maxLength={200}
      spellCheck={false}
      readOnly={locked}
      value={v}
      aria-label="标题"
      onFocus={() => (editing.current = true)}
      onChange={e => setV(e.target.value)}
      onBlur={commit}
      onKeyDown={e => {
        if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
          e.preventDefault();
          ref.current?.blur();
        }
        if (e.key === 'Escape') {
          e.stopPropagation();
          setV(c.title);
          editing.current = false;
          requestAnimationFrame(() => ref.current?.blur());
        }
      }}
    />
  );
}

function DescTabs({ mode, setMode, modes }: { mode: string; setMode: (m: string) => void; modes: [string, string][] }) {
  const ref = useRef<HTMLDivElement>(null);
  const [thumb, setThumb] = useState({ w: 0, x: 0 });
  useLayoutEffect(() => {
    const on = ref.current?.querySelector<HTMLElement>('button.on');
    if (on) setThumb({ w: on.offsetWidth, x: on.offsetLeft - 2 });
  }, [mode]);
  return (
    <div className="act desc-tabs" ref={ref}>
      <span className="thumb" style={{ width: thumb.w, transform: `translateX(${thumb.x}px)` }} />
      {modes.map(([k, label]) => (
        <button key={k} type="button" className={cx(mode === k && 'on')} onClick={() => setMode(k)}>
          {label}
        </button>
      ))}
    </div>
  );
}
export { DescTabs };

/** 卡片的全部附件：图片缩略图点开放大，文件点击下载 */
function Attachments({ list }: { list: Attachment[] }) {
  return (
    <section className="d-section" data-part="files" style={{ ['--i' as string]: 3 }}>
      <div className="d-label">
        附件 <span className="count">{list.length}</span>
      </div>
      <ul className="att-list">
        {list.map(a => (
          <li key={a.id}>
            {a.image ? (
              <button className="att-thumb" type="button" aria-label={`查看 ${a.name}`} onClick={() => openImage(a.url, a.name)}>
                <img src={a.url} alt="" loading="lazy" />
              </button>
            ) : (
              <span className="att-thumb file">
                <Icon name="file" />
              </span>
            )}
            <a className="att-name" href={a.url} download={a.name} title={`下载 ${a.name}`}>
              {a.name}
            </a>
            <span className="att-meta">
              {fileSize(a.size)} · {actorName(a.createdBy)}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function DescEditor({ card: c, locked }: { card: Card; locked: boolean }) {
  const [mode, setMode] = useState<'view' | 'edit'>('view');
  const [v, setV] = useState(c.body);
  const ta = useRef<HTMLTextAreaElement>(null);
  const path = useData(s => s.path);
  const attach = useAttach({ board: path, card: c.id, setValue: setV, textarea: ta });
  useEffect(() => {
    if (locked && mode === 'edit') setMode('view');
  }, [locked, mode]);
  const edit = () => {
    if (locked) return;
    setV(c.body);
    setMode('edit');
    requestAnimationFrame(() => {
      const t = ta.current;
      if (!t) return;
      autosize(t);
      t.focus();
      t.setSelectionRange(t.value.length, t.value.length);
    });
  };
  const save = async () => {
    if (stillUploading(attach.busy)) return;
    try {
      await exec({ op: 'edit', id: c.id, body: v });
      setMode('view');
    } catch (e) {
      toastError(e);
    }
  };
  return (
    <>
      <div className="d-label">
        描述
        {!locked && (
          <DescTabs
            mode={mode}
            setMode={m => (m === 'edit' ? edit() : setMode('view'))}
            modes={[
              ['view', '预览'],
              ['edit', '编辑'],
            ]}
          />
        )}
      </div>
      <div className="desc-holder">
        {mode === 'edit' ? (
          <div className={cx('desc-edit', attach.over && 'drop-on')}>
            <textarea
              ref={ta}
              {...attach.handlers}
              className="textarea scroll"
              spellCheck={false}
              placeholder="支持 Markdown：验收标准、相关链接…"
              value={v}
              onChange={e => {
                setV(e.target.value);
                autosize(e.target);
              }}
              onKeyDown={e => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  save();
                }
                if (e.key === 'Escape') {
                  e.stopPropagation();
                  setMode('view');
                }
              }}
            />
            <div className="row">
              <span className="hint">Markdown · 可粘贴截图 · {accel('↵')} 保存 · esc 取消</span>
              <button className="btn sm ghost" type="button" title="附件（≤ 10 MB）" onClick={attach.pick}>
                <Icon name="clip" />
                {attach.busy ? '上传中…' : '附件'}
              </button>
              {attach.fileInput}
              <button className="btn sm ghost" type="button" onClick={() => setMode('view')}>
                取消
              </button>
              <button className="btn sm primary" type="button" onClick={save}>
                保存
              </button>
            </div>
          </div>
        ) : c.body ? (
          <div className={cx('desc-view', locked && 'locked')} onDoubleClick={edit}>
            <Markdown src={c.body} />
          </div>
        ) : (
          <div className={cx('desc-view empty', locked && 'locked')} onClick={edit}>
            <Icon name="edit" />
            {locked ? '没有描述' : '添加描述…（支持 Markdown）'}
          </div>
        )}
      </div>
    </>
  );
}

function ClaimPart({ card: c, now }: { card: Card; now: number }) {
  const start = boardFlow(useData(s => s.board)).home?.name ?? '待办';
  if (c.column === 'inbox') {
    return (
      <section className="d-section" data-part="claim" style={{ ['--i' as string]: 1 }}>
        <div className="inbox-note">
          <div className="in-ic">
            <Icon name="inbox" />
          </div>
          <div>
            <b>未整理的条目</b>
            <span>还没放上白板，agent 不会领取。补充好标题和描述后整理到「{start}」。</span>
          </div>
          <button
            className="btn sm primary"
            type="button"
            onClick={async () => {
              await triage(c.id);
              toast({ type: 'success', title: `#${c.id} 已整理到「${start}」`, text: `放在「${start}」最下面，可以拖动调整优先级。`, duration: 2400 });
            }}
          >
            整理到{start}
            <Icon name="move" />
          </button>
        </div>
      </section>
    );
  }
  const t = timing(c, now);
  if (!c.holder) {
    // 做过、现在没人在做：只显示耗时
    if (!t.runs) return null;
    return (
      <section className="d-section" data-part="claim" style={{ ['--i' as string]: 1 }}>
        <div className="claim-card idle">
          <span className="claim-ic">
            <Icon name="timer" />
          </span>
          <div className="claim-who">
            <b>耗时</b>
            <span>{t.runs} 次领取的耗时之和</span>
          </div>
          <div className="claim-big">
            <span>累计</span>
            {clock(t.total)}
          </div>
          <RunStats t={t} />
        </div>
      </section>
    );
  }
  return (
    <section className="d-section" data-part="claim" style={{ ['--i' as string]: 1 }}>
      <div className="lock-note" style={{ marginBottom: 10 }}>
        <Icon name="lock" />
        <div>
          「进行中」的卡片只有持有者 <b className="mono">{c.holder}</b> 能编辑和移动。你仍可以评论、加或去标签，或强制释放让它回到待办。
        </div>
      </div>
      <div className={cx('claim-card', t.beat === 'late' && 'late')} style={{ ['--ac' as string]: agentColor(c.holder) }}>
        <Avatar actor={agent(c.holder)} className="lg" />
        <div className="claim-who">
          <b>{c.holder}</b>
          <span>
            领取于 {hhmm(c.claimedAt || c.updatedAt)}
            {c.runStartedAt != null && <> · 第一次心跳 {hhmm(c.runStartedAt)}</>}
          </span>
        </div>
        <div className="claim-big">
          <span>{t.runs > 0 ? `第 ${t.runs} 次` : '本次'}</span>
          <RunPill card={c} now={now} big />
        </div>
        <RunStats t={t} />
        <div className="release">
          <span>
            持有者每 {BEAT_MS / 1000} 秒发一次心跳；连续 {BEAT_MISSES} 次没收到判定异常退出，卡片回到领取它时所在的状态
          </span>
          <button
            className="btn sm danger"
            type="button"
            onClick={async () => {
              try {
                await exec({ op: 'release', id: c.id });
                toast({ type: 'success', title: `已强制释放 #${c.id}`, text: `${c.holder} 的领取已清空，卡片回到「${backTo(boardFlow(useData.getState().board), c.claimedFrom)}」最上面` });
              } catch (e) {
                toastError(e);
              }
            }}
          >
            <Icon name="unlock" />
            强制释放
          </button>
        </div>
      </div>
    </section>
  );
}

/** 累计 / 领取次数 / 异常退出 */
function RunStats({ t }: { t: ReturnType<typeof timing> }) {
  return (
    <div className="run-stats">
      <div>
        <span>累计</span>
        <b>{clock(t.total)}</b>
      </div>
      <div>
        <span>领取次数</span>
        <b>{t.runs}</b>
      </div>
      <div>
        <span>异常退出</span>
        <b className={cx(t.lost > 0 && 'bad')}>{t.lost}</b>
      </div>
    </div>
  );
}

function ActivityPart({ card: c, now }: { card: Detail; now: number }) {
  const scroller = useRef<HTMLDivElement>(null);
  const shown = useRef<number | null>(null);
  const [text, setText] = useState('');
  const ta = useRef<HTMLTextAreaElement>(null);
  const send = useRef<HTMLButtonElement>(null);
  const n = c.activity.length;
  const path = useData(s => s.path);
  const attach = useAttach({ board: path, card: c.id, setValue: setText, textarea: ta });
  const firstNew = shown.current;

  // 默认显示到最新一条；新动态到来时如果停在底部就跟随
  useLayoutEffect(() => {
    const sc = scroller.current;
    if (!sc || !n) return;
    if (shown.current == null) sc.scrollTop = sc.scrollHeight;
    else if (n > shown.current && sc.scrollHeight - sc.scrollTop - sc.clientHeight < 160) sc.scrollTo({ top: sc.scrollHeight, behavior: 'smooth' });
    shown.current = n;
  }, [n]);

  const submit = async () => {
    if (stillUploading(attach.busy)) return;
    const t = text.trim();
    if (!t) {
      ta.current?.focus();
      return;
    }
    try {
      await exec({ op: 'comment', id: c.id, text: t });
      setText('');
      requestAnimationFrame(() => autosize(ta.current));
      send.current?.animate(
        [{ transform: 'rotate(-12deg)' }, { transform: 'translate(18px,-18px) rotate(-12deg)', opacity: 0 }, { transform: 'translate(-10px, 10px)', opacity: 0 }, { transform: 'none', opacity: 1 }],
        { duration: 520, easing: 'cubic-bezier(.16,1,.3,1)' },
      );
      requestAnimationFrame(() => scroller.current?.scrollTo({ top: scroller.current.scrollHeight, behavior: 'smooth' }));
    } catch (e) {
      toastError(e);
    }
  };

  return (
    <div className="d-side" data-part="activity">
      <div className="d-label d-side-head">
        动态 <span className="count">{n}</span>
        <span className="act">只能追加</span>
      </div>
      <div className="d-activity scroll" ref={scroller}>
        <div className="timeline">
          {c.activity.map((a, idx) => (
            <ActivityItem key={a.id} a={a} now={now} fresh={firstNew != null && idx >= firstNew} />
          ))}
        </div>
      </div>
      <div className="d-comment">
        <div className={cx('comment-box', text.trim() && 'has-text', attach.over && 'drop-on')}>
          <button className="comment-attach" type="button" aria-label="添加附件" title="附件：也可以直接粘贴截图（≤ 10 MB）" onClick={attach.pick}>
            <Icon name="clip" />
          </button>
          {attach.fileInput}
          <textarea
            ref={ta}
            {...attach.handlers}
            rows={1}
            // 评论里常有代码：关掉拼写检查，macOS 也就不会把 ' " 自动换成弯引号
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            placeholder={`写评论…  ${accel('↵')} 发送`}
            aria-label="评论"
            value={text}
            onChange={e => {
              setText(e.target.value);
              autosize(e.target);
            }}
            onKeyDown={e => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                submit();
              }
              if (e.key === 'Escape') {
                e.stopPropagation();
                ta.current?.blur();
              }
            }}
          />
          <button className="comment-send" type="button" aria-label="发送评论" ref={send} onClick={submit}>
            <Icon name="send" />
          </button>
        </div>
      </div>
    </div>
  );
}

function ActivityItem({ a, now, fresh }: { a: Activity; now: number; fresh: boolean }) {
  const name =
    a.actor.kind === 'agent' ? (
      <b className="agent-name" style={{ color: agentColor(a.actor.name) }}>
        {a.actor.name}
      </b>
    ) : (
      <b>{actorName(a.actor)}</b>
    );
  const time = (
    <span className="tl-time" title={new Date(a.at).toLocaleString()}>
      {ago(a.at, now)}
    </span>
  );
  if (a.kind === 'comment') {
    return (
      <div className={cx('tl-item', a.actor.kind === 'human' && 'mine', fresh && 'new')}>
        <div>
          <Avatar actor={a.actor} />
        </div>
        <div className="tl-bubble">
          <div className="who">
            {name}
            {a.actor.kind === 'agent' && (
              <span className="chip" style={{ height: 18, fontSize: 10.5 }}>
                CLI
              </span>
            )}
            {time}
          </div>
          <Markdown src={a.text} />
        </div>
      </div>
    );
  }
  // 「…，第 N 次耗时 X」从文字里拿出来，做成末尾的耗时标签
  const run = a.text.match(/^(.*?)[，,]\s*第 (\d+) 次耗时(约)? ?([^，]+)(.*)$/);
  return (
    <div className={cx('tl-item', fresh && 'new')}>
      <div className={`tl-icon k-${a.action}`}>
        <Icon name={ACT_ICON[a.action] || 'info'} />
      </div>
      <div className="tl-sys">
        {name} {run ? run[1] + run[5] : a.text} {time}
        {run && (
          <span className={cx('run-chip', a.action === 'lost' && 'bad')}>
            第 {run[2]} 次 · {run[3] ?? ''}
            {run[4]}
          </span>
        )}
      </div>
    </div>
  );
}
