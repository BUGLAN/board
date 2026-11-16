# board host 心跳与实时推送调研

2026-11-16 调研。范围是 `board host` 的在线判定和任务下发链路；结论只讨论传输机制，不改协议代码。

## 结论

**保留现在的「HTTP 心跳 + SSE + 心跳兜底」，不改成 WebSocket。**

- 心跳本身是合适的：它不是单纯的“在线状态”，而是机器的存活租约、任务兜底和离线恢复依据。WebSocket 也会半开、会被 NAT/代理/sleep 截断，仍然需要租约和超时回收，不能拿它替代在线判定。
- SSE 也是合适的：Board 的任务是服务端向一台机器单向通知“有变化”，机器再用普通 HTTP 取权威任务。SSE 已经做到低延迟，而且断线后 `GET /api/hosts/{名字}/work` 和下一次 beat 都能补上，不依赖消息恰好送达。
- 现在的问题不是选错传输层，而是 **beat 做得太重**：每 10 秒同时传完整机器清单、回完整任务和配置，并发起多次 SQLite 写入。当前规模可以接受，机器数量上来后应先拆小心跳、改成按版本取增量，这比换 WebSocket 的收益更直接、风险更低。
- WebSocket 只有在未来需要大量双向实时消息、多个服务端实例、外部存在状态存储时才值得做。它不是当前单服务端 + SQLite + 低频率任务下发的优先项。

## 现状链路

1. `board host` 每 10 秒 `POST /api/hosts/{名字}/beat`，请求里带实例标识、系统、版本、本机 agent、启动环境、工作空间、目录、正在跑的派发和资源采样（`crates/board-cli/src/host.rs:2073`、`crates/board-core/src/hosts.rs:3326`）。
2. 服务端在同一个写事务里更新机器、资源、在线时间，给正在跑的派发名字发 agent 心跳，并更新卡片心跳；返回这台机器的完整任务（`crates/board-core/src/hosts.rs:3326`、`crates/board-cli/src/host.rs:2534`）。
3. 同一时间，`board host` 用访问令牌订阅 `GET /api/stream`。服务端每次有机器或派发变化就广播；host 收到后立即 `GET /api/hosts/{名字}/work`，所以正常路径不是等 10 秒（`crates/board-cli/src/host.rs:3228`、`3293`、`crates/board-server/src/lib.rs:1257`）。
4. SSE 服务端每 15 秒发 keep-alive，客户端单次读超时 45 秒；连接断开后 0.5 秒起指数退避，最长 10 秒重连（`crates/board-server/src/lib.rs:1302`、`crates/board-cli/src/backend.rs:296`、`crates/board-cli/src/host.rs:3235`）。
5. 服务端每秒扫描一次。机器超过 50 秒没有 beat 就算离线；机器上的在跑派发结束为「机器掉线」，卡片按同样 50 秒规则释放（`crates/board-core/src/hosts.rs:379`、`5667`）。
6. 机器恢复后，如果卡片还没被别人领走，`dispatch_revive` 会接回原来的派发；被别人领走时不再接回（`crates/board-core/src/hosts.rs:5009`）。

任务下发实际有两条路：

```text
界面/CLI 派发
  -> SQLite 写事务、hosts rev +1
  -> 广播循环（≤ 250ms）通过 SSE 发 hosts 版本
  -> board host 收到版本，GET /work
  -> 启动 agent

SSE 断了 / 漏了一条
  -> 下一次 beat（≤ 10 秒）的响应直接带任务
```

因此“最长 10 秒延迟”只适用于 SSE 失效后的兜底路径；正常路径由 SSE 唤醒，服务端代码按不超过 300ms 的目标推送，host 主循环每 200ms 检查一次。折算上网络和 `GET /work`，正常通常在一秒内。

## 成本与数据

下面是可以直接从代码路径算出的量，不是生产监控数据：

- 每台机器固定每分钟 6 次 beat，即每天 8,640 次 HTTP 请求。100 台机器约 10 次/秒，1,000 台约 100 次/秒。
- 没有在跑的派发时，每次 beat 至少两次 `UPDATE hosts`（机器行和资源行），即每台每天 17,280 次写入；有 1 张卡在跑时，再加 1 次 agent、1 次卡片、1 次 `card_runs`，约每台每天 43,200 次写入（`crates/board-core/src/hosts.rs:3326`、`crates/board-core/src/engine.rs:1177`）。
- 每次 beat 都重新读取并序列化本机 agent、启动环境、目录、本机 skill；响应还包含运行/排队派发、skills、MCP、供应商和配置哈希。报文大小随机器配置线性增长，不随任务数量保持常数。
- SQLite 使用 WAL、`busy_timeout`，但同一进程仍然只有一个写事务入口（`crates/board-core/src/store.rs:284`、`542`）。把每次 beat 放在一个写事务里，意味着机器数增长时，心跳、卡片、评论和派发结束会竞争同一个写者。

