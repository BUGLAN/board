/* ==========================================================================
   复制为新卡片（PRD §10.8）：卡片详情底部「复制为新卡片」打开的居中弹窗。
   列出原卡的经历（不带过去），选带过去的标签、附件、人的评论，新卡放到哪，原卡怎么处理，一条命令（Cmd copy）做完。
   乐观更新：新卡当帧出现（临时卡片），原卡当帧移走或拿掉；服务端确认后详情换到新卡
   ========================================================================== */
import { useEffect, useRef, useState } from 'react';
import { openLink } from '../lib/actions';
import { carryLabels, filesToCopy, pastOf, withNotes } from '../lib/copy';
import { boardFlow } from '../lib/schema';
import type { CardDetail } from '../lib/types';
import { cx, isImeKey } from '../lib/util';
import { draftCard, exec, joinPatch, movePatch, removeCard, useCurrent, useData } from '../store/data';
import { openCopy, openPicker, toast, toastError, useUI } from '../store/ui';
import { LabelChip } from './bits';
import { Icon } from './Icon';
import { usePresence } from './overlays/presence';

export function CopyDialog() {
  const id = useUI(s => s.copyOf);
  const detail = useData(s => (id != null && s.detail?.id === id ? s.detail : null));
  const open = id != null && !!detail;
  const { mounted, on } = usePresence(open, 220);
  const last = useRef(detail);
  if (detail) last.current = detail;
  // 详情换到了别的卡片、关掉了：面板跟着关
  useEffect(() => {
    if (id != null && !detail) openCopy(null);
  }, [id, detail]);
  if (!mounted || !last.current) return null;
  return <CopyBody key={last.current.id} src={last.current} on={on && open} />;
}

type Old = 'keep' | 'final' | 'trash';

