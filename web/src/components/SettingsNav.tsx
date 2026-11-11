/* 设置页的分节导航（侧栏）和滚动定位。单独成文件：侧栏常驻，设置页本身按需加载 */
import { create } from 'zustand';
import { canHost, canUpdate } from '../lib/platform';
import { cx, reduceMotion, replay } from '../lib/util';
import { Icon, type IconName } from './Icon';

export const SECTIONS: { id: string; title: string; icon: IconName }[] = [
  { id: 'account', title: '账号', icon: 'user' },
  { id: 'appearance', title: '外观', icon: 'sun' },
  { id: 'board', title: '白板规则', icon: 'lock' },
  { id: 'agents', title: 'Agent 接入', icon: 'terminal' },
  { id: 'tokens', title: '访问令牌', icon: 'key' },
  ...(canHost ? [{ id: 'host', title: '本机', icon: 'host' as IconName }] : []),
  ...(canUpdate ? [{ id: 'update', title: '桌面端更新', icon: 'renew' as IconName }] : []),
  { id: 'shortcuts', title: '快捷键', icon: 'keyboard' },
];

export const useSpy = create<{ active: string; lock: boolean }>(() => ({ active: SECTIONS[0].id, lock: false }));

export function SettingsNav() {
  const active = useSpy(s => s.active);
  return (
    <nav className="settings-nav">
      {SECTIONS.map(s => (
        <a
          key={s.id}
          href={`#set-${s.id}`}
          className={cx('sn-item', active === s.id && 'on')}
          onClick={e => {
            e.preventDefault();
            const page = document.querySelector<HTMLElement>('.page-settings');
            const target = document.getElementById(`set-${s.id}`);
            if (!page || !target) return;
            useSpy.setState({ active: s.id, lock: true });
            setTimeout(() => useSpy.setState({ lock: false }), 900);
            page.scrollTo({ top: target.offsetTop - 20, behavior: reduceMotion() ? 'auto' : 'smooth' });
            replay(target, 'flash');
          }}
        >
          <Icon name={s.icon} />
          <span>{s.title}</span>
        </a>
      ))}
    </nav>
  );
}
