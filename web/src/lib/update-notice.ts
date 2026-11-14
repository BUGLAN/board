/* ==========================================================================
   自动检查之后的提示决策（纯函数，便于测试；调用方在 update-runner.ts）
   - 只有发现新版本时才打扰：弹「发现新版本」确认框
   - 已是最新、检查失败（网络错误、清单不对、旧版外壳的 unsupported）都静默；结果照样在 store 里，设置页手动检查能看到
   ========================================================================== */
import type { CheckState } from './update';

export type AutoNotice = { action: 'confirm'; version: string } | { action: 'none' };

/** 自动检查完要不要提示：只有 `available` 弹确认框，其余什么都不弹（default/main#327） */
export function autoNotice(state: CheckState): AutoNotice {
  return state.kind === 'available' ? { action: 'confirm', version: state.version } : { action: 'none' };
}
