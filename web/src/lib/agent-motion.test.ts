import { afterEach, describe, expect, it, vi } from 'vitest';
import { ACT, ACT_MS, ENTER, LOOP_MS, loopAgentMotion, loopFrames, playAgentMotion, type AgentMotion } from './agent-motion';
import { AGENT_IDS, agentMark } from './hosts';

const MOTIONS = Object.keys(ACT) as AgentMotion[];

/** 假的元素：记下 animate 的参数，getAnimations 返回已经播过、还在播的 */
function fakeEl() {
  const played: { frames: Keyframe[]; opts: KeyframeAnimationOptions; anim: { id: string; playState: string; cancel: () => void } }[] = [];
  const el = {
    animate: vi.fn((frames: Keyframe[], opts: KeyframeAnimationOptions) => {
      const anim = { id: '', playState: 'running', cancel: vi.fn() };
      played.push({ frames, opts, anim });
      return anim;
    }),
    getAnimations: () => played.map(p => p.anim),
  };
  return { el: el as unknown as Element, played };
}

afterEach(() => {
  delete document.documentElement.dataset.motion;
});

describe('agent logo 的签名动作（default/main#357）', () => {
  it('认得出的 agent 都有自己的 logo 和动作，动作各不相同', () => {
    const ms = AGENT_IDS.map(id => agentMark(id).motion);
    for (const id of AGENT_IDS) {
      expect(agentMark(id).icon).toBeTruthy();
      expect(MOTIONS).toContain(agentMark(id).motion);
    }
    expect(new Set(ms).size).toBe(ms.length);
  });

  it('做一次的动作从原样开始、回到原样，循环播不会越动越偏；进场版从看不见落到原样', () => {
    for (const m of MOTIONS) {
      expect(ACT[m][0].transform).toBe('none');
      expect(ACT[m].at(-1)!.transform).toBe('none');
      expect(ENTER[m][0].opacity).toBe(0);
      expect(ENTER[m].at(-1)!.transform).toBe('none');
    }
  });

  it('loopFrames：把一次动作压进循环开头，其余时间停在原样；不改原来的关键帧', () => {
    const frames = ACT.hop;
    const copy = JSON.stringify(frames);
    const out = loopFrames(frames, 800, 5000);
    const k = 800 / 5000;
    expect(out).toHaveLength(frames.length + 1);
    expect(out[0].offset).toBe(0);
    // 中间的关键帧按比例压缩
    expect(out[2].offset).toBeCloseTo((frames[2].offset as number) * k);
    // 动作做完（k）之后一直停在原样到循环结束
    expect(out.at(-2)!.offset).toBeCloseTo(k);
    expect(out.at(-1)).toEqual({ transform: 'none', offset: 1 });
    expect(out.every((f, i) => i === 0 || (f.offset as number) >= (out[i - 1].offset as number))).toBe(true);
    expect(JSON.stringify(frames)).toBe(copy);
    // 一次动作比循环还长：不再补停顿
    expect(loopFrames(frames, 6000, 5000)).toHaveLength(frames.length);
  });

  it('hover 做一次、出现用进场版；上一次没播完不叠着播', () => {
    const { el, played } = fakeEl();
    expect(playAgentMotion(el, 'swim', 'act')).not.toBeNull();
    expect(played[0].frames).toBe(ACT.swim);
    expect(played[0].opts.duration).toBe(ACT_MS);
    expect(playAgentMotion(el, 'swim', 'act')).toBeNull();
    played[0].anim.playState = 'finished';
    expect(playAgentMotion(el, 'swim', 'enter')).not.toBeNull();
    expect(played[1].frames).toBe(ENTER.swim);
  });

  it('循环：每 LOOP_MS 一次、一直播，delay 用来错开', () => {
    const { el, played } = fakeEl();
    loopAgentMotion(el, 'hop', 1250);
    expect(played[0].opts).toMatchObject({ duration: LOOP_MS, iterations: Infinity, delay: 1250 });
    expect(played[0].frames.at(-1)).toEqual({ transform: 'none', offset: 1 });
  });

  it('减弱动效、没有动作、没有元素、浏览器不支持时什么都不播', () => {
    const { el, played } = fakeEl();
    expect(playAgentMotion(null, 'hop', 'act')).toBeNull();
    expect(playAgentMotion(el, undefined, 'act')).toBeNull();
    expect(playAgentMotion({} as Element, 'hop', 'act')).toBeNull();
    document.documentElement.dataset.motion = 'reduced';
    expect(playAgentMotion(el, 'hop', 'act')).toBeNull();
    expect(loopAgentMotion(el, 'hop')).toBeNull();
    expect(played).toHaveLength(0);
  });
});
