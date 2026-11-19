/* ==========================================================================
   「未整理」页三个标签（未整理、稍后再做、回收站）共用的行部件：勾选框、行的骨架。
   样式在 styles/inbox.css（.ib-check、.ib-grp、.ib-rows、.ib-line）
   ========================================================================== */
import { cx } from '../lib/util';
import { Skeleton } from './Skeleton';

/** 勾选框：自己画的方块（选中打勾、部分选中一道横线），点的时候弹一下。
    一个按钮就是一个勾选框（读屏报出名称和勾没勾），空格、回车切换，按住 Shift 连选；
    「全选」和各行的勾选一起按 aria-checked / aria-label 报出来（default/main#315） */
export function Check({ on, some, label, onToggle }: { on: boolean; some?: boolean; label: string; onToggle: (shift: boolean) => void }) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={some ? 'mixed' : on}
      aria-label={label}
      className={cx('ib-check', on && 'on', some && 'some')}
      onClick={e => {
        e.stopPropagation();
        onToggle(e.shiftKey);
      }}
      onKeyDown={e => {
        // 回车照常触发 click；空格默认会滚页面，这里自己切换
        if (e.key !== ' ') return;
        e.preventDefault();
        e.stopPropagation();
        onToggle(e.shiftKey);
      }}
    >
      <i />
    </button>
  );
}

/** 列表还没取回来：一个组头加几行占位，和真实的行同高 */
export function RowsSkeleton({ n }: { n: number }) {
  return (
    <div className="ib-grp" aria-busy="true">
      <div className="ib-gh">
        <Skeleton w={48} h={10} />
      </div>
      <div className="ib-rows">
        {Array.from({ length: n }, (_, i) => (
          <div key={i} className="ib-line sk">
            <Skeleton w={16} h={16} r={5} />
            <Skeleton w={28} h={10} />
            <Skeleton w={20} h={20} r={999} />
            <Skeleton w={['48%', '62%', '36%', '54%'][i % 4]} h={11} />
          </div>
        ))}
      </div>
    </div>
  );
}
