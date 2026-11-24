/* 一条领取规则里的标签（PRD §9.1「领取规则」）：从机器详情挪出来，Agent tab「领取规则」里用 */
import type { Label } from '../../lib/types';
import { cx } from '../../lib/util';
import { openPicker, updatePicker, useUI } from '../../store/ui';
import { LabelChip } from '../bits';
import { Icon } from '../Icon';

/** 一条规则里的标签：和卡片详情的标签一行一样——标签胶囊（项目里的颜色，× 去掉）加标签按钮，点开同一个标签选择器，
    勾一个就应用一个。项目里已经没有的标签原样保留 */
export function LabelSet({ value, onChange, labels }: {
  value: string[];
  onChange: (v: string[]) => void | Promise<unknown>;
  labels: Label[];
}) {
  const byName = (n: string, ls = labels) => ls.find(l => l.name.toLowerCase() === n.toLowerCase());
  const ids = (names: string[]) => names.map(n => byName(n)).filter((l): l is Label => !!l).map(l => l.id);
  const open = (anchor: HTMLElement) => {
    // 不在列表里的（项目里已经没有的）标签选择器里没有，原样留在规则里
    const kept = value.filter(n => !byName(n));
    let applied = value;
    let latest = ids(value);
    // 勾一个应用一个。机器设置保存完之前不能再改（BUSY）：连点时排队，轮到时按选择器最新的勾选保存，中间几次合并成一次
    let chain: Promise<unknown> = Promise.resolve();
    openPicker({
      rect: anchor.getBoundingClientRect(),
      project: '',
      labels,
      selected: latest,
      onToggle: () => {
        latest = useUI.getState().picker?.selected ?? latest;
        chain = chain.then(async () => {
          const ls = labels;
          const next = [...ls.filter(l => latest.includes(l.id)).map(l => l.name), ...kept];
          if (!next.length) {
            // 规则至少要有一个标签：一个都不勾时不改，勾回原样
            updatePicker(applied.map(n => byName(n, ls)).filter((l): l is Label => !!l).map(l => l.id));
            return;
          }
          if (next.length === applied.length && next.every(n => applied.includes(n))) return;
          applied = next;
          await onChange(next);
        });
      },
    });
  };
  return (
    <span className="hp-lset">
      {value.map(n => {
        const l = byName(n);
        const off = !l;
        return (
          <span key={n} className={cx('hp-lset-chip', off && 'off')} title={off ? '项目里没有这个标签' : undefined}>
            <LabelChip label={l ?? { id: -1, name: n, color: 'var(--text-4)' }} onRemove={() => onChange(value.filter(x => x !== n))} />
          </span>
        );
      })}
      <button type="button" className="lb-add" aria-label="添加标签" data-label-picker onClick={e => open(e.currentTarget)}>
        <Icon name="tag" />
        {value.length ? '' : '选标签'}
      </button>
    </span>
  );
}