这些量在现在的自托管规模下很小，不足以成为迁移协议的理由；但它们说明 beat 不适合继续无界增长，应先做减法和观测。

## 具体问题

1. **50 秒固定门槛会把休眠和短时断网判成掉线。** 笔记本合盖、系统休眠或网络切换超过 50 秒，进程和 SSE 都会停；服务端释放卡片，其他机器可能接走。恢复后 `dispatch_revive` 只能在没有别人领取时接回，否则原 agent 会被结束，最坏情况是两台机器都做了同一张卡的一部分。WebSocket 不能解决这个问题，它同样会在休眠时断开。
2. **beat 是“胖心跳”。** 每 10 秒发送完整清单、返回完整配置，并在数据库里做多处更新；资源趋势和历史 skill 也在同一个请求里。当前规模能跑，但带宽、序列化和 SQLite 写锁都随机器数线性放大。
3. **离线检测慢于任务下发。** 正常任务通过 SSE 是亚秒级，但机器无响应后要等满 50 秒才释放，卡片在这段时间不可用。这是可靠性和回收速度的取舍，应该显式做成两级：先显示“可疑/失联”，再延迟回收；不能只靠 WebSocket 的 close 事件，因为半开连接和进程被 kill 时 close 往往不会及时到达。
4. **没有抖动和自适应退避。** 所有 host 从同一时间启动时会每 10 秒一起打服务端；多个 host 同时重连也会形成尖峰。心跳间隔应加少量随机抖动；机器多时应允许服务端下发或协商间隔。
5. **当前 SSE 广播是单进程内存广播。** `broadcast::channel(256)` 只在当前服务端进程里有效；服务重启时连接断开，但数据库心跳和 `resume_after_downtime` 能把停机时间排除。以后如果部署多个服务端实例，SSE 必须换成跨实例 pub/sub；WebSocket 一样需要，而且还要处理连接粘性和会话状态。
6. **代理和 NAT 不是当前瓶颈。** 10 秒 beat 和 15 秒 SSE keep-alive 都比 Cloudflare 的代理空闲/读超时短得多；Cloudflare Tunnel 对 SSE 需要 `Content-Type: text/event-stream`，Board 已经由 SSE 响应设置该类型，并加 `X-Accel-Buffering: no`（`crates/board-server/src/lib.rs:1298`、`docs/deploy.md:114`）。

## 方案对比

| 方案 | 和 Board 的匹配度 | 延迟 | 实现/运维成本 | 迁移风险 | 判断 |
|---|---|---|---|---|---|
| HTTP 心跳（现状） | 高：无状态、令牌鉴权、穿过 NAT/Cloudflare/Tunnel 都简单；可做租约和兜底 | 单独用时任务最多 10 秒；配合 SSE 正常亚秒级 | 最低；已有重试、实例接回、离线回收 | 无 | 保留，改成轻量心跳 + 版本增量 |
| SSE（现状） | 高：单向服务端通知正是 Board 的任务模型；浏览器原生 `EventSource`，CLI 用普通 HTTP 流 | 服务端变化后通常 <1 秒 | 低；已有 15 秒 keep-alive、45 秒读超时、退避重连 | 无 | 保留为主推送通道 |
| WebSocket | 中：全双工，但 Board 没有持续双向消息需求；仍需心跳、重连、背压和粘性 | 理论最低，实际受网络 RTT 限制 | 高：握手鉴权、帧处理、ping/pong、背压、断开恢复、代理/负载均衡会话粘性、更多依赖 | 高：要同时兼容旧 host 和旧服务端 | 不值得现在迁移 |
| 长轮询 | 中：可做“没有任务就挂起 25–30 秒”；能减少空轮询 | 事件发生时接近即时 | 中：服务端要维护等待者、超时和重连；每个 host 反复占用连接 | 中：要替换 SSE 或增加第二套路径 | 没有比 SSE 更简单，不作为主方案 |
| gRPC streaming | 低到中：类型安全、双向流、二进制高效，适合大规模内部 RPC | 低 | 高：protobuf、HTTP/2 端到端、Cloudflare 兼容性、桌面端/WASM 和 CLI 依赖都更重 | 高 | 当前规模过重，不采用 |
| MQTT / 消息队列 | 低：适合大量设备 fan-out 和离线队列，但 Board 的任务状态已经在 SQLite，broker 会引入第二套一致性来源 | 低 | 最高：部署、鉴权、持久化、监控、重复投递去重 | 很高 | 只有多租户/大规模机器集群才考虑 |
| UDP / QUIC | 低：任务下发要求可靠、有序、可审计；自己实现重传和拥塞控制是重复造 TCP | 低但不确定 | 高：NAT、防火墙、TLS、可观测性都更复杂 | 很高 | 不采用；Tunnel 内部用 QUIC 不等于应用层要用 UDP |

