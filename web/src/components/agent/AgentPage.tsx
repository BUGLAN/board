/* ==========================================================================
   Agent tab（PRD §10.16）：服务器上存一份、和具体机器无关的配置，一节一页
   - 共用配置：模型供应商、MCP 服务器、skills、Git 凭据（hosts/ 下的四个区块，原样用）
   - 自动领取：领取规则、跑多久停（hosts/Sched.tsx）
   - 接入：接入 agent、访问令牌（Access.tsx，从设置页搬来）
   没有机器时照常能进、每一节都能看能改，页面顶部一行提示带「添加机器」
   手机上侧栏不显示：页头「组 · 节」那一行换成选择框
   ========================================================================== */
import type { ReactNode } from 'react';
import { hostsOf } from '../../lib/hosts';
import { useData } from '../../store/data';
import { openHost, setTab, useUI } from '../../store/ui';
import { Select } from '../bits';
import { Icon } from '../Icon';
import { usePollHosts } from '../hosts/bits';
import { GitCredSection } from '../hosts/GitCred';
import { McpSection } from '../hosts/Mcp';
import { ProvidersSection } from '../hosts/Providers';
import { ClaimRules, RunLimits } from '../hosts/Sched';
import { SharedConfig } from '../hosts/Shared';
import { AccessCard, TokensCard } from './Access';
import { GROUPS, goSec, groupOf, secOf, useAgentSec, type SecId } from './AgentNav';

const SEC_OPTIONS = GROUPS.flatMap(g => g.secs.map(s => ({ value: s.id, label: `${g.title} · ${s.title}` })));

export function AgentPage() {
  const ws = useData(s => s.ws);
  const tab = useUI(s => s.tab);
  const hosts = hostsOf(ws);
  const sec = useAgentSec(s => s.sec);
  // 各节里「哪些机器在用 / 同步到哪台」跟着机器的上报走
  usePollHosts(tab === 'agent');

  const body: Record<SecId, ReactNode> = {
    providers: <ProvidersSection hosts={hosts} />,
    mcp: <McpSection hosts={hosts} />,
    skills: <SharedConfig hosts={hosts} />,
    git: <GitCredSection hosts={hosts} />,
    rules: <ClaimRules hosts={hosts} />,
    limits: <RunLimits />,
    access: <AccessCard />,
    tokens: <TokensCard />,
  };
  return (
    <div className="page page-agent" data-page="agent">
      <div className="hp-b ag-b">
        <div className="hp-b-head">
          <div>
            <h1>Agent</h1>
            <p className="ag-where">
              {groupOf(sec).title} · {secOf(sec).title}
            </p>
            {/* 手机上没有侧栏：页头这一行换成选择框，从这里换节 */}
            <Select className="ag-pick" value={sec} options={SEC_OPTIONS} onChange={v => goSec(v as SecId)} aria-label="Agent 的哪一节" />
          </div>
        </div>

        {ws && hosts.length === 0 && (
          <div className="ag-nohost">
            <Icon name="host" />
            <span>还没有机器。这里的配置现在就能改，机器接上之后直接用上。</span>
            <button
              type="button"
              className="btn sm"
              onClick={() => {
                setTab('hosts');
                openHost('new');
              }}
            >
              <Icon name="plus" />
              添加机器
            </button>
          </div>
        )}

        {/* key：换一节时重新挂载，各区块各自取一次数据 */}
        <div key={sec} className="ag-sec" data-sec={sec}>
          {body[sec]}
        </div>
      </div>
    </div>
  );
}