function CopyBody({ src, on }: { src: CardDetail; on: boolean }) {
  const cur = useCurrent();
  const board = useData(s => s.board);
  const f = boardFlow(board);
  const all = cur?.project.labels ?? [];
  const past = pastOf(src);
  const final = f.list.find(s => s.final);
  const home = f.home;
  const held = !!src.holder;
  const [labels, setLabels] = useState(() => carryLabels(src.labels, all));
  const [files, setFiles] = useState(true);
  const [notes, setNotes] = useState(false);
  const [dest, setDest] = useState<'inbox' | 'todo'>('inbox');
  const [old, setOld] = useState<Old>('keep');
  const body = notes ? withNotes(src.body, src.id, past.human) : src.body;
  const copies = filesToCopy(src, body);
  // 附件一行在有可能复制的附件时才有（勾上人的评论才引用到的也算），勾选人的评论时弹窗高度不变
  const maybe = filesToCopy(src, withNotes(src.body, src.id, past.human));
  const close = () => openCopy(null);

  // esc 关掉；标签选择器、管理标签开着时先关它们
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || isImeKey(e)) return;
      const ui = useUI.getState();
      if (ui.picker || ui.labelsOf) return;
      e.preventDefault();
      e.stopPropagation();
      close();
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, []);

  const pick = (anchor: HTMLElement) =>
    cur &&
    openPicker({
      rect: anchor.getBoundingClientRect(),
      project: cur.project.key,
      selected: labels,
      onToggle: (l, add) => setLabels(ls => (add ? [...new Set([...ls, l])] : ls.filter(x => x !== l))),
    });

  const submit = () => {
    const path = useData.getState().path;
    const inbox = dest === 'inbox';
    const temp = draftCard({ title: src.title, body, labels, column: inbox ? 'inbox' : (home?.key ?? 'todo') });
    const moveTo = old === 'final' && final ? final.key : undefined;
    const gone = moveTo ? movePatch(src.id, moveTo) : old === 'trash' ? removeCard(src.id) : null;
    // 原卡也记进「保存中」：确认之前不能再改它
    const patch = joinPatch(temp.patch, ...(gone ? [gone] : []), { cards: [src.id] });
    close();
    exec(
      {
        op: 'copy',
        id: src.id,
        title: src.title,
        body,
        column: inbox ? 'inbox' : undefined,
        labels,
        files: files && copies.length > 0,
        move_to: moveTo,
        trash: old === 'trash' || undefined,
      },
      patch,
      c => temp.settle(c.id),
    ).then(
      c => {
        const s = useData.getState();
        // 还看着原卡：换到新卡；已经去看别的了，提示一下
        if (s.path === path && s.detailId === src.id) openLink(path, c.id);
        else toast({ type: 'success', title: `已复制为 #${c.id}`, action: { label: '打开', fn: () => openLink(path, c.id) } });
      },
      toastError,
    );
  };

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog wide cpy-dialog" role="dialog" aria-modal="true" aria-label={`复制 #${src.id} 为新卡片`}>
        <h3>复制 #{src.id} 为新卡片</h3>

        <div className="cpy-past" aria-label="原卡的经历，不带过去">
          <span className="cpy-past-k">原卡的经历</span>
          <span>
            领取 <b>{past.runs}</b> 次
          </span>
          <span className={cx(past.lost > 0 && 'bad')}>
            异常退出 <b>{past.lost}</b> 次
          </span>
          {past.errors > 0 && (
            <span className="bad">
              报错日志 <b>{past.errors}</b> 条
            </span>
          )}
          {past.wips.length > 0 && (
            <span title={past.wips.join('\n')}>
              wip 分支 <b>{past.wips.length}</b> 条
            </span>
          )}
          <span>
            动态 <b>{past.events}</b> 条
          </span>
        </div>

        <div className="cpy-sec">
          <div className="cpy-k">带过去</div>
          <div className="cpy-rows">
            <label className="cpy-row on locked">
              <input type="checkbox" checked readOnly disabled />
              <span>
                <b>标题和描述</b>
              </span>
            </label>
            <div className="cpy-row on cpy-labels">
              <span className="cpy-ic" aria-hidden="true">
                <Icon name="tag" />
              </span>
              <span className="d-labels">
                {all
                  .filter(l => labels.includes(l.id))
                  .map(l => (
                    <LabelChip key={l.id} label={l} onRemove={() => setLabels(ls => ls.filter(x => x !== l.id))} />
                  ))}
                <button type="button" className="lb-add" aria-label="添加标签" data-label-picker onClick={e => pick(e.currentTarget)}>
                  <Icon name="tag" />
                  {labels.length ? '' : '添加标签'}
                </button>
              </span>
            </div>
            {maybe.length > 0 && (
              <label className={cx('cpy-row', files && copies.length > 0 && 'on')}>
                <input type="checkbox" checked={files && copies.length > 0} disabled={!copies.length} onChange={e => setFiles(e.target.checked)} />
                <span>
                  <b>附件 {copies.length} 个</b>
                  <em>{copies.map(a => a.name).join('、')}</em>
                </span>
              </label>
            )}
            {past.human.length > 0 && (
              <label className={cx('cpy-row', notes && 'on')}>
                <input type="checkbox" checked={notes} onChange={e => setNotes(e.target.checked)} />
                <span>
                  <b>人的评论 {past.human.length} 条</b>
                  <em>「{past.human[past.human.length - 1].text.trim()}」</em>
                </span>
              </label>
            )}
          </div>
        </div>

        <div className="cpy-sec">
          <div className="cpy-k">新卡放到</div>
          <div className="cpy-seg" role="radiogroup" aria-label="新卡放到">
            <button type="button" role="radio" aria-checked={dest === 'inbox'} className={cx(dest === 'inbox' && 'on')} onClick={() => setDest('inbox')}>
              未整理
            </button>
            <button type="button" role="radio" aria-checked={dest === 'todo'} className={cx(dest === 'todo' && 'on')} onClick={() => setDest('todo')}>
              {home?.name ?? '待办'}
            </button>
          </div>
        </div>

        <div className="cpy-sec">
          <div className="cpy-k">原卡 #{src.id}</div>
          {/* 原卡正被领取时后两个禁用（不隐藏，弹窗高度不变），原因放在悬停提示里 */}
          <div className="cpy-seg three" role="radiogroup" aria-label="原卡怎么处理">
            <button type="button" role="radio" aria-checked={old === 'keep'} className={cx(old === 'keep' && 'on')} onClick={() => setOld('keep')}>
              留着
            </button>
            <button
              type="button"
              role="radio"
              aria-checked={old === 'final'}
              disabled={held || !final || src.column === final.key}
              title={held ? '正被领取' : undefined}
              className={cx(old === 'final' && 'on')}
              onClick={() => setOld('final')}
            >
              移到「{final?.name ?? '已完成'}」
            </button>
            <button
              type="button"
              role="radio"
              aria-checked={old === 'trash'}
              disabled={held}
              title={held ? '正被领取' : undefined}
              className={cx(old === 'trash' && 'on')}
              onClick={() => setOld('trash')}
            >
              移进回收站
            </button>
          </div>
        </div>

        <div className="dialog-actions">
          <button className="btn" type="button" onClick={close}>
            取消
          </button>
          <button className="btn brand" type="button" onClick={submit}>
            <Icon name="copy" />
            复制
          </button>
        </div>
      </div>
    </div>
  );
}
