/* ==========================================================================
   卡片详情（PRD §10.8）
   桌面：居中弹窗 920×640，左内容 / 右动态；手机：底部 sheet，可下拉关闭
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { copyCardPrompt, triage } from '../lib/actions';
import { fileSize, stillUploading, useAttach } from '../lib/attach';
import { cardDispatch } from '../lib/hosts';
import { cardPrompt } from '../lib/prompt';
import { deleteCards } from '../lib/trash';
import { backTo, BEAT_MISSES, BEAT_MS, boardFlow, timing } from '../lib/schema';
import type { Activity, Attachment, Card, CardDetail as Detail } from '../lib/types';
import { accel, actorName, agentColor, agentInk, ago, autosize, clock, copyText, cx, dateTime, hhmm, isMobile, replay, since } from '../lib/util';
import { addLabel, closeDetail, editCard, exec, isSettled, movePatch, omittedIn, openDetail, removeComment, removeLabel, sendComment, serverNow, useCardBusy, useCurrent, useData, useOnline, useSending } from '../store/data';
import { closeCtx, confirm, openCtx, openDispatch, openImage, openPicker, overlayOpen, toast, toastError, useUI } from '../store/ui';
import { Avatar, DescTabs, LabelChip, Markdown, agent, useNow } from './bits';
import { RunPill } from './Card';
import { Skeleton, SkeletonText } from './Skeleton';
import { DispatchHistory, DispatchNote } from './hosts/Dispatch';
import { Icon, type IconName } from './Icon';

const NO_IDS: number[] = [];

const ACT_ICON: Record<string, IconName> = {
  create: 'sparkle', edit: 'edit', move: 'move', reorder: 'reorder', claim: 'claim', renew: 'renew', timeout: 'timer', release: 'unlock', label: 'tag',
  beat: 'timer', lost: 'alert', dispatch: 'dispatch', merge: 'merge',
};

export function CardDetail() {
  const id = useData(s => s.detailId);
  const detail = useData(s => s.detail);
  const detailError = useData(s => s.detailError);
  const board = useData(s => s.board);
  const summary = id != null ? board?.cards[id] : undefined;
  // 详情（动态、附件）还在路上：标题、标签、ID、描述先用摘要显示，动态和附件放骨架
  const loading = id != null && !(detail && detail.id === id) && !detailError;
  // 详情还没取回来时先用白板上的摘要
  const live: Detail | null = detail && detail.id === id ? detail : summary ? { ...summary, activity: [] } : null;
  const last = useRef<Detail | null>(null);
  if (live) last.current = live;
  const card = live ?? last.current;
  const on = id != null && !!live;
  const drawer = useRef<HTMLElement>(null);
  const [sheetY, setSheetY] = useState<number | null>(null);

  // 卡片被删除（例如在另一个窗口）：关闭。终点状态更早的卡片本来就不在快照里（omitted），以取回的详情为准；
  // 它被删掉时由刷新取详情得到 NOT_FOUND 关闭（store/data.ts load）
  const outside = !!detail && detail.id === id && omittedIn(board, detail.column) > 0;
  useEffect(() => {
    if (id != null && board && !board.cards[id] && !loading && !outside) closeDetail();
  }, [id, board, loading, outside]);

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
        {card && <Body key={card.id} card={card} on={on} loading={loading} error={detailError} />}
      </aside>
    </>
  );
}

function Body({ card: c, on, loading, error }: { card: Detail; on: boolean; loading: boolean; error: string | null }) {
  const board = useData(s => s.board);
  const now = useNow(1000);
  const bodyRef = useRef<HTMLDivElement>(null);
  const locked = !!c.holder;
  const f = boardFlow(board);
  // 这张卡片有还没确认的修改时，头部显示「保存中」，对它的修改都禁用，保存完再改
  const saving = useCardBusy(c.id);
  const dispatched = useData(s => cardDispatch(s.ws, s.path, c.id));
  const list = board?.cols[c.column] ?? [];
  const i = list.indexOf(c.id);
  const skipped = omittedIn(board, c.column);

  // 打开时把焦点放到关闭按钮，避免误触快捷键输入
  useEffect(() => {
    if (!on) return;
    const t = setTimeout(() => bodyRef.current?.querySelector<HTMLElement>('[data-act="close"]')?.focus({ preventScroll: true }), 60);
    return () => clearTimeout(t);
  }, [on]);

  const step = (d: number) => {
    const next = i < 0 ? undefined : list[i + d];
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
      // 菜单（「移动到」、右键菜单）、弹窗打开时只作用于它们：名单和全局快捷键共用 overlayOpen
      if (overlayOpen()) return;
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

  const creator = c.createdBy;
  return (
    <div className="drawer-body" ref={bodyRef}>
      <div className="d-head">
        <span className="d-id">#{c.id}</span>
        <span className="d-col" ref={pill} data-col={c.column} style={{ ['--c' as string]: f.color(c.column) }}>
          <i />
          <span>{f.name(c.column)}</span>
        </span>
        <span className="d-nav">
          <button aria-label="上一张" className="icon-btn sm" type="button" data-tip="上一张  ↑ / K" disabled={i <= 0} onClick={() => step(-1)}>
            <Icon name="chevUp" />
          </button>
          {/* 快照省略了更早的卡片（可能夹在中间）时整个状态里的位置算不出来，只显示总数 */}
          <span className="d-pos mono">{i < 0 ? '' : skipped ? `共 ${list.length + skipped}` : `${i + 1} / ${list.length}`}</span>
          <button aria-label="下一张" className="icon-btn sm" type="button" data-tip="下一张  ↓ / J" disabled={i < 0 || i >= list.length - 1} onClick={() => step(1)}>
            <Icon name="chevDown" />
          </button>
        </span>
        <span className={cx('d-saving', saving && 'on')} role="status" aria-live="polite">
          <i aria-hidden="true" />
          {saving ? '保存中' : ''}
        </span>
        <span className="spacer" />
        <button className="btn sm ghost d-prompt" type="button" data-tip={`复制 ${cardPrompt(useData.getState().path, c.id)}  C`} data-tip-align="end" onClick={() => copyCardPrompt(c)}>
          <Icon name="prompt" />
          复制为提示词
        </button>
        <button aria-label="关闭" className="icon-btn" type="button" data-act="close" data-tip="关闭  esc" data-tip-align="end" onClick={closeDetail}>
          <Icon name="close" />
        </button>
      </div>
      <div className="d-grid">
        <div className="d-main">
          <section className="d-section" style={{ ['--i' as string]: 0 }}>
            <TitleEditor card={c} locked={locked} busy={saving} />
            <div className="d-meta">
              <span>
                由 <b>{actorName(creator)}</b> 创建于 <b>{ago(c.createdAt, now)}</b>
              </span>
              <span>
                最近更新 <b>{ago(c.updatedAt, now)}</b>
              </span>
            </div>
            <CardLabels card={c} saving={saving} />
          </section>
          <DispatchNote board={useData.getState().path} card={c.id} />
          <DispatchHistory board={useData.getState().path} card={c.id} />
          <ClaimPart card={c} now={now} busy={saving} />
          <section className="d-section" data-part="desc" style={{ ['--i' as string]: 2 }}>
            <DescEditor card={c} locked={locked} busy={saving} />
          </section>
          {!!c.attachments?.length && <Attachments list={c.attachments} />}
          {/* 描述里引用了附件：附件列表多半不空，先占位 */}
          {loading && c.body.includes('/api/files/') && <AttachmentsSkeleton />}
          {/* 底部操作栏：左边删除（不可恢复写在二次确认框里），右边「移动到」（拖拽的快捷方式） */}
          <div className="d-actions">
            <button
              className="btn sm danger"
              type="button"
              disabled={saving}
              onClick={async () => {
                const yes = await confirm({
                  title: `删除 #${c.id}？`,
                  text: `「${c.title}」会移进回收站（侧栏「回收站」），30 天内可以恢复；永久删除前评论和附件都还在。`,
                  ok: '删除',
                });
                if (!yes) return;
                closeDetail();
                // 当帧从白板上拿掉；提示里可以「撤销」（= 从回收站恢复）
                await deleteCards([c.id], { titles: { [c.id]: c.title } });
              }}
            >
              <Icon name="trash" />
              删除卡片
            </button>
            <button
              className="btn sm d-move"
              type="button"
              disabled={saving || !f.list.find(s => s.key === c.column)?.claim || !!dispatched}
              data-tip={dispatched ? '已经派发过了：在上面取消派发或停止' : !f.list.find(s => s.key === c.column)?.claim ? '只有可领取状态里的卡片能派发' : '派给一台机器上的 agent'}
              onClick={() => openDispatch(c.id)}
            >
              <Icon name="dispatch" />
              派发
            </button>
            <button
              className="btn sm"
              type="button"
              aria-haspopup="menu"
              disabled={saving}
              onClick={e => {
                const open = useUI.getState().ctx;
                if (open?.kind === 'move' && open.id === String(c.id)) return closeCtx();
                openCtx({ kind: 'move', id: String(c.id), x: 0, y: 0, anchor: e.currentTarget.getBoundingClientRect() });
              }}
            >
              <Icon name="move" />
              移动到
            </button>
          </div>
        </div>
        <ActivityPart card={c} now={now} loading={loading} error={error} />
      </div>
    </div>
  );
}
let navDir = 0;

