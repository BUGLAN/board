/* 胶囊输入框：一个值一个胶囊（MCP 的参数、供应商的模型）。
   回车、逗号加一个（allowSpace 为假时空格也算），粘贴多行拆开，退格删最后一个，× 删掉；离开输入框时没回车的也加上 */
import { useRef, useState } from 'react';
import { cx } from '../../lib/util';
import { Icon } from '../Icon';

/** 把输入拆成几个值：换行、逗号（allowSpace 为假时空白也算）分开，去掉空的 */
export const splitChips = (raw: string, allowSpace = false) =>
  raw
    .split(allowSpace ? /[\n,，]/ : /[\n,，\s]/)
    .map(s => s.trim())
    .filter(Boolean);

export function ChipInput({ value, onChange, placeholder, allowSpace, label, unique = !allowSpace }: { value: string[]; onChange: (v: string[]) => void; placeholder: string; allowSpace?: boolean; label: string; unique?: boolean }) {
  const [text, setText] = useState('');
  const ref = useRef<HTMLInputElement>(null);
  const add = (raw: string) => {
    const parts = splitChips(raw, allowSpace);
    if (parts.length) onChange([...value, ...(unique ? parts.filter((p, i) => !value.includes(p) && parts.indexOf(p) === i) : parts)]);
    setText('');
  };
  return (
    <div className={cx('chip-in input')} onClick={() => ref.current?.focus()}>
      {value.map((v, i) => (
        <span key={`${v}:${i}`} className="chip-in-v">
          {v}
          <button type="button" aria-label={`去掉 ${v}`} onClick={() => onChange(value.filter((_, j) => j !== i))}>
            <Icon name="close" />
          </button>
        </span>
      ))}
      <input
        ref={ref}
        value={text}
        aria-label={label}
        placeholder={value.length ? '' : placeholder}
        spellCheck={false}
        autoComplete="off"
        onChange={e => setText(e.target.value)}
        onPaste={e => {
          const t = e.clipboardData.getData('text');
          if (/[\n,]/.test(t)) {
            e.preventDefault();
            add(t);
          }
        }}
        onKeyDown={e => {
          if (e.key === 'Enter' || e.key === ',' || (!allowSpace && e.key === ' ')) {
            e.preventDefault();
            add(text);
          } else if (e.key === 'Backspace' && !text && value.length) {
            onChange(value.slice(0, -1));
          }
        }}
        onBlur={() => text.trim() && add(text)}
      />
    </div>
  );
}
