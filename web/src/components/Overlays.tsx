/* ==========================================================================
   浮层：命令面板、确认框、表单弹窗、⋯ 菜单、标签选择器、标签管理、提示词预览、toast
   ========================================================================== */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  copyBoardEnv, copyBoardPrompt, deleteBoard, deleteProject, editBoard, editProject, newBoard, newProject, openBoard, openCard, setProjectColor,
} from '../lib/actions';
import { api } from '../lib/api';
import { cycleTheme, getTheme } from '../lib/platform';
import { boardFlow, INBOX, LABEL_COLORS, PROJECT_COLORS } from '../lib/schema';
import type { Card } from '../lib/types';
import { cx, MOD, replay } from '../lib/util';
import { allBoards, boardMeta, refresh, useData } from '../store/data';
import {
  closeCtx, closeForm, closeLabels, closePalette, closePicker, confirm, dismissToast, manageLabels, openCreator,
  setSide, setTab, settleConfirm, toast, toastError, updatePicker, useUI, type FormCfg, type Toast,
} from '../store/ui';
import { LabelChip, ProjectAvatar } from './bits';
import { Icon, type IconName } from './Icon';

/** 打开时先挂载再加 .on（入场过渡）；关闭时先去掉 .on 再卸载 */
function usePresence(open: boolean, ms = 240) {
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
      { id: 'labels', group: '操作', title: '管理标签', icon: 'tag', run: () => manageLabels(path.split('/')[0]) },
      { id: 'tab-board', group: '切换到', title: '白板', hint: `${MOD}1`, icon: 'board', run: () => setTab('board') },
      { id: 'tab-projects', group: '切换到', title: '项目总览（画布）', hint: `${MOD}2`, icon: 'folder', run: () => setTab('projects') },
      { id: 'tab-inbox', group: '切换到', title: '未整理', hint: `${MOD}3`, icon: 'inbox', run: () => setTab('inbox') },
      { id: 'tab-settings', group: '切换到', title: '设置', hint: `${MOD}4`, icon: 'sun', run: () => setTab('settings') },
      { id: 'side', group: '切换到', title: '收起 / 展开侧栏', hint: `${MOD}B`, icon: 'monitor', run: () => setSide(!useUI.getState().side) },
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
        <div className="palette-list scroll" role="listbox" ref={list}>
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
          <Icon name={r.icon === 'tag' ? 'tag' : r.icon === 'reset' ? 'renew' : r.icon === 'logout' ? 'logout' : r.icon === 'key' ? 'key' : 'trash'} />
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
  const projects = useData(s => s.ws?.projects ?? []);
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
                <div className="swatches">
                  {PROJECT_COLORS.map(c => (
                    <button
                      key={c}
                      type="button"
                      className={cx('swatch', c === v[f.name] && 'on')}
                      style={{ ['--sw' as string]: c }}
                      aria-label={c}
                      onClick={() => update(f.name, c)}
                    />
                  ))}
                </div>
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
          <button className="btn brand" type="submit" disabled={busy}>
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
    const down = (e: PointerEvent) => !(e.target as HTMLElement).closest('.ctx') && closeCtx();
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
        <div className="ctx-swatches">
          {PROJECT_COLORS.map(c => (
            <button
              key={c}
              type="button"
              className={cx('swatch', c === p.color && 'on')}
              style={{ ['--sw' as string]: c }}
              aria-label={`颜色 ${c}`}
              onClick={() => setProjectColor(p.key, c)}
            />
          ))}
        </div>
        <div className="ctx-sep" />
        <button type="button" data-do="delete-project" className="danger" onClick={go(() => deleteProject(p.key))}>
          <Icon name="trash" />
          删除项目
        </button>
      </>
    );
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
      className={cx('ctx', pos && 'on')}
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
  const create = async () => {
    if (!req) return;
    try {
      const l = await api.createLabel(req.project, query);
      await refresh();
      setQ('');
      updatePicker([...sel, l.id]);
      await req.onToggle(l.id, true);
    } catch (e) {
      toastError(e);
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
      <div className="lbp-list scroll" role="listbox">
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
          <button type="button" className={cx('lbp-row create', active === list.length && 'active')} onClick={create}>
            <Icon name="plus" />
            新建标签「{query}」
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

  const run = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
      await refresh();
    } catch (e) {
      toastError(e);
      await refresh();
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
          标签属于项目，项目下的所有白板共用。改名或改色会立即生效；删除会从所有卡片上移除。agent 用 <code>board label &lt;ID&gt; +名称</code> 打标签。
        </p>
        <div className="lbm-list scroll" ref={list}>
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
                  await run(() => api.updateLabel(pk, l.id, { name: v }));
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
              <span className="lbm-count">{l.uses} 张卡片</span>
              <button
                type="button"
                className="icon-btn sm lbm-del"
                aria-label="删除"
                onClick={async () => {
                  if (l.uses && !(await confirm({ title: `删除标签「${l.name}」？`, text: `${l.uses} 张卡片上的这个标签会一起移除。`, ok: '删除标签', icon: 'tag' })))
                    return;
                  await run(() => api.deleteLabel(pk, l.id));
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
                        await run(() => api.updateLabel(pk, l.id, { color: c }));
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
            const ok = await run(() => api.createLabel(pk, name, nc));
            if (ok) {
              setName('');
              setNewColor(null);
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
          <button type="submit" className="btn sm primary">
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

