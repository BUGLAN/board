/* 浮层共用：打开时先挂载再加 .on（入场过渡），关闭时先去掉 .on 再卸载 */
import { useEffect, useState } from 'react';

/** 打开时先挂载再加 .on（入场过渡）；关闭时先去掉 .on 再卸载 */
export function usePresence(open: boolean, ms = 240) {
  const [mounted, setMounted] = useState(open);
  const [on, setOn] = useState(false);
  useEffect(() => {
    if (open) {
      setMounted(true);
      const r = requestAnimationFrame(() => requestAnimationFrame(() => setOn(true)));
      return () => cancelAnimationFrame(r);
    }
    setOn(false);
    const t = setTimeout(() => setMounted(false), ms);
    return () => clearTimeout(t);
  }, [open, ms]);
  return { mounted, on };
}

/* ============================ 命令面板 ============================ */
