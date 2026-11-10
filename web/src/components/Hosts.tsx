/* ==========================================================================
   机器（board host）与派发（任务下发需求 §八）
   - 机器 Tab（侧栏机器列表、主区机位总览）和机器详情弹窗按需加载：第一次用到时才下载（hosts/HostsPage、hosts/HostModal）
   - 卡片上的派发状态、卡片详情里的派发说明、「派发」对话框在 hosts/Dispatch（白板上就要用，不按需加载）
   ========================================================================== */
import { lazy, Suspense, useState } from 'react';
import { useUI } from '../store/ui';
import { PageLoading } from './Skeleton';

export { CardDispatchLine, DispatchDialog, DispatchNote } from './hosts/Dispatch';

const Page = lazy(() => import('./hosts/HostsPage').then(m => ({ default: m.HostsPage })));
const Side = lazy(() => import('./hosts/HostsPage').then(m => ({ default: m.HostsSide })));
const Modal = lazy(() => import('./hosts/HostModal').then(m => ({ default: m.HostModal })));

/** 机器页主区；加载中用骨架占位 */
export function HostsPage() {
  return (
    <Suspense fallback={<PageLoading name="hosts" />}>
      <Page />
    </Suspense>
  );
}

/** 侧栏的机器列表：第一次切到机器 Tab 时才加载，之后一直挂着（侧栏切换有过渡） */
export function HostsSide() {
  const tab = useUI(s => s.tab);
  const [seen, setSeen] = useState(tab === 'hosts');
  if (!seen && tab === 'hosts') setSeen(true);
  if (!seen) return <section className="side-view" data-view="hosts" />;
  return (
    <Suspense fallback={<section className="side-view" data-view="hosts" />}>
      <Side />
    </Suspense>
  );
}

/** 机器详情弹窗：第一次打开时才加载，之后一直挂着（关闭有退场动画） */
export function HostModal() {
  const open = useUI(s => s.host != null);
  const [seen, setSeen] = useState(open);
  if (!seen && open) setSeen(true);
  if (!seen) return null;
  return (
    <Suspense fallback={null}>
      <Modal />
    </Suspense>
  );
}
