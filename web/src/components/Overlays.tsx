/* ==========================================================================
   浮层：命令面板、确认框、表单弹窗、⋯ 菜单、标签选择器、标签管理、提示词预览、toast
   ========================================================================== */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  copyBoardEnv, copyBoardPrompt, copyQuickPrompt, deleteBoard, deleteProject, editBoard, editProject, newBoard, newProject, openBoard, openCard,
  setProjectColor,
} from '../lib/actions';
import { api } from '../lib/api';
import { moveTo, vetoFor } from '../lib/drag';
import { canNewWindow, cycleTheme, getTheme, newWindow } from '../lib/platform';
import { quickPrompts } from '../lib/prompt';
import { SORTS, sortKey, type SortSpec } from '../lib/sort';
import { boardFlow, INBOX, LABEL_COLORS } from '../lib/schema';
import type { Card, ProjectInfo } from '../lib/types';
import { accel, cx, replay } from '../lib/util';
import { allBoards, boardMeta, deleteLabelPatch, editLabelPatch, optimistic, useData, type Patch } from '../store/data';
import {
  closeCtx, closeForm, closeImage, stepImage, closeLabels, closePalette, closePicker, confirm, dismissToast, manageLabels, openCreator,
  setSide, setSort, setTab, settleConfirm, toast, toastError, updatePicker, useUI, type FormCfg, type Toast,
} from '../store/ui';
import { ColorRow, LabelChip, ProjectAvatar } from './bits';
import { Icon, type IconName } from './Icon';

/** 工作区还没取到时的项目列表：模块级常量，选择器每次返回同一个数组（新数组会让 useData 认为变了、反复重渲染） */
const NO_PROJECTS: ProjectInfo[] = [];

/** 打开时先挂载再加 .on（入场过渡）；关闭时先去掉 .on 再卸载 */
export function usePresence(open: boolean, ms = 240) {
  const [mounted, setMounted] = useState(open);
  const [on, setOn] = useState(false);
  useEffect(() => {
    if (open) {
      setMounted(true);
      const r = requestAnimationFrame(() => requestAnimationFrame(() => setOn(true)));
      return () => cancelAnimationFrame(r);
    }
    setOn(false);
    const t = setTimeout(() => setMounted(false), ms);
    return () => clearTimeout(t);
  }, [open, ms]);
  return { mounted, on };
}

/* ============================ 命令面板 ============================ */

interface PItem {
  id: string;
  group: string;
  title: string;
  sub?: string;
  hint?: string;
  icon?: IconName;
  card?: Card;
  q?: string;
  run: () => void;
}

