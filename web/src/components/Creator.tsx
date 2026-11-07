/* ==========================================================================
   新建（PRD §10.7）：白板「新建卡片」与「记一条未整理」共用的居中弹窗
   - ↵ 在标题里创建；⌘/Ctrl ↵ 在任何位置创建；⇧↵ 跳到描述
   - 「连续新建」：创建后清空标题和描述、保留标签，并列出本次已创建的卡片
   - 关闭时未提交的内容保留为草稿，下次打开自动恢复，可以一键清空
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { openCard } from '../lib/actions';
import { stillUploading, useAttach } from '../lib/attach';
import { boardFlow } from '../lib/schema';
import { accel, autosize, cx, pref, replay } from '../lib/util';
import { closeDetail, exec, useCurrent, useData } from '../store/data';
import { closeCreator, openPicker, reveal, setTab, toast, toastError, useUI } from '../store/ui';
import { LabelChip, Markdown } from './bits';
import { DescTabs } from './CardDetail';
import { BoardIcon24, Icon, InboxIcon24 } from './Icon';

interface Draft {
  title: string;
  body: string;
  column: 'todo' | 'inbox';
  labels: number[];
}
const DRAFT = 'creator.draft';
const TEMPLATE = '**验收标准**\n- [ ] \n\n**相关链接**\n- ';

export function Creator() {
  const { open, column: want, seq } = useUI(s => s.creator);
  const cur = useCurrent();
  const path = useData(s => s.path);
  const f = boardFlow(useData(s => s.board));
  const startName = f.home?.name ?? '待办';
  const startColor = f.color(f.home?.key ?? 'todo');
  const [column, setColumn] = useState<'todo' | 'inbox'>('todo');
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [labels, setLabels] = useState<number[]>([]);
  const [mode, setMode] = useState<'edit' | 'preview'>('edit');
  const [restored, setRestored] = useState(false);
  const [error, setError] = useState('');
  const [keep, setKeep] = useState(() => pref.get('creator.keep', false));
  const [created, setCreated] = useState<{ id: number; title: string; column: 'todo' | 'inbox' }[]>([]);
  const [busy, setBusy] = useState(false);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const root = useRef<HTMLElement>(null);
  const state = useRef({ title, body, column, labels });
  state.current = { title, body, column, labels };
  // 附件：还没有卡片，先上传，新建时按描述里的引用归到卡片
  const attach = useAttach({ board: path, card: null, setValue: setBody, textarea: bodyRef });

  // 打开：恢复草稿
  useEffect(() => {
    if (!open) return;
    closeDetail();
    const d = pref.get<Draft | null>(DRAFT, null);
    const has = !!d && !!(d.title || d.body);
    const valid = new Set((cur?.project.labels ?? []).map(l => l.id));
    setColumn(want);
    setTitle(has ? d!.title : '');
    setBody(has ? d!.body : '');
    setLabels(has ? (d!.labels || []).filter(id => valid.has(id)) : []);
    setRestored(has);
    setCreated([]);
    setMode('edit');
    setError('');
    requestAnimationFrame(() => {
      const t = titleRef.current;
      if (!t) return;
      autosize(t);
      t.focus();
      t.setSelectionRange(t.value.length, t.value.length);
    });
  }, [open, seq]); // eslint-disable-line react-hooks/exhaustive-deps

  useLayoutEffect(() => autosize(titleRef.current), [title]);

  const saveDraft = () => {
    const s = state.current;
    pref.set(DRAFT, s.title.trim() || s.body.trim() ? s : null);
  };
  const close = () => {
    saveDraft();
    closeCreator();
  };

  useEffect(() => {
    const unload = () => useUI.getState().creator.open && saveDraft();
    window.addEventListener('beforeunload', unload);
    return () => window.removeEventListener('beforeunload', unload);
  }, []);

  const submit = async () => {
    if (busy || stillUploading(attach.busy)) return;
    if (!title.trim()) {
      setError('标题是必填的');
      replay(titleRef.current, 'invalid');
      titleRef.current?.focus();
      return;
    }
    setBusy(true);
    try {
      const c = await exec({ op: 'new', column: column === 'inbox' ? 'inbox' : undefined, title, body, labels });
      pref.set(DRAFT, null);
      setRestored(false);
      if (keep) {
        setCreated(list => [{ id: c.id, title: c.title, column }, ...list]);
        setTitle('');
        setBody('');
        setMode('edit');
        setError('');
        titleRef.current?.focus();
        root.current
          ?.querySelector('.cr-submit')
          ?.animate([{ transform: 'scale(.94)' }, { transform: 'scale(1.03)' }, { transform: 'none' }], { duration: 360, easing: 'cubic-bezier(.34,1.56,.64,1)' });
        return;
      }
      closeCreator();
      const tab = useUI.getState().tab;
      if (column === 'inbox') {
        if (tab !== 'inbox')
          toast({
            type: 'success',
            title: `已记到「未整理」 #${c.id}`,
            text: c.title,
            action: {
              label: '查看',
              fn: () => {
                setTab('inbox');
                setTimeout(() => reveal(c.id), 120);
              },
            },
          });
        else setTimeout(() => reveal(c.id), 80);
      } else {
        if (tab === 'settings' || tab === 'projects') setTab('board');
        setTimeout(() => reveal(c.id), 80);
      }
    } catch (e) {
      if ((e as { code?: string }).code === 'EMPTY_TITLE') setError('标题是必填的');
      else toastError(e);
    } finally {
      setBusy(false);
    }
  };

  // ⌘/Ctrl ↵ 在任何位置创建；Esc 关闭（保留草稿）
  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => {
      if (useUI.getState().picker || useUI.getState().labelsOf) return;
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        e.stopPropagation();
        submit();
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  });

  const q = (s: string) => `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')}"`;
  const short = body.trim().length > 40 ? body.trim().slice(0, 40) + '…' : body.trim();
  const projectLabels = cur?.project.labels ?? [];

  return (
    <>
    <div className={cx('scrim', open && 'on')} onClick={close} />
    <aside
      ref={root}
      className={cx('drawer creator', open && 'on', title.trim() && 'has-title')}
      role="dialog"
      aria-modal="true"
      aria-label={column === 'inbox' ? '记一条未整理' : '新建卡片'}
      aria-hidden={!open}
      data-dest={column}
      onKeyDown={e => {
        if (e.key !== 'Tab') return;
        const f = [...(root.current?.querySelectorAll<HTMLElement>('button:not([disabled]), textarea:not([hidden]), input:not([type="radio"])') ?? [])].filter(
          n => n.offsetParent !== null,
        );
        if (!f.length) return;
        const first = f[0];
        const last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }}
    >
      <div className="drawer-body">
        <div className="d-head">
          <span className="cr-badge">
            <Icon name="plus" sw={1.9} />
            <span>{column === 'inbox' ? '记一条未整理' : '新建卡片'}</span>
          </span>
          {cur && (
            <span className="cr-project">
              <i style={{ background: cur.project.color }} />
              {cur.project.name} / {cur.board.name}
            </span>
          )}
          {restored && (
            <span className="cr-draft">
              已恢复上次未提交的草稿
              <button
                type="button"
                onClick={() => {
                  setTitle('');
                  setBody('');
                  pref.set(DRAFT, null);
                  setRestored(false);
                  titleRef.current?.focus();
                }}
              >
                清空
              </button>
            </span>
          )}
          <span className="spacer" />
          <button className="icon-btn" type="button" data-tip="关闭（保留草稿）  esc" data-tip-align="end" onClick={close}>
            <Icon name="close" />
          </button>
        </div>
        <form
          className="d-grid cr-form"
          noValidate
          onSubmit={e => {
            e.preventDefault();
            submit();
          }}
        >
          <div className="d-main cr-main">
            <textarea
              ref={titleRef}
              className="d-title cr-title"
              rows={1}
              maxLength={200}
              spellCheck={false}
              placeholder="标题（必填）"
              aria-label="标题"
              value={title}
              onChange={e => {
                setTitle(e.target.value);
                if (e.target.value.trim()) setError('');
              }}
              onKeyDown={e => {
                if (e.key === 'Enter' && !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  submit();
                }
                if (e.key === 'Enter' && e.shiftKey) {
                  e.preventDefault();
                  setMode('edit');
                  requestAnimationFrame(() => bodyRef.current?.focus());
                }
              }}
            />
            {error && (
              <div className="cr-error" role="alert">
                {error}
              </div>
            )}
            <div className="cr-keys">
              <kbd>↵</kbd> 创建 · <kbd>⇧↵</kbd> 写描述 · <kbd>{accel('↵')}</kbd> 在任意位置创建
            </div>
            <div className="d-label cr-label">
              描述 <span className="opt">选填 · Markdown</span>
              <div className="act">
                <button
                  className="btn sm ghost"
                  type="button"
                  onClick={() => {
                    setMode('edit');
                    const v = body;
                    const next = v + (v && !v.endsWith('\n') ? '\n\n' : '') + TEMPLATE;
                    setBody(next);
                    const pos = next.indexOf('- [ ] ', v.length) + 6;
                    requestAnimationFrame(() => {
                      bodyRef.current?.focus();
                      bodyRef.current?.setSelectionRange(pos, pos);
                    });
                  }}
                >
                  插入验收标准
                </button>
                <button
                  className="btn sm ghost"
                  type="button"
                  title="附件：也可以直接粘贴截图、拖入文件（≤ 10 MB）"
                  onClick={() => {
                    setMode('edit');
                    attach.pick();
                  }}
                >
                  <Icon name="clip" />
                  {attach.busy ? '上传中…' : '附件'}
                </button>
                {attach.fileInput}
                <DescTabs
                  mode={mode}
                  setMode={m => {
                    setMode(m as 'edit' | 'preview');
                    if (m === 'edit') requestAnimationFrame(() => bodyRef.current?.focus());
                  }}
                  modes={[
                    ['edit', '编辑'],
                    ['preview', '预览'],
                  ]}
                />
              </div>
            </div>
            <div className={cx('cr-desc', attach.over && 'drop-on')}>
              <textarea
                ref={bodyRef}
                {...attach.handlers}
                className="textarea scroll"
                spellCheck={false}
                hidden={mode !== 'edit'}
                placeholder={'验收标准、相关链接、背景……\n\n例如：\n**验收标准**\n- [ ] board ls --search 能匹配描述'}
                value={body}
                onChange={e => setBody(e.target.value)}
              />
              {mode === 'preview' && (
                <div className="cr-preview scroll">{body.trim() ? <Markdown src={body} /> : <p className="cr-empty">还没有描述</p>}</div>
              )}
            </div>
          </div>
          <div className="d-side cr-side">
            <div className="cr-sec">
              <div className="d-label">放到</div>
              <div className="cr-dest" role="radiogroup" aria-label="放到">
                {(
                  [
                    ['todo', startName, '放在最下面，agent 可以领取', <BoardIcon24 key="b" />],
                    ['inbox', '未整理', `先记下来，整理后再放进${startName}`, <InboxIcon24 key="i" />],
                  ] as const
                ).map(([k, name, hint, icon]) => (
                  <label
                    key={k}
                    className={cx('dest', column === k && 'on')}
                    data-col={k}
                    style={{ ['--c' as string]: k === 'inbox' ? 'var(--c-inbox)' : startColor }}
                    onClick={e => {
                      e.preventDefault();
                      setColumn(k);
                      root.current
                        ?.querySelector('.cr-badge')
                        ?.animate([{ transform: 'scale(.9)' }, { transform: 'none' }], { duration: 300, easing: 'cubic-bezier(.34,1.56,.64,1)' });
                      titleRef.current?.focus();
                    }}
                  >
                    <input type="radio" name="dest" value={k} checked={column === k} readOnly />
                    <span className="dest-ic">{icon}</span>
                    <span className="dest-text">
                      <b>{name}</b>
                      <span>{hint}</span>
                    </span>
                    <span className="dest-check" />
                  </label>
                ))}
              </div>
            </div>
            <div className="cr-sec">
              <div className="d-label">
                标签 <span className="opt">选填</span>
              </div>
              <div className="cr-labels">
                {labels
                  .map(id => projectLabels.find(l => l.id === id))
                  .filter(Boolean)
                  .map(l => (
                    <LabelChip key={l!.id} label={l!} onRemove={() => setLabels(ls => ls.filter(x => x !== l!.id))} />
                  ))}
                <button
                  type="button"
                  className="lb-add"
                  data-label-picker
                  onClick={e =>
                    cur &&
                    openPicker({
                      rect: e.currentTarget.getBoundingClientRect(),
                      project: cur.project.key,
                      selected: labels,
                      onToggle: (id, on) => setLabels(ls => (on ? [...new Set([...ls, id])] : ls.filter(x => x !== id))),
                    })
                  }
                >
                  <Icon name="tag" />
                  {labels.length ? '' : '添加标签'}
                </button>
              </div>
            </div>
            <div className="cr-sec">
              <div className="d-label">等价的 CLI</div>
              <div className="cr-cli mono">
                {column === 'inbox' ? (
                  <span className="dim">「未整理」只在界面上新建，整理后再放进{startName}</span>
                ) : (
                  <>
                    <span className="cmd">board</span> new {q(title.trim() || '<标题>')}
                    {short ? ` --body ${q(short)}` : ''} <span className="dim">--board</span> {path}
                  </>
                )}
              </div>
            </div>
            {created.length > 0 && (
              <div className="cr-sec cr-recent">
                <div className="d-label">
                  本次已创建 <span className="count">{created.length}</span>
                </div>
                <div className="cr-recent-list scroll">
                  {created.map((c, i) => (
                    <button
                      key={c.id}
                      type="button"
                      className={cx('cr-recent-row', i === 0 && 'new')}
                      style={{ ['--c' as string]: c.column === 'inbox' ? 'var(--c-inbox)' : startColor }}
                      onClick={() => {
                        close();
                        if (c.column === 'inbox') setTab('inbox');
                        setTimeout(() => openCard(c.id), 120);
                      }}
                    >
                      <i />
                      <span className="mono">#{c.id}</span>
                      <span className="tt">{c.title}</span>
                      <em>{c.column === 'inbox' ? '未整理' : startName}</em>
                    </button>
                  ))}
                </div>
              </div>
            )}
            <div className="cr-foot">
              <label className="cr-keep">
                <button
                  type="button"
                  className={cx('toggle', keep && 'on')}
                  role="switch"
                  aria-checked={keep}
                  onClick={() => {
                    setKeep(!keep);
                    pref.set('creator.keep', !keep);
                  }}
                >
                  <i />
                </button>
                <span>
                  连续新建<em>创建后清空，继续录入下一条</em>
                </span>
              </label>
              <div className="cr-actions">
                <button className="btn" type="button" onClick={close}>
                  取消
                </button>
                <button className="btn brand cr-submit" type="submit" disabled={busy}>
                  <span>{column === 'inbox' ? '记到未整理' : `创建到${startName}`}</span>
                  <kbd>{accel('↵')}</kbd>
                </button>
              </div>
            </div>
          </div>
        </form>
      </div>
    </aside>
    </>
  );
}
