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
import './styles/skeleton.css';

import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ErrorBoundary } from './components/ErrorBoundary';
import { applyPrefs } from './lib/platform';
import { installScrollbars } from './lib/scrollbars';
import { autoUpdateCheck } from './lib/update-runner';

applyPrefs();
installScrollbars();
// 桌面端：打开 Board 时自动检查一次，之后每 30 分钟一次（设置里可以关；网页端什么都不做）
autoUpdateCheck();
createRoot(document.getElementById('root')!).render(
  <ErrorBoundary>
    <App />
  </ErrorBoundary>,
);
