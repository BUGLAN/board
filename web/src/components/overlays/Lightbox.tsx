/* 放大查看图片（附件），多图左右切换 */
import { useEffect } from 'react';
import { closeImage, stepImage, useUI } from '../../store/ui';
import { Icon } from '../Icon';

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
      {/* 底部条：文件名、下载，多张时右边是切换；关闭靠 Esc 或点遮罩 */}
      <div className="lb-bar" onClick={e => e.stopPropagation()}>
        <span className="lb-name">{img.alt}</span>
        <a className="btn sm" href={img.src} download={img.alt || true}>
          下载
        </a>
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
      </div>
    </div>
  );
}
