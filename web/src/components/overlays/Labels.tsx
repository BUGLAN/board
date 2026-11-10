/* 标签选择器、标签管理 */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { LABEL_COLORS } from '../../lib/schema';
import { cx, replay } from '../../lib/util';
import { deleteLabelPatch, editLabelPatch, optimistic, useData, type Patch } from '../../store/data';
import { closeLabels, closePicker, confirm, manageLabels, toastError, updatePicker, useUI } from '../../store/ui';
import { LabelChip } from '../bits';
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
          <b>仅人可打</b>的标签带小锁，只有人能在界面上加、去，agent 打会被拒绝。<b>优先</b>的标签带闪电：不带 ID 领取（和定时领取）时，带它的卡片先被领取，卡片在状态里的位置不变。
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
              <PriorityToggle
                on={!!l.priority}
                onChange={v => run(editLabelPatch(pk, l.id, { priority: v }), () => api.updateLabel(pk, l.id, { priority: v }))}
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
