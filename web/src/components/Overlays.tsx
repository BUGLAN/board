/* ==========================================================================
   浮层：命令面板、确认框、表单弹窗、⋯ 菜单、标签选择器、标签管理、图片放大、toast
   各自在 overlays/ 下；这里只按层叠顺序把它们挂出来（后挂的在上面）
   ========================================================================== */
import { CtxMenu } from './overlays/CtxMenu';
import { ConfirmDialog, FormModal } from './overlays/Dialogs';
import { LabelManager, LabelPicker } from './overlays/Labels';
import { Lightbox } from './overlays/Lightbox';
import { Palette } from './overlays/Palette';
import { Toasts } from './overlays/Toasts';

export { usePresence } from './overlays/presence';

export function Overlays() {
  return (
    <>
      <Palette />
      <FormModal />
      <LabelManager />
      <ConfirmDialog />
      <CtxMenu />
      <LabelPicker />
      <Lightbox />
      <Toasts />
    </>
  );
}
