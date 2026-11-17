/* 大盘总览：一台机器都没有配工作空间（接得了这块白板）时的提醒 */
import { boardHosts, hostsOf } from '../lib/hosts';
import { useData } from '../store/data';
import { openBoardSettings, openHost, openProjectSettings } from '../store/ui';
import { Icon } from './Icon';

/** 去机器的「工作空间」：有机器就打开第一台的详情，没有就打开「添加机器」 */
function goDirs() {
  const first = hostsOf(useData.getState().ws)[0];
  openBoardSettings(null);
  openProjectSettings(null);
  openHost(first ? first.name : 'new');
}

export function NoHostsNote({ path }: { path: string }) {
  const none = useData(s => boardHosts(s.ws, path).length === 0);
  if (!none) return null;
  return (
    <div className="bs-rc-empty sp-ov-nohost" role="status">
      <Icon name="alert" />
      <span>还没有机器能接这块白板的卡：不会被自动领取，也派发不了。给机器配上工作空间就能接。</span>
      <button type="button" className="btn sm" onClick={goDirs}>
        去设置工作空间
      </button>
    </div>
  );
}
