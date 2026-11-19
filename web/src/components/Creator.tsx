/* ==========================================================================
   新建（PRD §10.7）：白板「新建卡片」与「记一条未整理」共用的居中弹窗
   - ↵ 在标题里创建；⌘/Ctrl ↵ 在任何位置创建；⇧↵ 跳到描述
   - 「连续新建」：创建后清空标题和描述、保留标签，并列出本次已创建的卡片
   - 关闭时未提交的内容保留为草稿，下次打开自动恢复，可以一键清空
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { openCard } from '../lib/actions';
import { stillUploading, useAttach } from '../lib/attach';
import { addPrompt, insertPromptText, listPrompts, removePrompt, savePrompts, updatePrompt, type Prompt } from '../lib/prompts';
import { boardFlow } from '../lib/schema';
import { isWindows } from '../lib/platform';
import { accel, autosize, copyText, cx, pref, replay } from '../lib/util';
import { closeDetail, draftCard, exec, useCurrent, useData } from '../store/data';
import { closeCreator, openCreator, openPicker, reveal, setTab, toast, toastError, useUI } from '../store/ui';
import { DescTabs, LabelChip, Markdown, Tip, Title } from './bits';
import { BoardIcon24, Icon, InboxIcon24 } from './Icon';

interface Draft {
  title: string;
  body: string;
  column: 'todo' | 'inbox';
  labels: number[];
}
interface FailedDraft extends Draft {
  key: string;
}
interface FailedQueue {
  v: 1;
  items: FailedDraft[];
}
const DRAFT = 'creator.draft';
const FAILED = 'creator.failedDrafts';
const hasDraft = (d: Draft) => !!(d.title.trim() || d.body.trim());
const readFailed = (): FailedDraft[] => {
  const q = pref.get<FailedQueue | null>(FAILED, null);
  return q?.v === 1 && Array.isArray(q.items) ? q.items : [];
};
let failedSeq = 0;
const makeFailedDraft = (d: Draft): FailedDraft => ({ ...d, key: `${Date.now().toString(36)}-${++failedSeq}` });

export function Creator() {
  const { open, column: want, seq } = useUI(s => s.creator);
  const cur = useCurrent();
  const path = useData(s => s.path);
  const repo = useData(s => s.board?.repo);
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
  // 快捷提示词：浮层、本机存的列表、正在新建 / 编辑的那条、等待确认删除的 id
  const [promptsOpen, setPromptsOpen] = useState(false);
  const [prompts, setPrompts] = useState<Prompt[]>([]);
  const [promptEdit, setPromptEdit] = useState<{ id: string | null; title: string; text: string } | null>(null);
  const [promptDelete, setPromptDelete] = useState<string | null>(null);
  const [promptError, setPromptError] = useState('');
  const [keep, setKeep] = useState(() => pref.get('creator.keep', false));
  const [failed, setFailed] = useState<FailedDraft[]>(readFailed);
  const failedRef = useRef(failed);
  const pending = useRef(new Map<number, Draft>());
  const pendingRestore = useRef<Draft | null>(null);
  // 本次已创建：还没确认的 id 是负数（临时卡片），确认后换成真正的 ID
  const [created, setCreated] = useState<{ key: number; id: number; title: string; column: 'todo' | 'inbox' }[]>([]);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const root = useRef<HTMLElement>(null);
  const promptBox = useRef<HTMLDivElement>(null);
  const promptBtn = useRef<HTMLButtonElement>(null);
  const state = useRef({ title, body, column, labels });
  state.current = { title, body, column, labels };
  // 附件：还没有卡片，先上传，新建时按描述里的引用归到卡片
  const attach = useAttach({ board: path, card: null, setValue: setBody, textarea: bodyRef });

  const writeFailed = (next: FailedDraft[]) => {
    const items = next.slice(-8);
    failedRef.current = items;
    setFailed(items);
    pref.set(FAILED, items.length ? ({ v: 1, items } satisfies FailedQueue) : null);
  };

  const restoreFailed = (d: FailedDraft) => {
    const current = state.current;
    const next = failedRef.current.filter(x => x.key !== d.key);
    if (hasDraft(current)) {
      next.push(makeFailedDraft(current));
      pref.set(DRAFT, current);
    }
    writeFailed(next);
    pendingRestore.current = d;
    openCreator(d.column);
  };

  // 打开：恢复草稿
  useEffect(() => {
    if (!open) return;
    closeDetail();
    const d = pendingRestore.current ?? pref.get<Draft | null>(DRAFT, null);
    pendingRestore.current = null;
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
    setPromptsOpen(false);
    setPromptEdit(null);
    setPromptDelete(null);
    setPromptError('');
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

  // 提示词浮层：点外面关掉（点按钮本身不算，按钮的点击会自己切换）
  useEffect(() => {
    if (!promptsOpen) return;
    const down = (e: MouseEvent) => {
      if (promptBox.current?.contains(e.target as Node)) return;
      setPromptsOpen(false);
      setPromptEdit(null);
      setPromptDelete(null);
      setPromptError('');
    };
    document.addEventListener('mousedown', down, true);
    return () => document.removeEventListener('mousedown', down, true);
  }, [promptsOpen]);

  const closePrompts = (restore = false) => {
    setPromptsOpen(false);
    setPromptEdit(null);
    setPromptDelete(null);
    setPromptError('');
    if (restore) promptBtn.current?.focus();
  };

  const togglePrompts = () => {
    if (promptsOpen) return closePrompts();
    setPrompts(listPrompts());
    setPromptEdit(null);
    setPromptDelete(null);
    setPromptError('');
    setPromptsOpen(true);
  };

  /** 点一条：文本框有焦点时插在光标 / 选中处，没焦点时追加到末尾（和附件插入不同，不额外加换行） */
  const insertPrompt = (p: Prompt) => {
    const ta = bodyRef.current;
    const active = !!ta && document.activeElement === ta;
    const next = insertPromptText(body, active ? ta!.selectionStart : null, active ? ta!.selectionEnd : null, p.text);
    setBody(next.value);
    setMode('edit');
    requestAnimationFrame(() => {
      const el = bodyRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(next.caret, next.caret);
      autosize(el);
    });
  };

  const startPromptNew = () => {
    setPromptEdit({ id: null, title: '', text: '' });
    setPromptDelete(null);
    setPromptError('');
  };

  const startPromptEdit = (p: Prompt) => {
    setPromptEdit({ id: p.id, title: p.title, text: p.text });
    setPromptDelete(null);
    setPromptError('');
  };

  const savePrompt = () => {
    if (!promptEdit) return;
    const title = promptEdit.title.trim();
    const text = promptEdit.text;
    if (!title) return setPromptError('标题是必填的');
    if (!text.trim()) return setPromptError('内容是必填的');
    const next = promptEdit.id ? updatePrompt(prompts, promptEdit.id, title, text) : addPrompt(prompts, title, text);
    savePrompts(next);
    setPrompts(next);
    setPromptEdit(null);
    setPromptError('');
    requestAnimationFrame(() => promptBtn.current?.focus());
  };

  const deletePrompt = (id: string) => {
    const next = removePrompt(prompts, id);
    savePrompts(next);
    setPrompts(next);
    setPromptDelete(null);
  };

  /** 乐观新建：当帧关闭（连续新建时清空）、临时卡片出现在目标状态最下面；服务端确认后换成真正的卡片。
      失败时临时卡片消失，内容存回草稿，提示里可以一键恢复 */
  const submit = () => {
    if (stillUploading(attach.busy)) return;
    if (!title.trim()) {
      setError('标题是必填的');
      replay(titleRef.current, 'invalid');
      titleRef.current?.focus();
      return;
    }
    const draft: Draft = { title, body, column, labels };
    const home = f.home?.key ?? 'todo';
    const temp = draftCard({ title, body, labels, column: column === 'inbox' ? 'inbox' : home });
    pending.current.set(temp.id, draft);
    const done = exec({ op: 'new', column: column === 'inbox' ? 'inbox' : undefined, title, body, labels }, temp.patch, c => temp.settle(c.id));
    pref.set(DRAFT, null);
    setRestored(false);
    const tab = useUI.getState().tab;
    if (keep) {
      setCreated(list => [{ key: temp.id, id: temp.id, title: title.trim(), column }, ...list]);
      setTitle('');
      setBody('');
      setMode('edit');
      setError('');
      titleRef.current?.focus();
      root.current
        ?.querySelector('.cr-submit')
        ?.animate([{ transform: 'scale(.94)' }, { transform: 'scale(1.03)' }, { transform: 'none' }], { duration: 360, easing: 'cubic-bezier(.34,1.56,.64,1)' });
    } else {
      closeCreator();
      if (column !== 'inbox') {
        if (tab === 'settings' || tab === 'projects') setTab('board');
        setTimeout(() => reveal(temp.id, false), 80);
      } else if (tab === 'inbox') setTimeout(() => reveal(temp.id, false), 80);
    }
    done.then(
      c => {
        pending.current.delete(temp.id);
        setCreated(list => list.map(x => (x.id === temp.id ? { ...x, id: c.id } : x)));
        // 记到「未整理」而人不在未整理页：确认后告诉他记到了哪里
        if (!keep && column === 'inbox' && tab !== 'inbox')
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
      },
      e => {
        const entry = makeFailedDraft(pending.current.get(temp.id) ?? draft);
        pending.current.delete(temp.id);
        setCreated(list => list.filter(x => x.id !== temp.id));
        // 内容存回草稿（弹窗关着、或者已经清空时），下次打开自动恢复
        const open = useUI.getState().creator.open;
        const free = !open || !(state.current.title.trim() || state.current.body.trim());
        if (!free) {
          writeFailed([...failedRef.current, entry]);
          return void toastError(e, {
            text: `「${draft.title.trim()}」没有建成：${(e as Error).message}。这份内容已另外保存，当前输入没有改动`,
            action: { label: '恢复失败草稿', fn: () => restoreFailed(entry) },
            duration: 8000,
          });
        }
        pref.set(DRAFT, draft);
        toastError(e, {
          text: `「${draft.title.trim()}」没有建成：${(e as Error).message}。内容已存为草稿`,
          action: {
            label: '恢复草稿',
            fn: () => {
              if (open) closeCreator();
              setTimeout(() => openCreator(draft.column), open ? 60 : 0);
            },
          },
          duration: 8000,
        });
      },
    );
  };

  // ⌘/Ctrl ↵ 在任何位置创建；Esc 关闭（保留草稿）
  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => {
      if (useUI.getState().picker || useUI.getState().labelsOf) return;
      // 提示词浮层开着时，Esc 只关它，Ctrl/⌘ ↵ 也不在背后把卡片建出去
      if (promptsOpen) {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          closePrompts(true);
        }
        return;
      }
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
  const promptSummary = (s: string) => {
    const one = s.replace(/\s+/g, ' ').trim();
    return one.length > 72 ? one.slice(0, 72) + '…' : one;
  };
  const projectLabels = cur?.project.labels ?? [];
  const labelNames = labels.map(id => projectLabels.find(l => l.id === id)?.name).filter((n): n is string => !!n);
  // 复制用的整条命令（#522）：描述不截断，引号按这台电脑的终端写
  const cliText = creatorCli({ title: title.trim(), body: body.trim(), inbox: column === 'inbox', labels: labelNames, path: path ?? '', ps: isWindows });

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
          {failed.length > 0 && (
            <span className="cr-draft cr-failed" title={failed.map(d => d.title.trim() || d.body.trim()).join('\n')}>
              {failed.length === 1 ? `另存了「${(failed[0].title.trim() || failed[0].body.trim()).slice(0, 18)}」` : `另存了 ${failed.length} 份草稿`}
              <button type="button" aria-label="恢复另存的草稿" onClick={() => restoreFailed(failed[failed.length - 1])}>
                恢复
              </button>
            </span>
          )}
          <span className="spacer" />
          <Tip text="关闭（保留草稿）  esc">
            <button aria-label="关闭（保留草稿）" className="icon-btn" type="button" data-tip-align="end" onClick={close}>
              <Icon name="close" />
            </button>
          </Tip>
        </div>
        <form
          className="d-grid"
          noValidate
          onSubmit={e => {
            e.preventDefault();
            submit();
          }}
        >
          <div className="d-main cr-main">
            <textarea
              ref={titleRef}
              className="d-title"
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
                <div className="cr-prompts" ref={promptBox}>
                  <button
                    ref={promptBtn}
                    className="btn sm ghost"
                    type="button"
                    aria-haspopup="dialog"
                    aria-expanded={promptsOpen}
                    aria-controls="cr-prompt-pop"
                    title="快捷提示词：保存常用片段，点一下插入到描述"
                    onMouseDown={e => e.preventDefault()}
                    onClick={togglePrompts}
                  >
                    <Icon name="prompt" />
                    快捷提示词
                  </button>
                  {promptsOpen && (
                    <div className="cr-prompt-pop" id="cr-prompt-pop" role="dialog" aria-label="快捷提示词">
                      <div className="cr-prompt-head">
                        <b>快捷提示词</b>
                        <span className="spacer" />
                        {!promptEdit && (
                          <button className="btn sm ghost" type="button" onClick={startPromptNew}>
                            <Icon name="plus" />
                            新建
                          </button>
                        )}
                        <button className="icon-btn" type="button" aria-label="关闭快捷提示词" onClick={() => closePrompts(true)}>
                          <Icon name="close" />
                        </button>
                      </div>
                      {promptEdit ? (
                        <div
                          className="cr-prompt-form"
                          onKeyDown={e => {
                            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                              e.preventDefault();
                              savePrompt();
                            }
                          }}
                        >
                          <input
                            className="input"
                            autoFocus
                            maxLength={60}
                            aria-label="提示词标题"
                            placeholder="标题，例如：等发版"
                            value={promptEdit.title}
                            onChange={e => {
                              setPromptEdit(v => (v ? { ...v, title: e.target.value } : v));
                              setPromptError('');
                            }}
                          />
                          <textarea
                            className="textarea"
                            rows={4}
                            aria-label="提示词内容"
                            placeholder="内容，可多行；点这条时原样插入描述"
                            value={promptEdit.text}
                            onChange={e => {
                              setPromptEdit(v => (v ? { ...v, text: e.target.value } : v));
                              setPromptError('');
                            }}
                          />
                          {promptError && (
                            <div className="cr-prompt-error" role="alert">
                              {promptError}
                            </div>
                          )}
                          <div className="cr-prompt-form-act">
                            <button
                              className="btn sm"
                              type="button"
                              onClick={() => {
                                setPromptEdit(null);
                                setPromptError('');
                              }}
                            >
                              取消
                            </button>
                            <button
                              className="btn sm brand"
                              type="button"
                              disabled={!promptEdit.title.trim() || !promptEdit.text.trim()}
                              onClick={savePrompt}
                            >
                              保存
                            </button>
                          </div>
                        </div>
                      ) : prompts.length === 0 ? (
                        <div className="cr-prompt-empty">
                          <p>还没有快捷提示词。把常写的片段存下来，下次点一下就能插入。</p>
                          <button className="btn sm" type="button" onClick={startPromptNew}>
                            <Icon name="plus" />
                            新建提示词
                          </button>
                        </div>
                      ) : (
                        <div className="cr-prompt-list">
                          {prompts.map(p => (
                            <div className="cr-prompt-row" key={p.id}>
                              <button
                                className="cr-prompt-insert"
                                type="button"
                                onMouseDown={e => e.preventDefault()}
                                onClick={() => insertPrompt(p)}
                                title={`插入「${p.title}」`}
                              >
                                <b>{p.title}</b>
                                <span>{promptSummary(p.text)}</span>
                              </button>
                              {promptDelete === p.id ? (
                                <span className="cr-prompt-del">
                                  <span>删除？</span>
                                  <button className="btn sm danger" type="button" onClick={() => deletePrompt(p.id)}>
                                    删除
                                  </button>
                                  <button className="btn sm ghost" type="button" onClick={() => setPromptDelete(null)}>
                                    取消
                                  </button>
                                </span>
                              ) : (
                                <span className="cr-prompt-tools">
                                  <button
                                    className="icon-btn"
                                    type="button"
                                    aria-label={`编辑提示词「${p.title}」`}
                                    title="编辑"
                                    onClick={() => startPromptEdit(p)}
                                  >
                                    <Icon name="edit" />
                                  </button>
                                  <button
                                    className="icon-btn"
                                    type="button"
                                    aria-label={`删除提示词「${p.title}」`}
                                    title="删除"
                                    onClick={() => setPromptDelete(p.id)}
                                  >
                                    <Icon name="trash" />
                                  </button>
                                </span>
                              )}
                            </div>
                          ))}
                        </div>
                      )}
                      <div className="cr-prompt-foot">点一条插入到描述光标处</div>
                    </div>
                  )}
                </div>
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
                className="textarea"
                spellCheck={false}
                hidden={mode !== 'edit'}
                placeholder={'验收标准、相关链接、背景……\n\n例如：\n**验收标准**\n- [ ] board ls --search 能匹配描述'}
                value={body}
                onChange={e => setBody(e.target.value)}
              />
              {mode === 'preview' && (
                <div className="cr-preview">{body.trim() ? <Markdown src={body} repo={repo} links={false} /> : <p className="cr-empty">还没有描述</p>}</div>
              )}
            </div>
          </div>
          <div className="d-side cr-side">
            <div className="cr-sec">
              <div className="d-label">放到</div>
              <div role="radiogroup" aria-label="放到">
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
                <span className="cmd">board</span> new {q(title.trim() || '<标题>')}
                {short ? ` --body ${q(short)}` : ''}
                {column === 'inbox' ? ' --inbox' : ''}
                {labelNames.map(n => ` --label ${q(n)}`).join('')}{' '}
                <span className="dim">--board</span> {path}
                <CopyCli text={cliText} />
              </div>
            </div>
            {created.length > 0 && (
              <div className="cr-sec cr-recent">
                <div className="d-label">
                  本次已创建 <span className="count">{created.length}</span>
                </div>
                <div className="cr-recent-list">
                  {created.map((c, i) => (
                    <button
                      key={c.key}
                      type="button"
                      className={cx('cr-recent-row', i === 0 && 'new', c.id < 0 && 'pending')}
                      disabled={c.id < 0}
                      title={c.id < 0 ? '正在创建…' : undefined}
                      style={{ ['--c' as string]: c.column === 'inbox' ? 'var(--c-inbox)' : startColor }}
                      onClick={() => {
                        close();
                        if (c.column === 'inbox') setTab('inbox');
                        setTimeout(() => openCard(c.id), 120);
                      }}
                    >
                      <i />
                      <span className="mono">{c.id < 0 ? '#…' : `#${c.id}`}</span>
                      <span className="tt">
                        <Title text={c.title} />
                      </span>
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
                  aria-label="连续新建"
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
                <button className="btn brand cr-submit" type="submit">
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

/** 「等价的 CLI」整条命令（#522）：复制到终端就能执行。描述完整带上（界面上只显示前 40 个字），
 * macOS / Linux 用单引号（不展开 $、反引号，换行原样保留），Windows 用 PowerShell 的单引号 */
export function creatorCli(o: { title: string; body: string; inbox: boolean; labels: string[]; path: string; ps: boolean }): string {
  const q = (s: string) => (o.ps ? `'${s.replace(/'/g, "''")}'` : `'${s.replace(/'/g, `'\\''`)}'`);
  return [
    'board new',
    q(o.title || '<标题>'),
    o.body ? `--body ${q(o.body)}` : '',
    o.inbox ? '--inbox' : '',
    ...o.labels.map(n => `--label ${q(n)}`),
    `--board ${o.path}`,
  ]
    .filter(Boolean)
    .join(' ');
}

/** 「等价的 CLI」右上角的复制按钮：复制当前这条命令，成功后图标变勾并弹提示（toast 区是 aria-live） */
function CopyCli({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className={cx('cr-cli-copy', done && 'done')}
      aria-label="复制等价的 CLI 命令"
      title="复制命令"
      onClick={async () => {
        const ok = await copyText(text);
        toast({ type: ok ? 'success' : 'error', title: ok ? '已复制命令' : '没能写入剪贴板', duration: 1800 });
        if (!ok) return;
        setDone(true);
        setTimeout(() => setDone(false), 1400);
      }}
    >
      <Icon name={done ? 'check' : 'copy'} />
    </button>
  );
}
