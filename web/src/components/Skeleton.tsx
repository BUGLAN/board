/* ==========================================================================
   骨架屏：数据还没到时按真实内容的形状占位
   - <Skeleton w h r>：一块占位；<SkeletonText lines>：几行文字（最后一行短一些）
   - 颜色用 --surface-3，扫光节奏取 --dur-4；减弱动态效果时静态（styles/skeleton.css）
   - 白板骨架：SkeletonCard 和真实卡片同高（一行标题 + 一行标签），状态块的骨架见 BoardView
   ========================================================================== */
import type { CSSProperties } from 'react';
import { cx } from '../lib/util';

type Len = number | string;
const len = (v: Len | undefined) => (typeof v === 'number' ? `${v}px` : v);

export function Skeleton({ w, h = 12, r, className, style }: { w?: Len; h?: Len; r?: Len; className?: string; style?: CSSProperties }) {
  return <span className={cx('sk', className)} aria-hidden="true" style={{ width: len(w), height: len(h), borderRadius: len(r), ...style }} />;
}

/** 几行文字：每行高 h，最后一行宽 last */
export function SkeletonText({ lines = 2, h = 10, gap = 8, last = '62%' }: { lines?: number; h?: number; gap?: number; last?: Len }) {
  return (
    <span className="sk-text" aria-hidden="true" style={{ gap }}>
      {Array.from({ length: lines }, (_, i) => (
        <Skeleton key={i} h={h} w={i === lines - 1 && lines > 1 ? last : '100%'} />
      ))}
    </span>
  );
}

/** 卡片骨架：#ID、标题、标签胶囊，和一行标题 + 一行标签的真实卡片一样高 */
export function SkeletonCard({ seed = 0 }: { seed?: number }) {
  // 标题长短错开，免得一排一模一样
  const title = ['86%', '64%', '74%'][seed % 3];
  return (
    <div className="card sk-card" aria-hidden="true">
      <div className="card-row">
        <Skeleton w={28} h={10} />
      </div>
      <div className="sk-card-title">
        <Skeleton w={title} h={12} />
      </div>
      <div className="card-labels">
        <Skeleton w={40} h={20} r={6} />
        {seed % 2 === 0 && <Skeleton w={48} h={20} r={6} />}
      </div>
    </div>
  );
}

/** 按需加载的页面还在下载：和页面同样的容器，放几条骨架，不闪白 */
export function PageLoading({ name }: { name: string }) {
  return (
    <div className={`page page-${name}`} data-page={name} aria-busy="true">
      <div className="page-loading">
        <Skeleton w="34%" h={22} />
        <Skeleton w="62%" />
        <Skeleton w="48%" />
      </div>
    </div>
  );
}
