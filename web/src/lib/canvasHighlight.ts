/* 侧栏项目树悬停时，画布上对应的节点和连线高亮。单独成文件：侧栏不用为了它加载整个画布（画布按需加载） */
let setter: ((id: string | null) => void) | null = null;

/** 画布挂载时登记，返回注销函数 */
export function registerCanvasHighlight(fn: (id: string | null) => void) {
  setter = fn;
  return () => {
    if (setter === fn) setter = null;
  };
}

export const canvasHighlight = (id: string | null) => setter?.(id);
