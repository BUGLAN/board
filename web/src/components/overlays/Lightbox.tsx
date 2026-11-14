/* 放大查看图片（附件），多图左右切换 */
import { useEffect, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { closeImage, stepImage, useUI } from '../../store/ui';
import { Icon } from '../Icon';

/**
 * 当前画面（default/main#328：切换图片时整屏闪烁）。
 * 切换时先留着上一张当底图，新图画出第一帧、淡入动画接着跑，底图才撤掉：切换过程中屏幕上始终是一张完整的图，
 * 不会出现「遮罩在、图还没画出来」那一帧（闪烁就是那一帧把底下的页面露了出来）。
 */
interface Stage {
  src: string;
  alt: string;
  /** 上一张的 src，新图画出来之前垫在下面 */
  prev: string | null;
  /**
   * 新图已经解码好了（或加载失败），去掉藏它的透明度；
   * 同时当作淡入的锚点：每换一张图重新挂一个新的 <img>，动画从头跑
   */
  loaded: boolean;
  /** 淡入跑完了：这一帧已经画出来，底图可以撤了 */
  ready: boolean;
}

export function Lightbox() {
  const cur = useUI(s => s.image);
  const open = !!cur;
  const img = cur ? cur.list[cur.index] : null;
  const target = img?.src ?? null;
  const wasOpen = useRef(open);
  const frontRef = useRef<HTMLImageElement | null>(null);
  const [stage, setStage] = useState<Stage | null>(() => (img ? { src: img.src, alt: img.alt, prev: null, loaded: false, ready: false } : null));
  // 换图：这一帧就记成「新图盖在上一张上」，等新图画出来再撤底图。关掉时直接清空，不留底图
  if ((stage?.src ?? null) !== target) setStage(target ? { src: target, alt: img?.alt ?? '', prev: stage?.src ?? null, loaded: false, ready: false } : null);
  // 新图解码完的（缓存命中、小图）当场就撤底图，不等 load 事件
  if (stage && !stage.loaded && frontRef.current?.naturalWidth) setStage({ ...stage, loaded: true });
  // 关掉再打开同一个预览：淡入重跑一遍（换新的动画锚点），不然再打开时透明度 0 没人清，预览会一直空着
  if (stage?.src === target && open && !wasOpen.current) setStage({ ...stage, loaded: false, ready: false });
  wasOpen.current = open;
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
  if (!cur || !stage) return null;
  const n = cur.list.length;
  // 新图解码完：去掉内联的透明度 0，让它接着跑 .lb-img 的淡入（lb-img-in）。底图等淡入跑完再撤。
  // 这里直接改 DOM 的样式，同时改 state 让 React 一并把内联样式撤掉：只改 state 的话 React 下一次提交
  // 不碰这个属性，透明度 0 会一直留着，图一直看不见
  const loaded = (e: SyntheticEvent<HTMLImageElement>) => {
    e.currentTarget.style.opacity = '';
    setStage(s => (s && !s.loaded ? { ...s, loaded: true } : s));
  };
  // 淡入跑完：这一帧已经画出来了，撤掉底图
  const shown = () => setStage(s => (s && !s.ready ? { ...s, ready: true } : s));
  return (
    <div className="lightbox" role="dialog" aria-label={stage.alt || '图片'} onClick={closeImage}>
      {/* 这一层（连同 .lightbox）切换图片时不卸载：淡入动画、backdrop-filter 的合成层都不被丢掉 */}
      <div className="lb-stage">
        {stage.prev && !stage.ready && <img key={`${stage.prev}-${stage.src}`} className="lb-img lb-prev" src={stage.prev} alt="" aria-hidden="true" />}
        {/* 新图还没解码时按内联的透明度 0 藏着，解码完（loaded）换成新的节点、重新淡入 */}
        <img
          key={stage.loaded ? `show-${stage.loaded}-${stage.src}` : `wait-${stage.src}`}
          ref={frontRef}
          className="lb-img"
          src={stage.src}
          alt={stage.alt}
          style={stage.loaded ? undefined : { opacity: 0 }}
          onLoad={loaded}
          onError={loaded}
          onAnimationEnd={shown}
        />
      </div>
      {/* 底部条：文件名、下载，多张时右边是切换；关闭靠 Esc 或点遮罩 */}
      <div className="lb-bar" onClick={e => e.stopPropagation()}>
        <span className="lb-name">{stage.alt}</span>
        <a className="btn sm" href={stage.src} download={stage.alt || true}>
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
