import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import '@fontsource-variable/bricolage-grotesque';
import './styles/tokens.css';
import './styles/base.css';
import './styles/layout.css';
import './styles/workbench.css';
import './styles/canvas.css';
import './styles/labels.css';
import './styles/board.css';
import './styles/settings.css';
import './styles/overlays.css';
import './styles/platform.css';
import './styles/app.css';
import './styles/hosts.css';
import './styles/hostdetail.css';
import './styles/profiles.css';
import './styles/inbox.css';
import './styles/milestones.css';
import './styles/trash.css';
import './styles/review.css';
import './styles/skeleton.css';

import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ErrorBoundary } from './components/ErrorBoundary';
import { applyPrefs } from './lib/platform';
import { installScrollbars } from './lib/scrollbars';
import { installIosTopBar } from './lib/iosTopBar';
import { buildVersion } from './lib/build';
import { noticeStale, watchStale } from './lib/stale-runner';
import { autoUpdateCheck } from './lib/update-runner';
import { checkServerVersion } from './store/data';

applyPrefs();
installScrollbars();
// iOS 主屏安装版：让 iOS 认出顶部实色层、不画系统级模糊（default/main#627；其他环境什么都不做）
installIosTopBar();
// 桌面端：打开 Board 时自动检查一次，之后每 30 分钟一次（设置里可以关；网页端什么都不做）
autoUpdateCheck();
// 服务端升级之后还开着的旧页面：启动、切回标签页时查一次版本，不一样就提示刷新（default/main#368）
watchStale(checkServerVersion, buildVersion());
if (!window.__BOARD_SHELL__) void checkServerVersion().then(v => noticeStale(buildVersion(), v));
createRoot(document.getElementById('root')!).render(
  <ErrorBoundary>
    <App />
  </ErrorBoundary>,
);
