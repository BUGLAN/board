import { describe, expect, it } from 'vitest';
import { judge, markers } from './granularity';

describe('拆解前的粒度预检（default/main#306）', () => {
  it('一句话、单点操作：判偏小，不拆', () => {
    for (const body of ['', '把设置页的按钮圆角改成 8px', '登录页的 logo 换成新的那张图。']) {
      expect(judge('界面：设置页、登录页', body).size).toBe('small');
    }
  });

  it('同一件事写了「和」，不算几件事', () => {
    expect(judge('导出', '导出按钮和它旁边那一行提示文案要一起改，读起来别重复').size).toBe('small');
  });

  it('列了几项、分了几节、几句能各自跟进的话：偏大，要拆', () => {
    expect(judge('界面：设置页', '- 圆角改成 8px\n- logo 换成新图').size).toBe('large');
    expect(judge('导出', '## 导出 CSV\n要加这个。\n## 导出 Excel\n也要加。').size).toBe('large');
    expect(judge('一堆事', '第一件事是改导出；第二件事是加个筛选；第三件事是把样式统一；第四件事是补测试。').size).toBe('large');
  });

  it('几行并列的目标也算偏大', () => {
    expect(judge('导出', '要能导出 CSV，并且补上 Excel，另外再加一份汇总。').size).toBe('large');
    expect(markers('甲以及乙并且丙').goals).toBe(2);
    expect(markers('甲和乙').goals).toBe(0);
  });

  it('长但只是一件事：偏小，不按长度硬拆', () => {
    const one = '导出的 CSV 第一行写表头，后面一行一条记录，文件名带上时间，逗号用引号括起来，换行按系统来，编码用 UTF-8';
    expect(judge('导出 CSV', one).size).toBe('small');
    // 真的写成了几件事
    expect(judge('导出', `${one}。另外还要能导出 Excel，以及一份汇总。`).size).toBe('large');
  });

  it('标题也参与判断：描述为空时看标题', () => {
    expect(judge('把按钮圆角改成 8px', '').size).toBe('small');
    expect(judge('单点操作', '').why).toBeTruthy();
  });
});
