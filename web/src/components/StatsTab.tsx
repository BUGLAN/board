/* ==========================================================================
   大盘 Tab（PRD §10.13）：页面和侧栏按需加载（stats/Stats），第一次切到大盘时才下载
   ========================================================================== */
import { lazy, Suspense, useState } from 'react';
import { useUI } from '../store/ui';
import { PageLoading } from './Skeleton';

const Page = lazy(() => import('./stats/Stats').then(m => ({ default: m.StatsPage })));
const Side = lazy(() => import('./stats/Stats').then(m => ({ default: m.StatsSide })));

/** 大盘主区；加载中用骨架占位 */
export function StatsPage() {
  return (
    <Suspense fallback={<PageLoading name="stats" />}>
      <Page />
    </Suspense>
  );
}

/** 侧栏的范围导航：第一次切到大盘 Tab 时才加载，之后一直挂着（侧栏切换有过渡） */
export function StatsSide() {
  const tab = useUI(s => s.tab);
  const [seen, setSeen] = useState(tab === 'stats');
  if (!seen && tab === 'stats') setSeen(true);
  if (!seen) return <section className="side-view" data-view="stats" />;
  return (
    <Suspense fallback={<section className="side-view" data-view="stats" />}>
      <Side />
    </Suspense>
  );
}
