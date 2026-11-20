/* 原型（用完即弃）：底部切换条——分工页变体（←/→ 也能换）、派发对话框变体、白板卡片上的去向、动态折叠、几个快捷跳转、重置。只在开发环境出现 */
import { useEffect } from 'react';
import { openBoard, openCard } from '../../lib/actions';
import { cx } from '../../lib/util';
import { openDispatch, setTab } from '../../store/ui';
import { goSec } from '../../components/agent/AgentNav';
import { ProfileForm } from './bits';
import { resetModel } from './model';
import { DL, PG, isProto, setChip, setDl, setFold, setPg, startOnPage, useProto } from './proto';
import './proto.css';

const go = (id: number, then?: () => void) => {
  setTab('board');
  openBoard('oss/main');
  setTimeout(() => {
    openCard(id);
    if (then) setTimeout(then, 250);
  }, 150);
};

export function ProtoRoot() {
  const { pg, dl, chip, fold } = useProto();
  const idx = PG.findIndex(x => x.key === pg);
  const step = (d: number) => setPg(PG[(idx + d + PG.length) % PG.length].key);
  useEffect(() => {
    if (startOnPage) goSec('rules');
  }, []);
  useEffect(() => {
    if (!isProto) return;
    const key = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest?.('input, textarea, [contenteditable]')) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === 'ArrowLeft') step(-1);
      else if (e.key === 'ArrowRight') step(1);
      else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });
  if (!isProto) return null;
  return (
    <>
      <ProfileForm />
      <div className="pfp-bar" role="toolbar" aria-label="原型切换">
        <span className="pfp-tag">分工页</span>
        <button type="button" aria-label="上一个变体" onClick={() => step(-1)}>
          ←
        </button>
        {PG.map(x => (
          <button
            key={x.key}
            type="button"
            className={cx('pfp-v', x.key === pg && 'on')}
            onClick={() => {
              setPg(x.key);
              setTab('agent');
              goSec('rules');
            }}
          >
            {x.key} {x.name}
          </button>
        ))}
        <button type="button" aria-label="下一个变体" onClick={() => step(1)}>
          →
        </button>
        <span className="pfp-sep" />
        <span className="pfp-tag">派发</span>
        {DL.map(x => (
          <button key={x.key} type="button" className={cx('pfp-v', x.key === dl && 'on')} onClick={() => setDl(x.key)}>
            {x.key} {x.name}
          </button>
        ))}
        <button type="button" onClick={() => go(502, () => openDispatch(502))}>
          打开派发
        </button>
        <span className="pfp-sep" />
        <button type="button" className={cx(chip && 'on')} onClick={() => setChip(!chip)}>
          卡片上写去向 {chip ? '开' : '关'}
        </button>
        <button type="button" className={cx(fold && 'on')} onClick={() => setFold(!fold)}>
          动态折叠 {fold ? '开' : '关'}
        </button>
        <span className="pfp-sep" />
        <button type="button" onClick={() => go(503)}>
          #503
        </button>
        <button type="button" onClick={() => go(512)}>
          #512
        </button>
        <button type="button" onClick={resetModel}>
          重置
        </button>
      </div>
    </>
  );
}