/** 自适应高度的输入框：宽度变了（拖动左右分栏、窗口缩放）也重新算高度，免得多出来的行被截掉。
    on：输入框不是一直在的（例如描述只在编辑时才有）时，传一个随之变化的值 */
function useRefit(ref: React.RefObject<HTMLTextAreaElement | null>, on?: unknown) {
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    let w = el.clientWidth;
    const ro = new ResizeObserver(() => {
      if (el.clientWidth === w) return;
      w = el.clientWidth;
      autosize(el);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref, on]);
}

/** 编辑到一半卡片被 agent 领取（锁住）时没保存的内容：「白板路径#卡片」→ 标题、描述。再次编辑时先恢复它 */
const drafts = new Map<string, { title?: string; body?: string }>();
const draftKey = (id: number) => `${useData.getState().path}#${id}`;
function takeDraft(id: number, field: 'title' | 'body'): string | undefined {
  const k = draftKey(id);
  const d = drafts.get(k);
  const v = d?.[field];
  if (d && v != null) {
    delete d[field];
    if (d.title == null && d.body == null) drafts.delete(k);
  }
  return v;
}
/** 存草稿并提示（人输入的内容不能丢）：提示里可以复制 */
function keepDraft(c: Card, field: 'title' | 'body', text: string) {
  const k = draftKey(c.id);
  drafts.set(k, { ...drafts.get(k), [field]: text });
  toast({
    type: 'info',
    title: `#${c.id} 被 ${c.holder ?? 'agent'} 领取了`,
    text: `没保存的${field === 'title' ? '标题' : '描述'}存成了草稿：释放后再编辑会恢复`,
    duration: 12000,
    action: {
      label: '复制',
      fn: () => void copyText(text).then(ok => toast({ type: ok ? 'success' : 'error', title: ok ? '已复制草稿' : '复制失败' })),
    },
  });
}

