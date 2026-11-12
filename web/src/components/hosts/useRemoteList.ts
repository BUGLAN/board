/* ==========================================================================
   共用配置各区块共用的「读一份远程列表」
   - 取到之前 list 是 null（界面画「正在读取…」），取不到时 err 是原因，取到了清掉上一次的错误
   - 机器有变化（服务端推来新的 ws.hosts）时跟着重新取一次；上传、删除之后调 refresh() 手动重新取
   - skills 共用库、MCP 服务器、agent 模板三个区块都用它，各自只留上传、删除、编辑这些业务
   ========================================================================== */
import { useEffect, useState } from 'react';
import type { HostInfo } from '../../lib/types';

export function useRemoteList<T>(load: () => Promise<T>, hosts: HostInfo[] | undefined) {
  const [list, setList] = useState<T | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const refresh = async () => {
    try {
      setList(await load());
      setErr(null);
    } catch (e) {
      setErr(String((e as Error)?.message ?? e));
    }
  };
  useEffect(() => {
    void refresh();
  }, [hosts]);
  return { list, err, refresh };
}
