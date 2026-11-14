/* 标签选择器、标签管理 */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { LABEL_COLORS } from '../../lib/schema';
import { hiddenBy, hiddenRules, ruleError, visibleLabels } from '../../lib/sidebarLabels';
import { cx, replay } from '../../lib/util';
import { deleteLabelPatch, editLabelPatch, hiddenLabelsKey, hiddenLabelsPatch, optimistic, reorderLabels, useBusy, useData, type Patch } from '../../store/data';
import { closeLabels, closePicker, confirm, manageLabels, toastError, updatePicker, useUI } from '../../store/ui';
import { ColorPick, InfoTip, LabelChip, Tip } from '../bits';
import { Icon } from '../Icon';
import { usePresence } from './presence';

export function LabelPicker() {
  const req = useUI(s => s.picker);
  const ws = useData(s => s.ws);
  const ref = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [q, setQ] = useState('');
  const [active, setActive] = useState(0);
  const [pos, setPos] = useState({ left: 0, top: 0 });
  const [on, setOn] = useState(false);
  const fixed = !!req?.labels;
  const project = ws?.projects.find(p => p.key === req?.project);
  const labels = req?.labels ?? project?.labels ?? [];

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
  // 被隐藏的标签（默认是版本标签）不在可选列表里，default/main#324；已经勾上的、名称和输入完全一样的例外
  const exactId = labels.find(l => l.name.toLowerCase() === query.toLowerCase())?.id;
  const shown = fixed ? labels : visibleLabels(labels, hiddenRules(project), [...(req?.selected ?? []), ...(exactId != null ? [exactId] : [])]);
  const list = shown.filter(l => !query || l.name.toLowerCase().includes(query.toLowerCase()));
  const exact = labels.some(l => l.name.toLowerCase() === query.toLowerCase());
  const canCreate = !fixed && !!query && !exact;
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
          placeholder={fixed ? '搜索标签…' : '搜索或新建标签…'}
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
        {!total && <div className="lbp-empty">{fixed ? (query ? '没有这个标签' : '还没有标签') : '还没有标签，输入名称即可新建'}</div>}
      </div>
      {!fixed && (
      <div className="lbp-foot">
        <button type="button" onClick={() => req && manageLabels(req.project)}>
          <Icon name="edit" />
          管理标签
        </button>
      </div>
      )}
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

/** 「优先」开关：带这个标签的卡片先被领取 */
function PriorityToggle({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  return (
    <button
      type="button"
      className={cx('lbm-kind lbm-prio', on && 'on')}
      aria-pressed={on}
      title={on ? '优先：带这个标签的卡片先被领取；点一下取消' : '点一下设为优先：带这个标签的卡片先被领取'}
      onClick={() => onChange(!on)}
    >
      <Icon name="bolt" />
      优先
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
  const rules = hiddenRules(p);
  const nc = newColor ?? LABEL_COLORS[labels.length % LABEL_COLORS.length];
  // 拖动调整顺序（default/main#289）：拖着的那一行跟着指针走，插入位置画一道线，松手当帧换位，顺序写进项目；侧栏跟着变
  const [drag, setDrag] = useState<{ id: number; dy: number; to: number } | null>(null);
  const move = (id: number, to: number) => {
    const ids = labels.map(l => l.id);
    const from = ids.indexOf(id);
    if (from < 0 || to < 0 || to > ids.length || to === from || to === from + 1) return;
    ids.splice(from, 1);
    ids.splice(to > from ? to - 1 : to, 0, id);
    reorderLabels(pk, ids).catch(toastError);
  };
  const grip = (e: React.PointerEvent<HTMLElement>, id: number) => {
    if (e.button !== 0 || !list.current) return;
    e.preventDefault();
    const box = list.current;
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const y0 = e.clientY + box.scrollTop;
    let to = labels.findIndex(l => l.id === id);
    let auto = 0;
    let lastY = e.clientY;
    const at = (y: number) => {
      const rows = [...box.querySelectorAll<HTMLElement>(':scope > .lbm-row')];
      const i = rows.findIndex(r => {
        const b = r.getBoundingClientRect();
        return y < b.top + b.height / 2;
      });
      return i < 0 ? rows.length : i;
    };
    const update = () => {
      to = at(lastY);
      setDrag({ id, dy: lastY + box.scrollTop - y0, to });
    };
    // 拖到列表上下边缘时自动滚动
    const scroll = () => {
      const b = box.getBoundingClientRect();
      const d = lastY < b.top + 24 ? -8 : lastY > b.bottom - 24 ? 8 : 0;
      if (d) {
        box.scrollTop += d;
        update();
      }
      auto = requestAnimationFrame(scroll);
    };
    auto = requestAnimationFrame(scroll);
    const onMove = (ev: PointerEvent) => {
      lastY = ev.clientY;
      update();
    };
    const end = (ok: boolean) => {
      cancelAnimationFrame(auto);
      el.removeEventListener('pointermove', onMove);
      el.removeEventListener('pointerup', onUp);
      el.removeEventListener('pointercancel', onCancel);
      setDrag(null);
      if (ok) move(id, to);
    };
    const onUp = () => end(true);
    const onCancel = () => end(false);
    el.addEventListener('pointermove', onMove);
    el.addEventListener('pointerup', onUp);
    el.addEventListener('pointercancel', onCancel);
    update();
  };

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
        <div className="lbm-list" ref={list}>
          {labels.length === 0 && <div className="lbp-empty">这个项目还没有标签</div>}
          {labels.map((l, i) => (
            <div
              className={cx(
                'lbm-row',
                drag?.id === l.id && 'dragging',
                drag && drag.id !== l.id && drag.to === i && 'ins-before',
                drag && drag.id !== l.id && drag.to === labels.length && i === labels.length - 1 && 'ins-after',
              )}
              key={l.id}
              style={{ ['--lc' as string]: l.color, ...(drag?.id === l.id ? { transform: `translateY(${drag.dy}px)` } : null) }}
            >
              <Tip text="拖动调整顺序">
                <button
                  type="button"
                  className="lbm-grip"
                  aria-label={`调整「${l.name}」的顺序（↑ ↓）`}
                  onPointerDown={e => grip(e, l.id)}
                  onKeyDown={e => {
                    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
                    e.preventDefault();
                    move(l.id, e.key === 'ArrowUp' ? i - 1 : i + 2);
                  }}
                >
                  <Icon name="reorder" />
                </button>
              </Tip>
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
              <PriorityToggle
                on={!!l.priority}
                onChange={v => run(editLabelPatch(pk, l.id, { priority: v }), () => api.updateLabel(pk, l.id, { priority: v }))}
              />
              <span className="lbm-count">
                {hiddenBy(l.name, rules) && (
                  <Tip text={`名称匹配隐藏规则 ${hiddenBy(l.name, rules)}：不出现在侧栏、提示词菜单、标签选择器`}>
                    <em className="lbm-hid">已隐藏</em>
                  </Tip>
                )}
                {l.uses} 张卡片
              </span>
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
                  {/* 最后是取色器：任意颜色 */}
                  <ColorPick
                    value={l.color}
                    custom={!LABEL_COLORS.includes(l.color.toUpperCase())}
                    label={`「${l.name}」的颜色`}
                    onPick={async c => {
                      setSwatch(null);
                      if (c === l.color.toUpperCase()) return;
                      await run(editLabelPatch(pk, l.id, { color: c }), () => api.updateLabel(pk, l.id, { color: c }));
                    }}
                  />
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
              <ColorPick
                value={nc}
                custom={!LABEL_COLORS.includes(nc.toUpperCase())}
                label="新标签的颜色"
                onPick={c => {
                  setNewColor(c);
                  setSwatch(null);
                }}
              />
            </div>
          )}
        </form>
        <HiddenRules pk={pk} rules={rules} labels={labels} />
      </div>
    </div>
  );
}

/** 隐藏的标签（default/main#324）：一组正则，名称匹配任一条的标签不出现在侧栏、提示词菜单、标签选择器。
    每条后面写命中几个标签；× 删掉一条就是取消隐藏。整组保存在项目上，所有白板、所有设备一样 */
function HiddenRules({ pk, rules, labels }: { pk: string; rules: string[]; labels: { name: string }[] }) {
  const [text, setText] = useState('');
  const [error, setError] = useState('');
  const busy = useBusy(hiddenLabelsKey(pk));
  const input = useRef<HTMLInputElement>(null);
  const save = async (next: string[]) => {
    try {
      await optimistic(hiddenLabelsPatch(pk, next), () => api.updateProject(pk, { hiddenLabels: next }));
      return true;
    } catch (e) {
      toastError(e);
      return false;
    }
  };
  const add = async () => {
    const r = text.trim();
    const why = ruleError(r, rules);
    if (why) {
      setError(why);
      replay(input.current, 'invalid');
      return;
    }
    setError('');
    setText('');
    // 失败时把输入放回去，不丢
    if (!(await save([...rules, r]))) setText(r);
    input.current?.focus();
  };
  const hits = (r: string) => labels.filter(l => hiddenBy(l.name, [r])).length;
  return (
    <section className="lbm-hidden" aria-label="隐藏的标签">
      <div className="lbm-hidden-head">
        <b>隐藏的标签</b>
        <InfoTip text="名称匹配任一条正则（不区分大小写）的标签，不出现在侧栏、提示词菜单和标签选择器里；卡片上、卡片详情、里程碑照常显示。删掉一条规则就取消隐藏。" />
      </div>
      <div className="lbm-rules">
        {rules.length === 0 && <span className="lbm-rules-empty">没有隐藏的标签</span>}
        {rules.map(r => {
          const bad = ruleError(r) !== null;
          const n = hits(r);
          return (
            <span key={r} className={cx('lbm-rule', bad && 'bad')}>
              <code>{r}</code>
              <em>{bad ? '写错了，不生效' : n ? `${n} 个` : '没有命中'}</em>
              <button type="button" className="icon-btn sm" aria-label={`删掉规则 ${r}（取消隐藏）`} disabled={busy} onClick={() => void save(rules.filter(x => x !== r))}>
                <Icon name="close" />
              </button>
            </span>
          );
        })}
      </div>
      <form
        className="lbm-rule-new"
        noValidate
        onSubmit={e => {
          e.preventDefault();
          void add();
        }}
      >
        <input
          ref={input}
          className={cx('input mono', error && 'invalid')}
          placeholder="正则，例如 ^v\d 或 ^tmp-"
          maxLength={200}
          autoComplete="off"
          spellCheck={false}
          aria-label="新的隐藏规则"
          aria-invalid={!!error || undefined}
          value={text}
          onChange={e => {
            setText(e.target.value);
            if (error) setError('');
          }}
        />
        <button type="submit" className={cx('btn sm', busy && 'is-busy')} disabled={busy} aria-busy={busy || undefined}>
          <Icon name="plus" />
          隐藏
        </button>
      </form>
      {error && (
        <p className="lbm-rule-err" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