/** 卡片的标签：可以去掉，「添加标签」打开项目的标签选择器。卡片详情和「未整理」页的展开行共用。
    saving：这张卡片在保存中，× 和「添加标签」先不能点 */
export function CardLabels({ card: c, saving }: { card: Card; saving: boolean }) {
  const cur = useCurrent();
  const labels = cur?.project.labels ?? [];
  // 正在加上的标签（pending 态）
  const pendingIds = useData(s => s.pending[`${s.path}#${c.id}`]) ?? NO_IDS;
  const onLabels = (anchor: HTMLElement) =>
    cur &&
    openPicker({
      rect: anchor.getBoundingClientRect(),
      project: cur.project.key,
      selected: c.labels,
      onToggle: async (id, add) => {
        try {
          await (add ? addLabel(c.id, id) : removeLabel(c.id, id));
        } catch (e) {
          toastError(e);
        }
      },
    });
  return (
    <div className="d-labels">
      {/* 按项目里标签的顺序排，与侧栏、卡片上一致 */}
      {labels
        .filter(l => c.labels.includes(l.id))
        .map(l => (
          <LabelChip
            key={l.id}
            label={l}
            pending={pendingIds.includes(l.id)}
            removeDisabled={saving}
            onRemove={async () => {
              try {
                await removeLabel(c.id, l.id);
              } catch (e) {
                toastError(e);
              }
            }}
          />
        ))}
      <button type="button" className="lb-add" aria-label="添加标签" data-label-picker disabled={saving} onClick={e => onLabels(e.currentTarget)}>
        <Icon name="tag" />
        {c.labels.length ? '' : '添加标签'}
      </button>
    </div>
  );
}