## 推荐

### 现在做

**方案 A：保留混合架构。**

1. 心跳继续承担机器存活租约、离线回收和 SSE 失效时的兜底任务拉取；SSE 只做“有变化”的提示，`GET /work` 始终是权威读取。这样消息丢失、重复、乱序都不会影响正确性。
2. 把 beat 拆成轻量心跳：只报实例、序号、能力/配置摘要和资源摘要；完整 agent/目录/skill 清单只在发生变化或低频快照时上报。响应只带 `hosts_rev` 和是否有任务，需要任务时再 `GET /work`。
3. 给 beat 加 10%–20% 抖动，给失败重连加指数退避和上限；保留 50 秒作为“失联”显示，另设更长的租约回收宽限（例如 90–120 秒），把“界面显示离线”和“释放别人可能还在做的卡”拆开。
4. 先加监控再调参数：beat 成功率与 p95 延迟、SSE 重连次数、任务从写入到 agent 启动的 p95、`lost` 派发数、`dispatch_revive` 次数、SQLite busy/写事务耗时。没有这些数据，不应该凭感觉改 10 秒或 50 秒。

这些是优化，不改变传输协议，也不需要旧 host 升级才能继续工作。

### 什么时候再考虑 WebSocket

只有同时出现下面两类需求才值得另立卡片：

- 需要大量双向、低延迟控制消息（例如服务器持续向 agent 推 token 流、工具调用中间结果，且 SSE 的请求/响应模型不够）；
- 已经要部署多个服务端实例，存在跨实例在线状态和任务路由，此时需要外部 pub/sub、连接粘性或无状态会话恢复。

未来的兼容迁移应当是这样，而不是直接替换：

1. 保留 `/api/hosts/{名字}/beat` 和 `/api/hosts/{名字}/work`，把传输能力放进 beat 的能力字段；旧 host 永远走 HTTP + SSE。
2. 新 host 另开 `/api/host-ws`，携带 token、实例标识和协议版本；服务端只负责推送带 `seq`/`hosts_rev` 的提示，任务仍以 `GET /work` 的结果为准。
3. 两端都支持“WebSocket 不可用就回退 SSE”，断线重连带 `since` 或版本号；服务端保留心跳租约，不把 WebSocket 连接本身当作永久在线。
4. 先在少量机器上双跑并比较指标，至少保留一个完整发版周期；确认旧 host 数量为零后，才删除 SSE/beat 兼容路径。任何时候都不改已有 token 和任务语义。

## 依据

代码：

- `crates/board-cli/src/host.rs:2073`：完整 `HostReport` 的组成。
- `crates/board-cli/src/host.rs:3221`：SSE 订阅；`3245`：主循环；`3282`：beat；`3293`：`/work` 兜底。
- `crates/board-core/src/hosts.rs:379`：`HOST_GAP_MS`；`3326`：`host_beat`；`5667`：机器掉线扫描。
- `crates/board-server/src/lib.rs:1257`：SSE；`1302`：15 秒 keep-alive 和 `X-Accel-Buffering`。
- `crates/board-cli/src/backend.rs:296`：SSE 客户端读超时和重连行为。
- `crates/board-core/src/store.rs:542`：单写事务；`crates/board-core/src/engine.rs:1177`：卡片心跳写入。

外部资料：

- [Cloudflare Connection limits](https://developers.cloudflare.com/fundamentals/reference/connection-limits/)：普通代理链路 `Proxy Read Timeout` 125 秒、`Proxy Idle Timeout` 900 秒；Tunnel 两层超时取较短者。
- [Cloudflare WebSockets](https://developers.cloudflare.com/network/websockets/)：支持 WebSocket，但空闲会被关闭，需要客户端 keepalive；Cloudflare 发布新代码时可能重启服务器并断开连接；负载均衡源站需要会话粘性。
- [Cloudflare Tunnel common errors](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/troubleshoot-tunnels/common-errors/)：Tunnel 默认缓冲流式响应，只有 `Content-Type: text/event-stream` 才按到达流式转发；长连接也可能因维护或重启断开。
- [SQLite WAL](https://www.sqlite.org/wal.html)：WAL 允许读并发，但写者仍只有一个；这决定了心跳写入不能无限随机器数增加。
- [MDN：Using server-sent events](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events)：SSE 的重连和 `retry` 语义；Board 的 CLI 另外实现了退避重连。

## 最终建议

当前不建议更换传输协议。保留心跳 + SSE，把心跳从“全量同步”降为“存活租约 + 版本提示”，用指标驱动超时和回收参数；WebSocket 留到出现明确的双向高频需求和多实例架构时再做兼容迁移。
