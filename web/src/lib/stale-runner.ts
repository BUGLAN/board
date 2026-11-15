/* ==========================================================================
   服务端升级之后提示刷新（lib/stale.ts 是判断，store/data.ts 记服务端版本，这里接界面）— default/main#368
   - 同一个服务端版本只提示一次；服务端再升级一次（提示过 0.1.31 之后变成 0.1.32）时再提示
   - 提示条上「刷新页面」一键刷新；没保存的草稿不用另外存：刷新后照样自动恢复（新建卡片的草稿、编辑框）
   - 桌面端自带的界面连别的版本的服务端时不提示：桌面端有自己的更新流程（怎么提示另定）
   - 检查的时机：启动、推送重连成功（store/data.ts 的 boot）、切回标签页（这里）
   ========================================================================== */
import { toast } from '../store/ui';
import { markStaleNoticed, staleNoticed, staleServer, staleText } from './stale';

/** 切回标签页时离上一次查不到这么久就不查：来回切几次不发一串请求 */
export const REFOCUS_GAP = 30 * 1000;

/** 网页端才提示：桌面端自带的界面有自己的更新流程（外壳在加载页面前注入 __BOARD_SHELL__） */
export const isWeb = () => (typeof window === 'undefined' ? false : !window.__BOARD_SHELL__);

/** 知道服务端版本之后：和这一页的版本不一样、这个版本还没提示过，就弹提示 */
export function noticeStale(build: string, server: string | null | undefined) {
  if (!isWeb()) return;
  const want = staleServer(build, server);
  if (!want || staleNoticed(want)) return;
  markStaleNoticed(want);
  toast({
    type: 'info',
    title: staleText(want),
    text: '这一页还是旧版本，接口可能已经变了。正在改的内容刷新后还在（草稿会恢复）',
    duration: 60000,
    action: { label: '刷新页面', fn: () => location.reload() },
  });
}

/** 挂了就不重复挂：切回标签页时查一次版本；check 是取服务端版本（store/data.ts 的 checkServerVersion） */
let watching = false;
let lastCheck = 0;

export function watchStale(check: () => Promise<string | null>, build: string) {
  if (watching || typeof document === 'undefined') return;
  watching = true;
  lastCheck = Date.now();
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible') return;
    const now = Date.now();
    if (now - lastCheck < REFOCUS_GAP) return;
    lastCheck = now;
    void check().then(v => noticeStale(build, v));
  });
}