/** 标题：失焦或回车时保存（当帧生效，被拒绝时改回来并提示）。卡片详情和「未整理」页的展开行共用。
    busy：这张卡片在保存中，先不能改（只读），保存完再改 */
export function TitleEditor({ card: c, locked, busy }: { card: Card; locked: boolean; busy: boolean }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useRefit(ref);
  const [v, setV] = useState(c.title);
  const editing = useRef(false);
  useEffect(() => {
    if (!editing.current) setV(c.title);
  }, [c.title]);
  // 改到一半被领取：改过的存成草稿，输入框回到已保存的标题
  const latest = useRef(v);
  latest.current = v;
  useEffect(() => {
    if (!locked || !editing.current) return;
    editing.current = false;
    const t = latest.current.trim();
    if (t && t !== c.title) keepDraft(c, 'title', latest.current);
    setV(c.title);
    ref.current?.blur();
  }, [locked]); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => autosize(ref.current), [v]);
  useEffect(() => {
    const t = requestAnimationFrame(() => autosize(ref.current));
    return () => cancelAnimationFrame(t);
  }, []);
  const commit = async () => {
    editing.current = false;
    const t = v.trim();
    if (locked || t === c.title) return;
    if (!t) {
      // 空标题服务端会拒绝：不发请求，直接改回去
      replay(ref.current, 'invalid');
      setV(c.title);
      return;
    }
    try {
      // 当帧生效（白板上的卡片一起变），被拒绝时改回来并提示。
      // 输入框不在这里改回：补丁撤掉后 c.title 变回去，上面的 effect 会跟着改；之后又改过一次时它停在那一次的值
      await exec({ op: 'edit', id: c.id, title: t }, editCard(c.id, { title: t }));
    } catch (e) {
      toastError(e);
      replay(ref.current, 'invalid');
    }
  };
  return (
    <textarea
      ref={ref}
      className="d-title"
      rows={1}
      maxLength={200}
      spellCheck={false}
      readOnly={locked || busy}
      value={v}
      aria-label="标题"
      onFocus={() => {
        if (locked || busy) return;
        editing.current = true;
        const d = takeDraft(c.id, 'title');
        if (d != null) setV(d);
      }}
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

/** 卡片的全部附件：图片缩略图点开放大，文件点击下载 */
function Attachments({ list }: { list: Attachment[] }) {
  // 图片附件可以在放大查看时左右切换
  const images = list.filter(a => a.image).map(a => ({ src: a.url, alt: a.name }));
  return (
    <section className="d-section" data-part="files" style={{ ['--i' as string]: 3 }}>
      <div className="d-label">
        附件 <span className="count">{list.length}</span>
      </div>
      <ul className="att-list">
        {list.map(a => (
          <li key={a.id}>
            {a.image ? (
              <button className="att-thumb" type="button" aria-label={`查看 ${a.name}`} onClick={() => openImage(a.url, a.name, images)}>
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

/** 描述：预览 / 编辑，编辑时可以粘贴截图、拖入文件。卡片详情和「未整理」页的展开行共用 */
export function DescEditor({ card: c, locked, busy }: { card: Card; locked: boolean; busy: boolean }) {
  const [mode, setMode] = useState<'view' | 'edit'>('view');
  const [v, setV] = useState(c.body);
  const ta = useRef<HTMLTextAreaElement>(null);
  const path = useData(s => s.path);
  const attach = useAttach({ board: path, card: c.id, setValue: setV, textarea: ta });
  useRefit(ta, mode);
  useEffect(() => {
    if (!locked || mode !== 'edit') return;
    // 编辑到一半被领取：改过的存成草稿（释放后再编辑时恢复），不丢
    if (v.trim() !== c.body.trim()) keepDraft(c, 'body', v);
    setMode('view');
  }, [locked, mode]); // eslint-disable-line react-hooks/exhaustive-deps
  const edit = () => {
    if (locked || busy) return;
    setV(takeDraft(c.id, 'body') ?? c.body);
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
    const body = v.trim();
    // 当帧切回预览、显示新描述；被拒绝时改回来并回到编辑，草稿还在
    setMode('view');
    if (body === c.body) return;
    try {
      await exec({ op: 'edit', id: c.id, body }, editCard(c.id, { body }));
    } catch (e) {
      toastError(e);
      setMode(m => (m === 'view' ? 'edit' : m));
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
              className="textarea"
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
          // 不响应双击：人常在这里选中文字阅读，双击进入编辑容易误触（进入编辑只用「编辑」按钮）
          <div className={cx('desc-view', locked && 'locked')}>
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

function ClaimPart({ card: c, now, busy }: { card: Card; now: number; busy: boolean }) {
  const start = boardFlow(useData(s => s.board)).home?.name ?? '待办';
  const online = useOnline();
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
  const t = timing(c, now, online);
  if (!c.holder) {
    // 做过、现在没人在做：耗时收成一行；没做过的不显示
    if (!t.runs) return null;
    return (
      <section className="d-section" data-part="claim" style={{ ['--i' as string]: 1 }}>
        <RunStats t={t} idle finishedAt={c.finishedAt} now={now} />
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
            持有者的后台心跳每 {BEAT_MS / 1000} 秒一次；连续 {BEAT_MISSES} 次没收到，名字离线、判定异常退出，卡片回到领取它时所在的状态
          </span>
          <button
            className="btn sm danger"
            type="button"
            disabled={busy}
            onClick={async () => {
              const holder = c.holder;
              const back = backTo(boardFlow(useData.getState().board), c.claimedFrom);
              // 危险操作：先二次确认，防止误触
              const yes = await confirm({
                title: `强制释放 #${c.id}？`,
                text: `${holder} 的领取会被清空，卡片回到「${back}」最上面。`,
                ok: '强制释放',
                icon: 'unlock',
              });
              if (!yes) return;
              // 当帧回到领取它时所在的状态最上面（服务端的规则），被拒绝时回来并提示
              const to = c.claimedFrom ?? boardFlow(useData.getState().board).home?.key ?? c.column;
              try {
                await exec({ op: 'release', id: c.id }, movePatch(c.id, to, 0, undefined, { holder: null, claimedAt: null, runStartedAt: null }));
                toast({ type: 'success', title: `已强制释放 #${c.id}`, text: `${holder} 的领取已清空，卡片回到「${back}」最上面` });
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

/** 耗时一行：累计 · 领取次数 · 心跳次数 · 异常退出（idle：没人在做时单独显示，前面带计时图标） */
function RunStats({ t, idle, finishedAt, now }: { t: ReturnType<typeof timing>; idle?: boolean; finishedAt?: number | null; now?: number }) {
  return (
    <div className={cx('run-stats', idle && 'idle')}>
      {idle && <Icon name="timer" />}
      <span>
        累计 <b>{clock(t.total)}</b>
      </span>
      <span>
        领取 <b>{t.runs}</b> 次
      </span>
      <span title="各次领取收到的心跳之和（含 final）">
        心跳 <b>{t.beats}</b> 次
      </span>
      <span>
        异常退出 <b className={cx(t.lost > 0 && 'bad')}>{t.lost}</b>
      </span>
      {finishedAt && now != null && (
        <span title={`完成于 ${dateTime(finishedAt)}`}>
          完成于 <b>{since(finishedAt, now)}</b>
        </span>
      )}
    </div>
  );
}

/** 动态骨架：系统事件行和一条评论气泡交替，条数按评论数估计 */
function ActivitySkeleton({ comments }: { comments: number }) {
  const rows = Math.min(6, 2 + comments);
  return (
    <div className="timeline sk-timeline" aria-busy="true" aria-label="正在加载动态">
      {Array.from({ length: rows }, (_, i) => {
        const bubble = comments > 0 && i % 2 === 1;
        return (
          <div key={i} className="tl-item sk-tl" aria-hidden="true">
            <Skeleton w={24} h={24} r={12} />
            {bubble ? (
              <div className="sk-bubble">
                <Skeleton w={88} h={10} />
                <SkeletonText lines={3} h={9} gap={7} last="48%" />
              </div>
            ) : (
              <div className="sk-line">
                <Skeleton w={['72%', '58%', '64%'][i % 3]} h={10} />
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function AttachmentsSkeleton() {
  return (
    <section className="d-section sk-attach" aria-busy="true" aria-label="正在加载附件">
      <Skeleton w={36} h={11} />
      <div className="sk-attach-row">
        <Skeleton w={96} h={72} r={10} />
        <Skeleton w={96} h={72} r={10} />
      </div>
    </section>
  );
}

function ActivityPart({ card: c, now, loading, error }: { card: Detail; now: number; loading: boolean; error: string | null }) {
  const scroller = useRef<HTMLDivElement>(null);
  const shown = useRef<number | null>(null);
  const [text, setText] = useState('');
  const ta = useRef<HTMLTextAreaElement>(null);
  const send = useRef<HTMLButtonElement>(null);
  const n = c.activity.length;
  const sending = useSending(c.id);
  const path = useData(s => s.path);
  // 输入超过一行后换成「上面输入、下面工具栏」的排法；清空前不换回去，免得宽度变化让它来回跳
  const [multi, setMulti] = useState(false);
  const fit = (el: HTMLTextAreaElement | null) => {
    autosize(el);
    if (!el) return;
    if (!el.value) setMulti(false);
    else if (!multi && el.scrollHeight > ONE_LINE) setMulti(true);
  };
  const attach = useAttach({ board: path, card: c.id, setValue: setText, textarea: ta });
  useRefit(ta);
  const firstNew = shown.current;
  // 这次打开时是不是从骨架开始的
  const wasLoading = useRef(loading);
  if (loading) wasLoading.current = true;

  // 默认显示到最新一条；新动态到来时如果停在底部就跟随
  useLayoutEffect(() => {
    const sc = scroller.current;
    if (!sc || !n) return;
    if (shown.current == null) sc.scrollTop = sc.scrollHeight;
    else if (n > shown.current && sc.scrollHeight - sc.scrollTop - sc.clientHeight < 160) sc.scrollTo({ top: sc.scrollHeight, behavior: 'smooth' });
    shown.current = n;
  }, [n]);

  // 乐观发送：输入框当帧清空，动态底部先出现「发送中」的气泡；失败时气泡移除、原文回到输入框
  // 高度、单行/多行跟着内容走（打字、粘贴、附件插入的占位文字都会改 text）
  useLayoutEffect(() => fit(ta.current), [text, multi]);

  const submit = () => {
    if (stillUploading(attach.busy)) return;
    const t = text.trim();
    if (!t) {
      ta.current?.focus();
      return;
    }
    setText('');
    send.current?.animate(
      [{ transform: 'rotate(-12deg)' }, { transform: 'translate(18px,-18px) rotate(-12deg)', opacity: 0 }, { transform: 'translate(-10px, 10px)', opacity: 0 }, { transform: 'none', opacity: 1 }],
      { duration: 520, easing: 'cubic-bezier(.16,1,.3,1)' },
    );
    requestAnimationFrame(() => scroller.current?.scrollTo({ top: scroller.current.scrollHeight, behavior: 'smooth' }));
    sendComment(c.id, t).catch(e => {
      // 期间又写了新内容：原文放在前面，两段都留着
      setText(cur => (cur.trim() ? `${t}\n\n${cur}` : t));
      toastError(e);
    });
  };

  return (
    <div className="d-side" data-part="activity">
      <SideSplitter />
      <div className="d-label d-side-head">
        动态 {loading ? <Skeleton className="count" w={14} h={10} /> : <span className="count">{n}</span>}
        <span className="act">只能追加</span>
      </div>
      <div className="d-activity" ref={scroller}>
        {loading ? (
          <ActivitySkeleton comments={c.comments} />
        ) : error && !n ? (
          <div className="d-activity-empty" role="alert">
            <b>动态没有加载出来</b>
            <span>{error}</span>
            <button className="btn sm" type="button" onClick={() => openDetail(c.id)}>
              重试
            </button>
          </div>
        ) : !n && !sending.length ? (
          <div className="d-activity-empty">还没有动态</div>
        ) : (
          // 从骨架换成内容时淡入（已缓存的直接显示，不播放）
          <div className={cx('timeline', wasLoading.current && 'sk-swap')}>
            {c.activity.map((a, idx) => (
              <ActivityItem key={a.id} a={a} card={c.id} now={now} fresh={firstNew != null && idx >= firstNew} />
            ))}
            {sending.map(s => (
              <div key={`sending-${s.key}`} className="tl-item mine new sending" aria-busy="true">
                <div>
                  <Avatar actor={ME} />
                </div>
                <div className="tl-bubble">
                  <div className="who">
                    <b>{actorName(ME)}</b>
                    <span className="tl-time sending-tag">
                      <i aria-hidden="true" />
                      发送中
                    </span>
                  </div>
                  <Markdown src={s.text} />
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
      <div className="d-comment">
        <div className={cx('comment-box', multi && 'multi', text.trim() && 'has-text', attach.over && 'drop-on')}>
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
            onChange={e => setText(e.target.value)}
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

/* ------------------------------ 左右栏分隔条 ------------------------------ */

const SIDE_KEY = 'board.detail.side';
const SIDE_MIN = 260;
const SIDE_MAX = 680;
/** 左栏至少留这么宽；和 overlays.css 里 .d-grid 的 clamp 一致 */
const MAIN_MIN = 320;

const savedSide = () => {
  try {
    const v = Number(localStorage.getItem(SIDE_KEY));
    return Number.isFinite(v) && v > 0 ? v : null;
  } catch {
    return null;
  }
};
const saveSide = (v: number | null) => {
  try {
    if (v == null) localStorage.removeItem(SIDE_KEY);
    else localStorage.setItem(SIDE_KEY, String(Math.round(v)));
  } catch {
    /* 忽略 */
  }
};

/** 动态栏左边的分隔条：拖动改右栏宽度（写到 .d-grid 的 --modal-side，记在本机），双击恢复默认。
    宽度范围由 CSS 的 clamp 兜底：窗口变窄时左栏仍至少留 MAIN_MIN */
function SideSplitter() {
  const ref = useRef<HTMLDivElement>(null);
  const grid = () => ref.current?.closest<HTMLElement>('.d-grid') ?? null;
  const drag = useRef<{ x0: number; w0: number; max: number; w: number } | null>(null);

  useLayoutEffect(() => {
    const v = savedSide();
    if (v != null) grid()?.style.setProperty('--modal-side', `${v}px`);
  }, []);

  const end = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    document.body.classList.remove('is-resizing');
    ref.current?.classList.remove('active');
    try {
      ref.current?.releasePointerCapture(e.pointerId);
    } catch {
      /* 已经释放了 */
    }
    saveSide(d.w);
  };

  return (
    <div
      ref={ref}
      className="d-split"
      role="separator"
      aria-orientation="vertical"
      aria-label="拖动调整动态栏宽度，双击恢复默认"
      title="拖动调整宽度，双击恢复默认"
      onPointerDown={e => {
        if (e.button !== 0) return;
        const g = grid();
        const side = ref.current?.parentElement;
        if (!g || !side) return;
        e.preventDefault();
        e.stopPropagation();
        const w0 = side.getBoundingClientRect().width;
        drag.current = { x0: e.clientX, w0, w: w0, max: Math.max(SIDE_MIN, Math.min(SIDE_MAX, g.clientWidth - MAIN_MIN)) };
        ref.current?.setPointerCapture(e.pointerId);
        ref.current?.classList.add('active');
        document.body.classList.add('is-resizing');
      }}
      onPointerMove={e => {
        const d = drag.current;
        if (!d) return;
        d.w = Math.round(Math.min(d.max, Math.max(SIDE_MIN, d.w0 - (e.clientX - d.x0))));
        grid()?.style.setProperty('--modal-side', `${d.w}px`);
      }}
      onPointerUp={end}
      onPointerCancel={end}
      onDoubleClick={() => {
        grid()?.style.removeProperty('--modal-side');
        saveSide(null);
      }}
    />
  );
}

/** 原地编辑自己发的评论：预填原 Markdown（上次没保存成功时是那次的草稿），⌘↵ 保存、Esc 取消；空内容不能保存。
    保存是乐观的：编辑框当即关闭、气泡显示新内容（「保存中」）；被拒绝时改回来，并带着草稿重新打开编辑框 */
function CommentEditor({ a, card, initial, onDone, onFail }: { a: Activity; card: number; initial: string; onDone: () => void; onFail: (draft: string) => void }) {
  const [draft, setDraft] = useState(initial);
  const ta = useRef<HTMLTextAreaElement>(null);
  const empty = !draft.trim();
  useLayoutEffect(() => autosize(ta.current), [draft]);
  useRefit(ta);
  useEffect(() => {
    const el = ta.current;
    if (!el) return;
    el.focus({ preventScroll: true });
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);
  const save = () => {
    if (empty) {
      toast({ type: 'error', title: '评论内容不能为空', text: '不想要这条评论的话，按 Esc 取消修改' });
      ta.current?.focus();
      return;
    }
    if (draft.trim() === a.text.trim()) return onDone();
    const text = draft;
    onDone();
    exec({ op: 'edit_comment', id: card, activity: a.id, text }, editComment(card, a.id, text.trim())).catch(e => {
      toastError(e);
      onFail(text);
    });
  };
  return (
    <div className={cx('tl-editor', empty && 'empty')}>
      <textarea
        ref={ta}
        value={draft}
        rows={1}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        aria-label="修改评论"
        onChange={e => setDraft(e.target.value)}
        onKeyDown={e => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            save();
          }
          if (e.key === 'Escape') {
            e.stopPropagation();
            onDone();
          }
        }}
      />
      <div className="tl-editor-bar">
        <span className={cx('tl-editor-hint', empty && 'bad')}>{empty ? '评论内容不能为空' : `${accel('↵')} 保存 · Esc 取消`}</span>
        <button className="btn sm ghost" type="button" onClick={onDone}>
          取消
        </button>
        <button className="btn sm primary" type="button" onClick={save} disabled={empty}>
          保存
        </button>
      </div>
    </div>
  );
}

/** 改评论：动态里的这条当帧换成新内容并标「已编辑」，画成保存中 */
const editComment = (card: number, act: number, text: string) => ({
  detail: (d: Detail) => (d.id === card ? { ...d, activity: d.activity.map(x => (x.id === act ? { ...x, text, edited_at: serverNow() } : x)) } : d),
  acts: [act],
});

/** 本窗口发出的评论：发送中气泡的作者 */
const ME: Activity['actor'] = { kind: 'human', name: '' };
/** 评论框一行的高度（line-height 13.5 × 1.5 + 上下内边距 6 × 2），超过就算多行 */
const ONE_LINE = 34;

function ActivityItem({ a, card, now, fresh }: { a: Activity; card: number; now: number; fresh: boolean }) {
  // 自己发的评论从「发送中」转正：不再播入场动画，只把透明度补回来
  const settled = isSettled(a.id);
  if (settled) fresh = false;
  // 正在编辑时是草稿的初始内容
  const [editing, setEditing] = useState<string | null>(null);
  const saving = useData(s => !!s.saving[`${s.path}@${a.id}`]);
  const name =
    a.actor.kind === 'agent' ? (
      <b className="agent-name" style={{ color: agentInk(a.actor.name) }}>
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
      <div className={cx('tl-item', a.actor.kind === 'human' && 'mine', fresh && 'new', settled && 'settled', saving && 'sending')} aria-busy={saving || undefined}>
        <div>
          <Avatar actor={a.actor} />
        </div>
        <div className="tl-bubble">
          <div className="who">
            {name}
            {a.actor.kind === 'agent' && (
              <span className="chip" style={{ height: 18, fontSize: 11 }}>
                CLI
              </span>
            )}
            {time}
            {a.edited_at != null && (
              <span className="tl-edited" title={`修改于 ${new Date(a.edited_at).toLocaleString()}`}>
                已编辑
              </span>
            )}
            {saving && (
              <span className="tl-time sending-tag">
                <i aria-hidden="true" />
                保存中
              </span>
            )}
            {a.actor.kind === 'human' && editing == null && !saving && (
              <span className="tl-actions">
                <button className="tl-edit" type="button" aria-label="编辑这条评论" title="编辑" onClick={() => setEditing(a.text)}>
                  <Icon name="edit" />
                </button>
                <button
                  className="tl-edit tl-del"
                  type="button"
                  aria-label="删除这条评论"
                  title="删除"
                  onClick={async () => {
                    const yes = await confirm({ title: '删除这条评论？', text: '删除后无法恢复。', ok: '删除', icon: 'trash' });
                    if (!yes) return;
                    try {
                      await exec({ op: 'delete_comment', id: card, activity: a.id }, removeComment(card, a.id));
                      toast({ type: 'success', title: '已删除评论' });
                    } catch (e) {
                      toastError(e);
                    }
                  }}
                >
                  <Icon name="trash" />
                </button>
              </span>
            )}
          </div>
          {editing != null ? (
            <CommentEditor a={a} card={card} initial={editing} onDone={() => setEditing(null)} onFail={setEditing} />
          ) : (
            <Markdown src={a.text} />
          )}
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
