/* ==========================================================================
   服务端升级之后还开着旧页面时，点「刷新页面」之前的一步（default/main#368）
   - 先记下「还有没确认的写操作」（保存中、发送中），刷新之后第一次挂载时提示一句：那几笔没发出去，内容在草稿里
   - 有草稿的输入框（卡片详情的描述、标题，新建卡片）刷新后会自己恢复，这里不重复存
   - 没东西丢就不提示：正常刷新安静地过去
   ========================================================================== */
import { useEffect } from 'react';
import { pref } from '../../lib/util';
import { useData } from '../../store/data';
import { toast } from '../../store/ui';

/** 点刷新的那一刻还有几笔没确认的写操作（0 = 没有） */
const KEY = 'stale.pending';

export function MigrationGuard() {
  // 刷新之前记下还有没有没确认的写操作（beforeunload 只有页面真的要走了才跑：这时候不拦，只是记下来）
  useEffect(() => {
    const note = () => pref.set(KEY, Object.keys(useData.getState().saving).length > 0 || Object.keys(useData.getState().sending).length > 0);
    window.addEventListener('beforeunload', note);
    return () => window.removeEventListener('beforeunload', note);
  }, []);

  // 刷新之后：上一次是被「刷新页面」换掉的、而且当时有没确认的改，就说一句
  useEffect(() => {
    const pending = pref.get<boolean>(KEY, false);
    pref.set(KEY, null);
    if (pending) {
      toast({
        type: 'info',
        title: '已经刷新到新版本',
        text: '刚才还在保存的改动没有发出去：填过的内容在草稿里，重新编辑会恢复',
        duration: 8000,
      });
    }
  }, []);

  return null;
}