function highlight(text: string, q: string): ReactNode {
  const parts = text.split(/(`[^`]+`)/g);
  const lower = q.toLowerCase();
  return parts.map((p, i) => {
    if (p.startsWith('`') && p.endsWith('`') && p.length > 1) return <code key={i}>{p.slice(1, -1)}</code>;
    const at = lower ? p.toLowerCase().indexOf(lower) : -1;
    if (at < 0) return <span key={i}>{p}</span>;
    return (
      <span key={i}>
        {p.slice(0, at)}
        <mark>{p.slice(at, at + q.length)}</mark>
        {p.slice(at + q.length)}
      </span>
    );
  });
}

export function Palette() {
  const { open, prefill } = useUI(s => s.palette);
  const { mounted, on } = usePresence(open, 200);
  const board = useData(s => s.board);
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const [q, setQ] = useState('');
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const prevFocus = useRef<Element | null>(null);

  useEffect(() => {
    if (!open) return;
    prevFocus.current = document.activeElement;
    setQ(prefill);
    setActive(0);
    const t = setTimeout(() => input.current?.focus(), 30);
    return () => clearTimeout(t);
  }, [open, prefill]);

  const close = () => {
    closePalette();
    (prevFocus.current as HTMLElement | null)?.focus?.({ preventScroll: true });
  };

  const items = useMemo<PItem[]>(() => {
    if (!open) return [];
    const query = q.trim();
    const out: PItem[] = [];
    const f = boardFlow(board);
    const all = board ? [INBOX, ...f.list.map(s => s.key)].flatMap(col => (board.cols[col] ?? []).map(id => board.cards[id])).filter(Boolean) : [];
    let cards: Card[];
    if (!query) cards = [...all].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 6);
    else {
      const ql = query.toLowerCase().replace(/^#/, '');
      cards = all.filter(c => String(c.id) === ql || c.title.toLowerCase().includes(ql) || c.body.toLowerCase().includes(ql)).slice(0, 12);
    }
    for (const c of cards)
      out.push({
        id: `card-${c.id}`, group: query ? '卡片' : '最近更新', title: c.title, card: c, q: query.replace(/^#/, ''),
        sub: c.holder ? `${f.name(c.column)} · ${c.holder}` : f.name(c.column),
        run: () => openCard(c.id),
      });
    const theme = { auto: '跟随系统', light: '浅色', dark: '深色' }[getTheme()];
    const acts: PItem[] = [
      { id: 'new', group: '操作', title: '新建卡片', hint: 'N', icon: 'plus', run: () => openCreator('todo') },
      { id: 'capture', group: '操作', title: '记一条未整理', hint: 'I', icon: 'inbox', run: () => openCreator('inbox') },
      { id: 'board-prompt', group: '操作', title: '复制给 agent：处理这块白板的待办', sub: `/board-skills ${path}`, icon: 'prompt', run: () => copyBoardPrompt() },
      ...quickPrompts(path, f.claimables.map(s => s.name)).map(q => ({
        id: `quick-${q.id}`, group: '提示词', title: `复制提示词：${q.title}`, sub: q.text, icon: 'prompt' as IconName, run: () => copyQuickPrompt(q),
      })),
      // 带标签的提示词：每个可领取状态 × 每个标签各一条，只在搜索时出现，免得空面板被刷屏
      ...(query
        ? (ws?.projects.find(p => p.key === path.split('/')[0])?.labels ?? []).flatMap(l =>
            quickPrompts(path, f.claimables.map(s => s.name), l.name)
              .filter(q => q.kind === 'label')
              .map(q => ({
                id: `quick-${q.id}-${l.id}`, group: '提示词', title: `复制提示词：${q.title}`, sub: q.text, icon: 'prompt' as IconName, run: () => copyQuickPrompt(q),
              })),
          )
        : []),
      { id: 'labels', group: '操作', title: '管理标签', icon: 'tag', run: () => manageLabels(path.split('/')[0]) },
      { id: 'tab-board', group: '切换到', title: '白板', hint: accel('1'), icon: 'board', run: () => setTab('board') },
      { id: 'tab-projects', group: '切换到', title: '项目总览（画布）', hint: accel('2'), icon: 'folder', run: () => setTab('projects') },
      { id: 'tab-inbox', group: '切换到', title: '未整理', hint: accel('3'), icon: 'inbox', run: () => setTab('inbox') },
      { id: 'tab-settings', group: '切换到', title: '设置', hint: accel('4'), icon: 'sun', run: () => setTab('settings') },
      { id: 'tab-hosts', group: '切换到', title: '机器（board host）', hint: accel('5'), icon: 'host', run: () => setTab('hosts') },
      { id: 'side', group: '切换到', title: '收起 / 展开侧栏', hint: accel('B'), icon: 'monitor', run: () => setSide(!useUI.getState().side) },
      ...(canNewWindow
        ? [{ id: 'new-window', group: '切换到', title: '新建窗口', hint: accel('N', true), icon: 'monitor' as IconName, run: () => newWindow() }]
        : []),
      {
        id: 'theme', group: '外观', title: `切换主题（当前：${theme}）`, icon: 'sun',
        run: () => {
          const t = cycleTheme();
          toast({ type: 'info', title: { auto: '主题：跟随系统', light: '主题：浅色', dark: '主题：深色' }[t], duration: 1400 });
        },
      },
    ];
    for (const b of allBoards(ws).filter(b => b.path !== path))
      acts.push({ id: `board-${b.path}`, group: '切换白板', title: `${b.project.name} / ${b.name}`, sub: b.path, icon: 'board', run: () => openBoard(b.path) });
    acts.push({ id: 'board-new', group: '切换白板', title: '新建白板…', icon: 'plus', run: () => newBoard() });
    acts.push({ id: 'proj-new', group: '切换白板', title: '新建项目…', icon: 'folder', run: () => newProject() });
    out.push(...acts.filter(a => !query || a.title.toLowerCase().includes(query.toLowerCase()) || a.sub?.includes(query)));
    return out;
  }, [open, q, board, ws, path]);

  const act = Math.min(active, Math.max(0, items.length - 1));
  useEffect(() => {
    list.current?.querySelector('.p-item.active')?.scrollIntoView({ block: 'nearest' });
  }, [act]);

  const run = (i: number) => {
    const it = items[i];
    if (!it) return;
    close();
    setTimeout(() => it.run(), 60);
  };

  if (!mounted) return null;
  let group: string | null = null;
  return (
    <div className={cx('palette-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="palette" role="dialog" aria-label="命令面板">
        <div className="palette-search">
          <Icon name="search" />
          <input
            ref={input}
            value={q}
            placeholder="搜索卡片（标题、描述、#ID）或输入命令…"
            spellCheck={false}
            onChange={e => {
              setQ(e.target.value);
              setActive(0);
            }}
            onKeyDown={e => {
              if (e.key === 'ArrowDown') {
                e.preventDefault();
                setActive((act + 1) % Math.max(1, items.length));
              } else if (e.key === 'ArrowUp') {
                e.preventDefault();
                setActive((act - 1 + items.length) % Math.max(1, items.length));
              } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                e.preventDefault();
                run(act);
              } else if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                close();
              }
            }}
          />
          <kbd>esc</kbd>
        </div>
        <div className="palette-list" role="listbox" ref={list}>
          {items.length === 0 && <div className="p-empty">没有匹配的结果</div>}
          {items.map((it, i) => {
            const head = it.group !== group ? <div className="p-group">{it.group}</div> : null;
            group = it.group;
            return (
              <div key={it.id}>
                {head}
                <button
                  type="button"
                  className={cx('p-item', i === act && 'active')}
                  role="option"
                  aria-selected={i === act}
                  onClick={() => run(i)}
                  onPointerMove={() => i !== act && setActive(i)}
                >
                  {it.card ? (
                    <span className="p-ic col" style={{ ['--c' as string]: `var(--c-${it.card.column})` }}>
                      {it.card.column.slice(0, 2).toUpperCase()}
                    </span>
                  ) : (
                    <span className="p-ic">
                      <Icon name={it.icon || 'sparkle'} />
                    </span>
                  )}
                  {it.card && <span className="p-id">#{it.card.id}</span>}
                  <span className="p-main">{highlight(it.title, it.q ?? '')}</span>
                  {it.sub && <span className="p-sub">{it.sub}</span>}
                  {it.hint && <kbd>{it.hint}</kbd>}
                </button>
              </div>
            );
          })}
        </div>
        <div className="palette-foot">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> 选择
          </span>
          <span>
            <kbd>↵</kbd> 打开
          </span>
          <span>
            <kbd>esc</kbd> 关闭
          </span>
        </div>
      </div>
    </div>
  );
}

/* ============================ 确认框 ============================ */

export function ConfirmDialog() {
  const req = useUI(s => s.confirm);
  const { mounted, on } = usePresence(!!req, 220);
  const last = useRef(req);
  if (req) last.current = req;
  const r = last.current;
  const cancel = useRef<HTMLButtonElement>(null);
  const ok = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!req) return;
    const prev = document.activeElement as HTMLElement | null;
    const t = setTimeout(() => cancel.current?.focus(), 40);
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        e.preventDefault();
        settleConfirm(false);
      }
      if (e.key === 'Tab') {
        e.preventDefault();
        (document.activeElement === ok.current ? cancel.current : ok.current)?.focus();
      }
    };
    document.addEventListener('keydown', key, true);
    return () => {
      clearTimeout(t);
      document.removeEventListener('keydown', key, true);
      prev?.focus?.({ preventScroll: true });
    };
  }, [req]);
  if (!mounted || !r) return null;
  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && settleConfirm(false)}>
      <div className="dialog" role="alertdialog" aria-modal="true">
        <div className="dialog-icon">
          <Icon name={r.icon === 'tag' ? 'tag' : r.icon === 'reset' ? 'renew' : r.icon === 'logout' ? 'logout' : r.icon === 'key' ? 'key' : r.icon === 'unlock' ? 'unlock' : 'trash'} />
        </div>
        <h3>{r.title}</h3>
        <p>{r.text}</p>
        <div className="dialog-actions">
          <button className="btn" type="button" ref={cancel} onClick={() => settleConfirm(false)}>
            取消
          </button>
          <button className="btn danger-solid" type="button" ref={ok} onClick={() => settleConfirm(true)}>
            {r.ok || '确定'}
          </button>
        </div>
      </div>
    </div>
  );
}

/* ============================ 表单弹窗：项目 / 白板 ============================ */

export function FormModal() {
  const cfg = useUI(s => s.form);
  const { mounted, on } = usePresence(!!cfg, 220);
  const last = useRef(cfg);
  if (cfg) last.current = cfg;
  if (!mounted || !last.current) return null;
  return <FormBody key={last.current.title + (cfg ? '1' : '0')} cfg={last.current} on={on && !!cfg} />;
}

function FormBody({ cfg, on }: { cfg: FormCfg; on: boolean }) {
  const [v, setV] = useState<Record<string, string>>(() => Object.fromEntries(cfg.fields.map(f => [f.name, f.value ?? ''])));
  const [touched, setTouched] = useState<Record<string, boolean>>({});
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const form = useRef<HTMLFormElement>(null);

  useEffect(() => {
    const t = setTimeout(() => form.current?.querySelector<HTMLElement>('input:not([readonly]), select')?.focus(), 40);
    return () => clearTimeout(t);
  }, []);

  const update = (name: string, value: string) => {
    setV(prev => {
      const next = { ...prev, [name]: value };
      // 标识自动跟随名称，直到用户手动改过
      for (const f of cfg.fields)
        if (f.type === 'key' && f.from && !f.readonly && !touched[f.name] && f.name !== name)
          next[f.name] = next[f.from]?.trim() ? (f.auto?.(next[f.from], next) ?? '') : '';
      return next;
    });
    setErr('');
  };
  const live = cfg.fields.map(f => (f.validate && v[f.name] ? f.validate(v[f.name], v) : null)).find(Boolean) || '';

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const first = cfg.fields.find(f => f.required && !String(v[f.name] ?? '').trim());
    if (first) {
      const el = form.current?.elements.namedItem(first.name) as HTMLElement | null;
      replay(el, 'invalid');
      el?.focus();
      return;
    }
    if (live) return setErr(live);
    setBusy(true);
    try {
      await cfg.onSubmit(v);
      closeForm();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const color = v.color;
  const pv = cfg.previewData ?? {};
  const projects = useData(s => s.ws?.projects ?? NO_PROJECTS);
  let preview: ReactNode = null;
  if (cfg.preview === 'project' || cfg.preview === 'edit-project') {
    preview = (
      <>
        <ProjectAvatar name={v.name || '新'} color={color} />
        <div>
          <b>{v.name || '新项目'}</b>
          <span className="mono">
            {cfg.preview === 'project' ? `export BOARD=${v.key || '…'}/${v.boardKey || 'main'}` : `${pv.key} · ${pv.boards} 块白板`}
          </span>
        </div>
      </>
    );
  } else if (cfg.preview === 'board' || cfg.preview === 'edit-board') {
    const p = projects.find(x => x.key === (v.project || pv.path?.split('/')[0]));
    preview = (
      <>
        <span className="pj-bic lg" style={{ ['--pc' as string]: p?.color ?? pv.color }}>
          <Icon name="board" />
        </span>
        <div>
          <b>
            {p?.name ?? pv.projectName} / {v.name || '新白板'}
          </b>
          <span className="mono">{cfg.preview === 'board' ? `export BOARD=${v.project}/${v.key || '…'}` : `BOARD=${pv.path}`}</span>
        </div>
      </>
    );
  }

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && closeForm()}>
      <form
        className="dialog pj-modal"
        ref={form}
        role="dialog"
        aria-modal="true"
        noValidate
        onSubmit={submit}
        onKeyDown={e => {
          if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            closeForm();
          }
        }}
      >
        <h3>{cfg.title}</h3>
        {cfg.subtitle && <p>{cfg.subtitle}</p>}
        {preview && <div className="pj-preview">{preview}</div>}
        {cfg.fields.map(f => {
          const hint = f.hint ? <em>{f.hint}</em> : null;
          if (f.type === 'color')
            return (
              <div className="field" key={f.name}>
                <span>
                  {f.label}
                  {hint}
                </span>
                <ColorRow value={v[f.name] ?? ''} onPick={c => update(f.name, c)} label={f.label} />
              </div>
            );
          if (f.type === 'select')
            return (
              <label className="field" key={f.name}>
                <span>
                  {f.label}
                  {hint}
                </span>
                <select className="input" name={f.name} value={v[f.name]} onChange={e => update(f.name, e.target.value)}>
                  {f.options?.map(o => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </label>
            );
          return (
            <label className="field" key={f.name}>
              <span>
                {f.label}
                {hint}
              </span>
              <input
                className={cx('input', f.type === 'key' && 'mono')}
                name={f.name}
                value={v[f.name] ?? ''}
                placeholder={f.placeholder}
                maxLength={f.type === 'key' ? 32 : 60}
                autoComplete="off"
                spellCheck={false}
                readOnly={f.readonly}
                onChange={e => {
                  let val = e.target.value;
                  if (f.type === 'key') {
                    val = val.toLowerCase().replace(/[^a-z0-9-]/g, '');
                    setTouched(t => ({ ...t, [f.name]: true }));
                  }
                  update(f.name, val);
                }}
              />
            </label>
          );
        })}
        {(err || live) && <div className="field-err">{err || live}</div>}
        <div className="dialog-actions">
          <button className="btn" type="button" onClick={closeForm}>
            取消
          </button>
          <button className={cx('btn brand', busy && 'is-busy')} type="submit" disabled={busy} aria-busy={busy || undefined}>
            {cfg.submit || '保存'}
          </button>
        </div>
      </form>
    </div>
  );
}

/* ============================ ⋯ 菜单：项目 / 白板 ============================ */

export function CtxMenu() {
  const req = useUI(s => s.ctx);
  const ws = useData(s => s.ws);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!req || !ref.current) return setPos(null);
    const m = ref.current;
    const w = m.offsetWidth;
    const h = m.offsetHeight;
    const r = req.anchor ?? ({ left: req.x, right: req.x, top: req.y, bottom: req.y } as DOMRect);
    setPos({
      left: Math.max(8, Math.min(req.anchor ? r.right - w : r.left, innerWidth - w - 8)),
      top: r.bottom + h + 8 > innerHeight ? Math.max(8, r.top - h - 6) : r.bottom + 6,
    });
    requestAnimationFrame(() => m.querySelector<HTMLElement>('button[data-do]:not([disabled])')?.focus({ preventScroll: true }));
  }, [req]);

  useEffect(() => {
    if (!req) return;
    const down = (e: PointerEvent) => !(e.target as HTMLElement).closest('.ctx, .prompts-btn, .col-sort-btn, .d-move') && closeCtx();
    document.addEventListener('pointerdown', down);
    window.addEventListener('resize', closeCtx);
    return () => {
      document.removeEventListener('pointerdown', down);
      window.removeEventListener('resize', closeCtx);
    };
  }, [req]);

  if (!req) return null;
  const go = (fn: () => void) => () => {
    closeCtx();
    fn();
  };
  let body: ReactNode = null;
  if (req.kind === 'project') {
    const p = ws?.projects.find(x => x.key === req.id);
    if (!p) return null;
    body = (
      <>
        <div className="ctx-title">
          <ProjectAvatar name={p.name} color={p.color} className="sm" />
          <b>{p.name}</b>
        </div>
        <button type="button" data-do="new-board" onClick={go(() => newBoard(p.key))}>
          <Icon name="plus" />
          新建白板
        </button>
        <button type="button" data-do="edit-project" onClick={go(() => editProject(p.key))}>
          <Icon name="edit" />
          编辑项目
        </button>
        <button type="button" data-do="labels" onClick={go(() => manageLabels(p.key))}>
          <Icon name="tag" />
          管理标签
        </button>
        <ColorRow className="ctx-swatches" value={p.color} onPick={c => setProjectColor(p.key, c)} />
        <div className="ctx-sep" />
        <button type="button" data-do="delete-project" className="danger" onClick={go(() => deleteProject(p.key))}>
          <Icon name="trash" />
          删除项目
        </button>
      </>
    );
  } else if (req.kind === 'prompts') {
    body = <PromptsMenu path={req.id} go={go} />;
  } else if (req.kind === 'sort') {
    body = <SortMenu col={req.id} go={go} />;
  } else if (req.kind === 'move') {
    body = <MoveMenu id={Number(req.id)} go={go} />;
  } else {
    const m = boardMeta(ws, req.id);
    if (!m) return null;
    const single = allBoards(ws).length <= 1;
    body = (
      <>
        <div className="ctx-title">
          <span className="pj-bic" style={{ ['--pc' as string]: m.project.color }}>
            <Icon name="board" />
          </span>
          <b>{m.board.name}</b>
        </div>
        <button type="button" data-do="open" onClick={go(() => openBoard(req.id))}>
          <Icon name="move" />
          打开白板
        </button>
        <button type="button" data-do="edit-board" onClick={go(() => editBoard(req.id))}>
          <Icon name="edit" />
          编辑白板
        </button>
        <button type="button" data-do="copy-board-prompt" onClick={go(() => copyBoardPrompt(req.id))}>
          <Icon name="prompt" />
          复制为提示词
        </button>
        <button type="button" data-do="copy-path" onClick={go(() => copyBoardEnv(req.id))}>
          <Icon name="copy" />
          复制 <code>BOARD={req.id}</code>
        </button>
        <div className="ctx-sep" />
        <button type="button" data-do="delete-board" className="danger" disabled={single} onClick={go(() => deleteBoard(req.id))}>
          <Icon name="trash" />
          删除白板
        </button>
      </>
    );
  }
  return (
    <div
      className={cx('ctx', req.kind === 'prompts' && 'ctx-prompts', req.kind === 'sort' && 'ctx-sort', req.kind === 'move' && 'ctx-move', pos && 'on')}
      ref={ref}
      role="menu"
      style={pos ?? { left: -9999, top: -9999 }}
      onKeyDown={e => {
        const bs = [...(ref.current?.querySelectorAll<HTMLElement>('[data-do]:not([disabled])') ?? [])];
        const i = bs.indexOf(document.activeElement as HTMLElement);
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          bs[(i + 1) % bs.length]?.focus();
        }
        if (e.key === 'ArrowUp') {
          e.preventDefault();
          bs[(i - 1 + bs.length) % bs.length]?.focus();
        }
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          closeCtx();
        }
      }}
    >
      {body}
    </div>
  );
}

/* ============================ 卡片详情「移动到」 ============================ */
/* 列出当前状态以外的工作流状态；不允许的（被 agent 持有、锁定状态）标出原因，点了给出与拖拽相同的提示（lib/drag.ts moveTo） */

function MoveMenu({ id, go }: { id: number; go: (fn: () => void) => () => void }) {
  const board = useData(s => s.board);
  const card = board?.cards[id];
  if (!board || !card) return null;
  const f = boardFlow(board);
  const targets = f.list.filter(s => s.key !== card.column);
  return (
    <>
      <div className="ctx-title">
        <Icon name="move" />
        <b>把 #{card.id} 移动到</b>
      </div>
      {card.holder && <p className="sort-hint">{card.holder} 正持有这张卡片，只有持有者能移动</p>}
      {targets.map(s => {
        const veto = vetoFor(card, s.key, f);
        return (
          <button
            key={s.key}
            type="button"
            data-do={`move-${s.key}`}
            className={cx('mv-item', veto && 'vetoed')}
            aria-disabled={!!veto}
            title={veto?.message}
            onClick={go(() => moveTo(card.id, s.key))}
          >
            <i className="mv-dot" style={{ background: f.color(s.key) }} />
            <span className="mv-name">{s.name}</span>
            {veto ? (
              <span className="mv-note">
                <Icon name="lock" />
                {/* 被持有时标题下已经说过一次，行里只留锁 */}
                {veto.code !== 'NOT_HOLDER' && veto.title}
              </span>
            ) : (
              <span className="mv-count mono">{board.cols[s.key]?.length ?? 0}</span>
            )}
          </button>
        );
      })}
      {!targets.length && <p className="sort-hint">工作流里没有别的状态</p>}
    </>
  );
}

/* ============================ 状态块排序菜单 ============================ */
/* 一次性排序：只改这块的显示顺序，不写服务端、不改优先级（lib/sort.ts） */

function SortMenu({ col, go }: { col: string; go: (fn: () => void) => () => void }) {
  const path = useData(s => s.path);
  const board = useData(s => s.board);
  const key = sortKey(path, col);
  const cur = useUI(s => s.sorts[key]);
  const pick = (spec: SortSpec | null) => go(() => setSort(key, spec));
  return (
    <>
      <div className="ctx-title">
        <Icon name="sort" />
        <b>排序「{boardFlow(board).name(col)}」</b>
      </div>
      <p className="sort-hint">只改这块的显示顺序，优先级和 agent 的领取顺序不变</p>
      <button type="button" data-do="sort-none" className={cx(!cur && 'on')} onClick={pick(null)}>
        <span className="sort-name">优先级</span>
        {!cur && <Icon name="check" />}
      </button>
      {SORTS.map(o => {
        const on = cur?.by === o.by;
        return (
          <div key={o.by} className={cx('sort-row', on && 'on')}>
            <button type="button" data-do={`sort-${o.by}`} onClick={pick({ by: o.by, desc: on ? cur.desc : o.desc })}>
              <span className="sort-name">{o.name}</span>
              {!o.dir && <span className="sort-note">按项目标签顺序分组</span>}
              {on && !o.dir && <Icon name="check" />}
            </button>
            {o.dir && (
              <span className="sort-dir" role="group" aria-label={`${o.name}的方向`}>
                {[false, true].map(desc => (
                  <button
                    key={String(desc)}
                    type="button"
                    data-do={`sort-${o.by}-${desc ? 'desc' : 'asc'}`}
                    className={cx(on && cur.desc === desc && 'on')}
                    aria-pressed={on && cur.desc === desc}
                    onClick={pick({ by: o.by, desc })}
                  >
                    {desc ? '降序' : '升序'}
                  </button>
                ))}
              </span>
            )}
          </div>
        );
      })}
    </>
  );
}

/* ============================ 提示词菜单 ============================ */
/* 白板页头「提示词」：每个可领取状态一条，点一下复制；默认单行摘要；带标签的提示词也按可领取状态各来一组 */

function PromptsMenu({ path, go }: { path: string; go: (fn: () => void) => () => void }) {
  const ws = useData(s => s.ws);
  const board = useData(s => s.board);
  const flow = boardFlow(board);
  // 可领取状态按工作流顺序；第一个也是「拆解 / 上传」的领取起点
  const claimables = flow.claimables.length ? flow.claimables.map(s => s.name) : [flow.home?.name ?? '待办'];
  const labels = ws?.projects.find(p => p.key === path.split('/')[0])?.labels ?? [];
  const list = quickPrompts(path, claimables);
  return (
    <>
      <div className="ctx-title">
        <Icon name="prompt" />
        <b>复制给 agent 的提示词</b>
      </div>
      <div className="qp-list">
        {list.map(q => (
          <button key={q.id} type="button" data-do={`quick-${q.id}`} className="qp-item" title={q.text} onClick={go(() => copyQuickPrompt(q))}>
            <span className="qp-title">{q.title}</span>
            <code className="qp-text">{q.text}</code>
          </button>
        ))}
      </div>
      {labels.length ? (
        claimables.map(name => (
          <div key={name} className="qp-item qp-labels">
            <span className="qp-title">做「{name}」里带指定标签的任务</span>
            <div className="qp-chips">
              {labels.map(l => {
                const q = quickPrompts(path, claimables, l.name).find(x => x.kind === 'label' && x.status === name)!;
                return (
                  <button key={l.id} type="button" data-do={`quick-label-${name}-${l.id}`} data-tip={q.text} onClick={go(() => copyQuickPrompt(q))}>
                    <LabelChip label={l} />
                  </button>
                );
              })}
            </div>
          </div>
        ))
      ) : (
        <div className="qp-item qp-labels">
          <span className="qp-empty">项目里还没有标签</span>
        </div>
      )}
    </>
  );
}

/* ============================ 标签选择器 ============================ */

export function LabelPicker() {
  const req = useUI(s => s.picker);
  const ws = useData(s => s.ws);
  const ref = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [q, setQ] = useState('');
  const [active, setActive] = useState(0);
  const [pos, setPos] = useState({ left: 0, top: 0 });
  const [on, setOn] = useState(false);
  const labels = ws?.projects.find(p => p.key === req?.project)?.labels ?? [];

  useLayoutEffect(() => {
    if (!req) {
      setOn(false);
      return;
    }
    setQ('');
    setActive(0);
    const r = req.rect;
    const w = 260;
    const h = ref.current?.offsetHeight || 300;
    setPos({ left: Math.max(8, Math.min(r.left, innerWidth - w - 8)), top: r.bottom + h + 8 > innerHeight ? Math.max(8, r.top - h - 6) : r.bottom + 6 });
    setOn(true);
    const t = setTimeout(() => input.current?.focus(), 20);
    return () => clearTimeout(t);
  }, [req?.rect]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!req) return;
    const down = (e: PointerEvent) => !(e.target as HTMLElement).closest('.lb-picker, [data-label-picker]') && closePicker();
    document.addEventListener('pointerdown', down);
    return () => document.removeEventListener('pointerdown', down);
  }, [req]);

  const query = q.trim();
  const list = labels.filter(l => !query || l.name.toLowerCase().includes(query.toLowerCase()));
  const exact = labels.some(l => l.name.toLowerCase() === query.toLowerCase());
  const canCreate = !!query && !exact;
  const total = list.length + (canCreate ? 1 : 0);
  const sel = new Set(req?.selected ?? []);

  const toggle = async (id: number) => {
    if (!req) return;
    const on = !sel.has(id);
    const next = on ? [...sel, id] : [...sel].filter(x => x !== id);
    updatePicker(next);
    await req.onToggle(id, on);
    input.current?.focus();
  };
  // 新建标签要等服务端分配 ID：这一行显示进行中，不会重复新建；建好后当即勾上
  const [creating, setCreating] = useState(false);
  const create = async () => {
    if (!req || creating) return;
    setCreating(true);
    try {
      const l = await optimistic(null, () => api.createLabel(req.project, query));
      setQ('');
      updatePicker([...sel, l.id]);
      await req.onToggle(l.id, true);
    } catch (e) {
      toastError(e);
    } finally {
      setCreating(false);
    }
    input.current?.focus();
  };

  return (
    <div className={cx('lb-picker', req && on && 'on')} ref={ref} role="dialog" aria-label="选择标签" style={{ left: pos.left, top: pos.top }}>
      <div className="lbp-search">
        <Icon name="tag" />
        <input
          ref={input}
          value={q}
          placeholder="搜索或新建标签…"
          spellCheck={false}
          autoComplete="off"
          onChange={e => {
            setQ(e.target.value);
            setActive(0);
          }}
          onKeyDown={e => {
            if (e.key === 'ArrowDown') {
              e.preventDefault();
              setActive(Math.min(total - 1, active + 1));
            } else if (e.key === 'ArrowUp') {
              e.preventDefault();
              setActive(Math.max(0, active - 1));
            } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
              e.preventDefault();
              if (active < list.length) toggle(list[active].id);
              else if (canCreate) create();
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              closePicker();
            }
          }}
        />
      </div>
      <div className="lbp-list" role="listbox">
        {list.map((l, i) => (
          <button
            key={l.id}
            type="button"
            className={cx('lbp-row', i === active && 'active')}
            role="option"
            aria-selected={sel.has(l.id)}
            onClick={() => toggle(l.id)}
          >
            <span className={cx('lbp-check', sel.has(l.id) && 'on')}>
              <Icon name="check" />
            </span>
            <LabelChip label={l} />
          </button>
        ))}
        {canCreate && (
          <button type="button" className={cx('lbp-row create', active === list.length && 'active', creating && 'busy')} onClick={create} disabled={creating} aria-busy={creating || undefined}>
            <Icon name="plus" />
            {creating ? `正在新建「${query}」…` : `新建标签「${query}」`}
          </button>
        )}
        {!total && <div className="lbp-empty">还没有标签，输入名称即可新建</div>}
      </div>
      <div className="lbp-foot">
        <button type="button" onClick={() => req && manageLabels(req.project)}>
          <Icon name="edit" />
          管理标签
        </button>
      </div>
    </div>
  );
}

/* ============================ 标签管理 ============================ */

/** 标签类型开关：agent 可打 ⇄ 仅人可打 */
function KindToggle({ human, onChange }: { human: boolean; onChange: (human: boolean) => void }) {
  return (
    <button
      type="button"
      className={cx('lbm-kind', human && 'on')}
      aria-pressed={human}
      title={human ? '仅人可打：只有人能在界面上加、去；点一下改成 agent 可打' : 'agent 可打：人和 agent 都能打；点一下改成仅人可打'}
      onClick={() => onChange(!human)}
    >
      <Icon name={human ? 'lock' : 'unlock'} />
      {human ? '仅人可打' : 'agent 可打'}
    </button>
  );
}

export function LabelManager() {
  const pk = useUI(s => s.labelsOf);
  const { mounted, on } = usePresence(!!pk, 220);
  const last = useRef(pk);
  if (pk) last.current = pk;
  if (!mounted || !last.current) return null;
  return <LabelManagerBody pk={last.current} on={on && !!pk} />;
}

function LabelManagerBody({ pk, on }: { pk: string; on: boolean }) {
  const p = useData(s => s.ws?.projects.find(x => x.key === pk));
  const [name, setName] = useState('');
  const [swatch, setSwatch] = useState<number | 'new' | null>(null);
  const [newColor, setNewColor] = useState<string | null>(null);
  const [newHuman, setNewHuman] = useState(false);
  const [names, setNames] = useState<Record<number, string>>({});
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const labels = p?.labels ?? [];
  const nc = newColor ?? LABEL_COLORS[labels.length % LABEL_COLORS.length];

  useEffect(() => {
    const t = setTimeout(() => input.current?.focus(), 40);
    return () => clearTimeout(t);
  }, []);
  useEffect(() => {
    if (!p) closeLabels();
  }, [p]);

  /** 改名、改色、删除当帧生效（补丁），被拒绝时改回来并提示；新建要等服务端分配 ID，按钮显示进行中 */
  const [creating, setCreating] = useState(false);
  const run = async (patch: Patch | null, fn: () => Promise<unknown>) => {
    try {
      await optimistic(patch, fn);
    } catch (e) {
      toastError(e);
      return false;
    }
    return true;
  };

  return (
    <div
      className={cx('dialog-wrap', on && 'on')}
      onClick={e => {
        if (e.target === e.currentTarget) closeLabels();
        else if (!(e.target as HTMLElement).closest('.lbm-swatches, .lbm-color')) setSwatch(null);
      }}
      onKeyDown={e => {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          if (swatch != null) setSwatch(null);
          else closeLabels();
        }
      }}
    >
      <div className="dialog lbm" role="dialog" aria-modal="true" aria-label="管理标签">
        <div className="lbm-head">
          <h3>管理标签</h3>
          <span className="lbm-sub">
            {p?.name} · {labels.length} 个标签
          </span>
          <button type="button" className="icon-btn" aria-label="关闭" onClick={closeLabels}>
            <Icon name="close" />
          </button>
        </div>
        <p className="lbm-p">
          标签属于项目，项目下的所有白板共用。改名或改色会立即生效；删除会从所有卡片上移除。agent 用 <code>board label &lt;ID&gt; +名称</code> 打标签；
          <b>仅人可打</b>的标签带小锁，只有人能在界面上加、去，agent 打会被拒绝。
        </p>
        <div className="lbm-list" ref={list}>
          {labels.length === 0 && <div className="lbp-empty">这个项目还没有标签</div>}
          {labels.map(l => (
            <div className="lbm-row" key={l.id} style={{ ['--lc' as string]: l.color }}>
              <button type="button" className="lbm-color" aria-label="改颜色" onClick={() => setSwatch(swatch === l.id ? null : l.id)} />
              <input
                className="lbm-name"
                value={names[l.id] ?? l.name}
                maxLength={24}
                spellCheck={false}
                aria-label="标签名称"
                onChange={e => setNames(n => ({ ...n, [l.id]: e.target.value }))}
                onKeyDown={e => {
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    (e.target as HTMLInputElement).blur();
                  }
                }}
                onBlur={async () => {
                  const v = names[l.id];
                  if (v == null || v === l.name) return;
                  await run(editLabelPatch(pk, l.id, { name: v.trim() }), () => api.updateLabel(pk, l.id, { name: v }));
                  setNames(n => {
                    const x = { ...n };
                    delete x[l.id];
                    return x;
                  });
                }}
              />
              <span className="lbm-preview">
                <LabelChip label={{ ...l, name: names[l.id] ?? l.name }} />
              </span>
              <KindToggle
                human={!!l.humanOnly}
                onChange={h => run(editLabelPatch(pk, l.id, { humanOnly: h }), () => api.updateLabel(pk, l.id, { humanOnly: h }))}
              />
              <span className="lbm-count">{l.uses} 张卡片</span>
              <button
                type="button"
                className="icon-btn sm lbm-del"
                aria-label="删除"
                onClick={async () => {
                  if (l.uses && !(await confirm({ title: `删除标签「${l.name}」？`, text: `${l.uses} 张卡片上的这个标签会一起移除。`, ok: '删除标签', icon: 'tag' })))
                    return;
                  await run(deleteLabelPatch(pk, l.id), () => api.deleteLabel(pk, l.id));
                }}
              >
                <Icon name="trash" />
              </button>
              {swatch === l.id && (
                <div className="lbm-swatches">
                  {LABEL_COLORS.map(c => (
                    <button
                      key={c}
                      type="button"
                      className="swatch"
                      style={{ ['--sw' as string]: c }}
                      aria-label={c}
                      onClick={async () => {
                        setSwatch(null);
                        await run(editLabelPatch(pk, l.id, { color: c }), () => api.updateLabel(pk, l.id, { color: c }));
                      }}
                    />
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
        <form
          className="lbm-new"
          noValidate
          onSubmit={async e => {
            e.preventDefault();
            if (!name.trim()) {
              replay(input.current, 'invalid');
              return;
            }
            if (creating) return;
            setCreating(true);
            const ok = await run(null, () => api.createLabel(pk, name, nc, newHuman));
            setCreating(false);
            if (ok) {
              setName('');
              setNewColor(null);
              setNewHuman(false);
              requestAnimationFrame(() => {
                const el = list.current;
                if (!el) return;
                el.scrollTop = el.scrollHeight;
                el.lastElementChild?.classList.add('new');
              });
            }
            input.current?.focus();
          }}
        >
          <button
            type="button"
            className="lbm-color"
            aria-label="选择颜色"
            style={{ ['--lc' as string]: nc }}
            onClick={() => setSwatch(swatch === 'new' ? null : 'new')}
          />
          <input ref={input} className="input" placeholder="新标签名称" maxLength={24} autoComplete="off" spellCheck={false} value={name} onChange={e => setName(e.target.value)} />
          <KindToggle human={newHuman} onChange={setNewHuman} />
          <button type="submit" className={cx('btn sm primary', creating && 'is-busy')} disabled={creating} aria-busy={creating || undefined}>
            <Icon name="plus" />
            新建
          </button>
          {swatch === 'new' && (
            <div className="lbm-swatches">
              {LABEL_COLORS.map(c => (
                <button
                  key={c}
                  type="button"
                  className="swatch"
                  style={{ ['--sw' as string]: c }}
                  aria-label={c}
                  onClick={() => {
                    setNewColor(c);
                    setSwatch(null);
                  }}
                />
              ))}
            </div>
          )}
        </form>
      </div>
    </div>
  );
}

/* ============================ Toast ============================ */

export function Toasts() {
  const toasts = useUI(s => s.toasts);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map(t => (
        <ToastItem key={t.id} t={t} />
      ))}
    </div>
  );
}

function ToastItem({ t }: { t: Toast }) {
  const [out, setOut] = useState(false);
  const close = () => setOut(true);
  const icon: IconName = t.type === 'error' ? 'alert' : t.type === 'success' ? 'checkCircle' : 'info';
  return (
    <div className={cx('toast', t.type, out && 'out')} role={t.type === 'error' ? 'alert' : 'status'} onAnimationEnd={e => out && e.target === e.currentTarget && dismissToast(t.id)}>
      <div className="toast-ic">
        <Icon name={icon} />
      </div>
      <div className="toast-body">
        <div className="toast-title">
          {t.title}
          {t.code && <span className="code-chip">{t.code}</span>}
        </div>
        {t.text && <div className="toast-text">{t.text}</div>}
        {t.action && (
          <div className="toast-action">
            <button
              className="btn sm"
              type="button"
              onClick={() => {
                t.action!.fn();
                close();
              }}
            >
              {t.action.label}
            </button>
          </div>
        )}
      </div>
      <button className="toast-close" type="button" aria-label="关闭" onClick={close}>
        <Icon name="close" />
      </button>
      <div className="toast-bar" style={{ animationDuration: `${t.duration}ms` }} onAnimationEnd={close} />
    </div>
  );
}


/* ---------------- 放大查看图片（附件） ---------------- */

export function Lightbox() {
  const cur = useUI(s => s.image);
  const open = !!cur;
  useEffect(() => {
    if (!open) return;
    // 捕获阶段处理：Esc 只关图片，不关卡片详情；← → 切换图片，不交给详情里的快捷键
    const key = (e: KeyboardEvent) => {
      const d = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : 0;
      if (e.key !== 'Escape' && !d) return;
      e.preventDefault();
      e.stopPropagation();
      if (d) stepImage(d);
      else closeImage();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [open]);
  if (!cur) return null;
  const img = cur.list[cur.index];
  const n = cur.list.length;
  return (
    <div className="lightbox" role="dialog" aria-label={img.alt || '图片'} onClick={closeImage}>
      <img key={img.src} src={img.src} alt={img.alt} />
      <div className="lb-bar" onClick={e => e.stopPropagation()}>
        {n > 1 && (
          <div className="lb-nav">
            <button className="icon-btn sm" type="button" aria-label="上一张" title="上一张（←）" disabled={cur.index === 0} onClick={() => stepImage(-1)}>
              <Icon name="chevLeft" />
            </button>
            <span className="lb-pos" aria-live="polite">
              {cur.index + 1} / {n}
            </span>
            <button className="icon-btn sm" type="button" aria-label="下一张" title="下一张（→）" disabled={cur.index === n - 1} onClick={() => stepImage(1)}>
              <Icon name="chevRight" />
            </button>
          </div>
        )}
        <span className="lb-name">{img.alt}</span>
        <a className="btn sm" href={img.src} download={img.alt || true}>
          下载
        </a>
        <button className="btn sm" type="button" onClick={closeImage}>
          关闭
        </button>
      </div>
    </div>
  );
}
