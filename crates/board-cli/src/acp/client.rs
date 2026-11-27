use super::merge::chunk;
use super::quota::{quota_of, rate_of};
use super::{AuthMode, AUTH_WAIT, EXIT_CANCELLED, EXIT_LIMIT, EXIT_OK, EXIT_PROTOCOL, EXIT_QUOTA, PROTOCOL};
use board_core::hosts::{AgentAuth, LocalModel, LocalModels, Quota, AUTH_ACCOUNT, AUTH_NONE, AUTH_UNKNOWN};
use serde_json::{json, Value};
use std::collections::HashMap;

/* ------------------------------ 协议 ------------------------------ */

/// 等着回应的请求
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    Init,
    Provider,
    New,
    /// 对话：接回会话（`session/resume`、`session/load`）
    Resume,
    Load,
    Mode,
    Prompt,
    Close,
    /// `auth/status`（#667）
    Auth,
    /// `session/options`（#666）：派发时认 configId，列模型时取模型下拉
    Options,
    /// 列模型：不发提示词的会话
    ProbeNew,
    /// 列模型：切到一个模型读它的强度
    ProbeModel,
}


/// 协商到的协议版本
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Proto {
    V1,
    V2,
}


/// 这一次运行怎么结束的
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    /// 这一轮做完了：v1 是 prompt 的回应，v2 是 `idle` 的 `stopReason`
    Stopped(String),
    /// 握手、prompt 出错，v2 这一轮出错（原因）
    Failed(String),
    /// 这一轮因为订阅账号的额度用完出错（原因；额度见 [`Client::quota`]）
    Quota(String),
}


/// 客户端的状态：收一帧、给出要发的帧。不碰进程和文件，单元测试直接喂帧
pub(crate) struct Client {
    next: i64,
    pending: HashMap<i64, Pending>,
    session: Option<String>,
    cwd: String,
    prompt: String,
    mode: Option<String>,
    sets: Vec<(String, String)>,
    /// initialize 之后、session/new 之前要发的 `providers/set` 参数
    provider: Option<Value>,
    /// session/new 之后、prompt 之前要发的配置请求（一个一个发）
    queue: Vec<(&'static str, Value)>,
    /// 这一轮怎么结束的；还在干活（v2 报了 `running`）时为空
    pub outcome: Option<Outcome>,
    /// 已经发了 session/prompt（之后才算「进行中」，取消才有意义）
    prompting: bool,
    /// 握手协商到的版本（握手完成前当 v1）
    pub proto: Proto,
    /// v2：最近一次报的后台工作有几项；`None` 是 agent 不报（不知道有没有），不是没有
    pub background: Option<usize>,
    /// 最近一次看到的额度（Claude 的 `usage_update` 带的）；额度用完时 `exhausted`，带上重置时间
    pub quota: Option<Quota>,
    /// 开会话前问不问身份（#667）
    auth_mode: AuthMode,
    /// agent 自己用的身份：`auth/status` 的回应或推来的 `_auth/status_update`
    pub auth: Option<AgentAuth>,
    /// 在等 agent 推身份（只公布了 `_meta.authStatus` 的，或者 `auth/status` 的回应里看不出种类的）
    pub waiting_auth: bool,
    /// agent 会推 `_auth/status_update`（公布了 `_meta.authStatus`）
    auth_push: bool,
    /// agent 公布了 `session.options`（Session options RFD）：认 `session/options`、开会话时认 `configOptions` 初始值（#666）
    can_options: bool,
    /// 开会话时带上的初始值：（configId、类别、值），回应里核对
    initial: Vec<(String, String, String)>,
    /// 列模型（`--auth probe`）：结果
    pub models: Option<LocalModels>,
    /// 列模型：模型那一项的 configId、还没切的模型、正在切的模型
    probe_model_id: String,
    probe_queue: Vec<String>,
    probe_current: String,
    /// 列模型做完了（或做不了）
    pub probe_done: bool,
    /// 对话模式：配置切完后不发提示词，等 [`Client::ask`]
    chat: bool,
    /// 对话：要接回的会话
    resume: Option<String>,
    /// 已经发了接回请求的会话（回应里不一定带 sessionId）
    pending_resume: Option<String>,
    /// agent 公布的能力：接回会话（`sessionCapabilities.resume`）、载入会话（`loadSession`，会回放历史）
    can_resume: bool,
    can_load: bool,
    /// `session/load` 正在回放历史：这些更新不记日志（之前的对话已经记过了）
    pub replaying: bool,
    /// 对话：会话开好、配置切完，可以发提示词了
    pub ready: bool,
    /// 对话：接回会话的结果（给 board host 看的一句话）；开了新会话时说明为什么
    pub note: Option<String>,
    /// 查额度（`--usage`）：本地命令的名字（不带 `/`）；开会话后 agent 公布的命令里没有它就不发
    usage_cmd: Option<String>,
    /// 查额度：agent 回的文字
    pub usage_text: String,
}

impl Client {
    pub(crate) fn new(cwd: &str, prompt: &str, mode: Option<&str>) -> Client {
        Client {
            next: 0,
            pending: HashMap::new(),
            session: None,
            cwd: cwd.into(),
            prompt: prompt.into(),
            mode: mode.map(str::to_string),
            sets: vec![],
            provider: None,
            queue: vec![],
            outcome: None,
            prompting: false,
            proto: Proto::V1,
            background: None,
            quota: None,
            auth_mode: AuthMode::Skip,
            auth: None,
            waiting_auth: false,
            auth_push: false,
            can_options: false,
            initial: vec![],
            models: None,
            probe_model_id: String::new(),
            probe_queue: vec![],
            probe_current: String::new(),
            probe_done: false,
            chat: false,
            resume: None,
            pending_resume: None,
            can_resume: false,
            can_load: false,
            replaying: false,
            ready: false,
            note: None,
            usage_cmd: None,
            usage_text: String::new(),
        }
    }


    /// 查额度（`--usage`）：开会话后发这条本地命令，收下它回的文字
    pub(crate) fn with_usage(mut self, cmd: &str) -> Client {
        self.prompt = cmd.to_string();
        self.usage_cmd = Some(cmd.trim_start_matches('/').to_string());
        self
    }


    /// 机器对话：提示词由 [`Client::ask`] 一条一条发；有 `resume` 时先接回这个会话
    pub(crate) fn for_chat(mut self, resume: Option<String>) -> Client {
        self.chat = true;
        self.resume = resume;
        self
    }


    /// 会话标识（开好、接回之后才有）
    pub(crate) fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }


    /// 对话：发一条提示词（会话开好了、这一轮没在跑时）
    pub(crate) fn ask(&mut self, text: &str) -> Vec<Value> {
        if !self.ready || (self.prompting && self.outcome.is_none()) {
            return vec![];
        }
        self.outcome = None;
        self.send_prompt(text)
    }


    /// 对话：这一轮在跑
    pub(crate) fn busy(&self) -> bool {
        self.prompting && self.outcome.is_none()
    }


    /// 会话开始后要切的配置（`--set`）
    pub(crate) fn with_sets(mut self, sets: Vec<(String, String)>) -> Client {
        self.sets = sets;
        self
    }


    /// 握手后、开会话前换上的供应商（`providers/set` 的参数）
    pub(crate) fn with_provider(mut self, provider: Option<Value>) -> Client {
        self.provider = provider;
        self
    }


    /// 开会话前问身份（`--auth`）
    pub(crate) fn with_auth(mut self, mode: AuthMode) -> Client {
        self.auth_mode = mode;
        self
    }


    /// 握手完、身份也问完（或不用问）之后：换供应商，或开会话
    fn after_init(&mut self) -> Vec<Value> {
        match self.provider.take() {
            Some(p) => vec![self.request("providers/set", p, Pending::Provider)],
            None => self.new_session(),
        }
    }


    /// 问到身份（或问不出来）之后：`only` 到此为止；`subscription` 不是订阅就出错结束，不开会话
    fn after_auth(&mut self) -> Vec<Value> {
        match self.auth_mode {
            AuthMode::Only => vec![],
            AuthMode::Probe => self.probe_options(),
            AuthMode::Subscription => {
                let a = self.auth.clone().unwrap_or_else(|| AgentAuth { kind: AUTH_UNKNOWN.into(), ..Default::default() });
                if a.kind == AUTH_ACCOUNT {
                    return self.after_init();
                }
                self.outcome = Some(Outcome::Failed(format!("按「官方订阅」派发，agent 用的不是订阅：{}，没有开会话", a.words())));
                vec![]
            }
            AuthMode::Skip => self.after_init(),
        }
    }


    /// 等 agent 推身份的时候过了 [`AUTH_WAIT`]：按问不出来算
    pub(crate) fn auth_timeout(&mut self) -> Vec<Value> {
        if !self.waiting_auth {
            return vec![];
        }
        self.waiting_auth = false;
        self.auth = Some(AgentAuth { kind: AUTH_UNKNOWN.into(), label: format!("{} 秒内没报身份", AUTH_WAIT.as_secs()), ..Default::default() });
        self.after_auth()
    }


    fn new_session(&mut self) -> Vec<Value> {
        // 对话接回会话：agent 会 `session/resume` 就用它（不回放历史），只会 `session/load` 就用它（回放的历史不记日志），
        // 都不会时开新会话、说明之前的上下文没了
        if let Some(id) = self.resume.take() {
            let params = json!({ "sessionId": id, "cwd": self.cwd, "mcpServers": [] });
            self.pending_resume = Some(id.clone());
            if self.can_resume {
                return vec![self.request("session/resume", params, Pending::Resume)];
            }
            if self.can_load {
                self.replaying = true;
                return vec![self.request("session/load", params, Pending::Load)];
            }
            self.note = Some("agent 不支持接回会话（session/resume、session/load）：开了新会话，之前的对话它不记得了".into());
        }
        // 模型、强度当开会话的初始值带上（#666）：先 session/options 认出 configId
        if self.can_options && self.proto == Proto::V2 && self.sets.iter().any(|(k, _)| k.starts_with('@')) && self.initial.is_empty() {
            return vec![self.request("session/options", json!({ "cwd": self.cwd }), Pending::Options)];
        }
        let mut params = json!({ "cwd": self.cwd, "mcpServers": [] });
        let initial: Vec<Value> = self.initial.iter().filter(|(id, _, _)| !id.is_empty()).map(|(id, _, v)| json!({ "configId": id, "type": "id", "value": v })).collect();
        if !initial.is_empty() {
            params["configOptions"] = Value::Array(initial);
        }
        vec![self.request("session/new", params, Pending::New)]
    }


    /// 列模型（#666）：先 `session/options`；agent 没公布时列不出来
    fn probe_options(&mut self) -> Vec<Value> {
        if !self.can_options {
            self.models = Some(LocalModels { error: "这个版本的 agent 不支持 session/options，列不出模型".into(), ..Default::default() });
            self.probe_done = true;
            return vec![];
        }
        vec![self.request("session/options", json!({ "cwd": self.cwd }), Pending::Options)]
    }


    /// 列模型：切下一个模型；都切完了就结束
    fn probe_next(&mut self) -> Vec<Value> {
        let Some(s) = self.session.clone() else {
            self.probe_done = true;
            return vec![];
        };
        if self.probe_queue.is_empty() {
            self.probe_done = true;
            return vec![];
        }
        let m = self.probe_queue.remove(0);
        self.probe_current = m.clone();
        let params = set_option(&s, &self.probe_model_id.clone(), &m, self.proto);
        vec![self.request("session/set_config_option", params, Pending::ProbeModel)]
    }


    /// 发下一个配置请求；都发完了发 prompt
    fn step(&mut self) -> Vec<Value> {
        if self.queue.is_empty() {
            if self.chat {
                self.ready = true;
                return vec![];
            }
            return self.prompt();
        }
        let (method, params) = self.queue.remove(0);
        vec![self.request(method, params, Pending::Mode)]
    }


    fn request(&mut self, method: &str, params: Value, what: Pending) -> Value {
        let id = self.next;
        self.next += 1;
        self.pending.insert(id, what);
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }


    /// 握手的第一帧：要 v2，同时带上 v1 的两个字段，只会 v1 的 agent（dsh）照样认得、回 `protocolVersion: 1`
    pub(crate) fn start(&mut self) -> Vec<Value> {
        let info = json!({ "name": "board", "title": "Board", "version": env!("CARGO_PKG_VERSION") });
        let params = json!({
            "protocolVersion": PROTOCOL,
            "info": info,
            "capabilities": {},
            "clientInfo": info,
            "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
        });
        vec![self.request("initialize", params, Pending::Init)]
    }


    /// 收尾：v2 发 `session/close`（停掉会话里还在跑的工作，包括后台的），不等回应；v1 不发
    pub(crate) fn close(&mut self) -> Vec<Value> {
        match (&self.session, self.proto) {
            (Some(s), Proto::V2) => {
                let params = json!({ "sessionId": s });
                vec![self.request("session/close", params, Pending::Close)]
            }
            _ => vec![],
        }
    }


    /// 还在握手（没发 prompt、也没结束）
    pub(crate) fn handshaking(&self) -> bool {
        !self.prompting && self.outcome.is_none()
    }


    /// 取消：prompt 进行中时发 `session/cancel`（通知，没有回应；agent 用 `cancelled` 结束这一轮）
    pub(crate) fn cancel(&mut self) -> Vec<Value> {
        match (&self.session, self.prompting && self.outcome.is_none()) {
            (Some(s), true) => vec![json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": s } })],
            _ => vec![],
        }
    }


    fn prompt(&mut self) -> Vec<Value> {
        let text = self.prompt.clone();
        self.send_prompt(&text)
    }


    /// 续轮：这一轮结束了、卡片还没做完时，在同一个会话里再发一条 prompt，回到「进行中」（取消照样有效）
    pub(crate) fn again(&mut self, text: &str) -> Vec<Value> {
        if self.session.is_none() || self.outcome.is_none() {
            return vec![];
        }
        self.outcome = None;
        self.send_prompt(text)
    }


    fn send_prompt(&mut self, text: &str) -> Vec<Value> {
        let Some(s) = self.session.clone() else { return vec![] };
        self.prompting = true;
        let params = json!({ "sessionId": s, "prompt": [{ "type": "text", "text": text }] });
        vec![self.request("session/prompt", params, Pending::Prompt)]
    }


    /// 收到 agent 的一帧，返回要发回去的帧
    pub(crate) fn on_frame(&mut self, v: &Value) -> Vec<Value> {
        let id = v.get("id").filter(|x| !x.is_null());
        match (v.get("method").and_then(Value::as_str), id) {
            // agent 发来的请求
            (Some(method), Some(id)) => vec![answer(method, id, &v["params"])],
            // 通知：记日志；v2 的 `state_update` 还决定这一轮在不在干活
            (Some(method), None) => {
                if method == "session/update" {
                    self.on_usage(&v["params"]);
                    self.on_usage_text(&v["params"]);
                    if self.proto == Proto::V2 {
                        self.on_update(&v["params"]);
                    }
                }
                // 身份推送（#667）：扩展的 `_auth/status_update`（`{authStatus}`）和 RFD 的 `auth/status_update`（`{state, accounts}`）
                if method == "_auth/status_update" || method == "auth/status_update" {
                    let p = &v["params"];
                    let a = if p["authStatus"].is_object() { auth_of_ext(&p["authStatus"]) } else { auth_of_status(p) };
                    // RFD 的 `unknown` 不盖掉已经知道的
                    if a.kind != AUTH_UNKNOWN || self.auth.is_none() {
                        self.auth = Some(a);
                    }
                    if self.waiting_auth {
                        self.waiting_auth = false;
                        return self.after_auth();
                    }
                }
                vec![]
            }
            // 回应
            (None, Some(id)) => match id.as_i64().and_then(|i| self.pending.remove(&i)) {
                Some(what) => self.on_response(what, v),
                None => vec![],
            },
            (None, None) => vec![],
        }
    }


    fn on_response(&mut self, what: Pending, v: &Value) -> Vec<Value> {
        let r = &v["result"];
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            let msg = error_text(e);
            match what {
                // 切不了模式、配置不影响干活：接着发下一个，最后发 prompt（权限请求还会自动允许）
                Pending::Mode => return self.step(),
                // 收尾时发的，不等也不管
                Pending::Close => {}
                Pending::Init => self.outcome = Some(Outcome::Failed(format!("initialize 失败：{msg}"))),
                Pending::Provider => self.outcome = Some(Outcome::Failed(format!("换供应商（providers/set）失败：{msg}"))),
                Pending::New if e["code"].as_i64() == Some(-32000) => self.outcome = Some(Outcome::Failed(format!("agent 要求先登录：{msg}"))),
                Pending::New => self.outcome = Some(Outcome::Failed(format!("session/new 失败：{msg}"))),
                // 接不回（会话已经没了、agent 记不住）：开新会话，说明之前的上下文没了
                Pending::Resume | Pending::Load => {
                    self.replaying = false;
                    self.note = Some(format!("接不回之前的会话（{msg}）：开了新会话，之前的对话它不记得了"));
                    return self.new_session();
                }
                Pending::Prompt => self.outcome = Some(self.turn_error(e, format!("session/prompt 失败：{msg}"))),
                // 列模型时出错：记下原因，有多少算多少
                Pending::Options if self.auth_mode == AuthMode::Probe => {
                    self.models = Some(LocalModels { error: format!("session/options 失败：{msg}"), ..Default::default() });
                    self.probe_done = true;
                }
                // 派发时认不了 configId：照旧开会话、开完再切
                Pending::Options => {
                    self.initial.push((String::new(), String::new(), String::new()));
                    return self.new_session();
                }
                Pending::ProbeNew => {
                    if let Some(m) = self.models.as_mut() {
                        m.error = format!("开会话失败（{msg}），只列出了默认模型的强度");
                    }
                    self.probe_done = true;
                }
                Pending::ProbeModel => return self.probe_next(),
                Pending::Auth => {
                    if self.auth.as_ref().is_some_and(|x| x.kind != AUTH_UNKNOWN) {
                        return self.after_auth();
                    }
                    if self.auth_push {
                        self.waiting_auth = true;
                        return vec![];
                    }
                    self.auth = Some(AgentAuth { kind: AUTH_UNKNOWN.into(), label: format!("auth/status 失败：{msg}"), ..Default::default() });
                    return self.after_auth();
                }
            }
            return vec![];
        }
        match what {
            Pending::Init => {
                // 版本协商：agent 回它能讲的、不高于我们要的最新版本
                self.proto = if r["protocolVersion"].as_i64().unwrap_or(1) >= 2 { Proto::V2 } else { Proto::V1 };
                // 能力：v1 在 agentCapabilities，v2 在 capabilities
                let caps = if r["agentCapabilities"].is_object() { &r["agentCapabilities"] } else { &r["capabilities"] };
                self.can_load = caps["loadSession"].as_bool().unwrap_or(false);
                self.can_resume = ["sessionCapabilities", "session", "sessions"].iter().any(|k| caps[*k]["resume"].is_object() || caps[*k]["resume"].as_bool() == Some(true));
                self.can_options = caps["session"]["options"].is_object() || caps["sessionCapabilities"]["options"].is_object();
                if self.auth_mode == AuthMode::Skip {
                    return self.after_init();
                }
                // 身份（#667）：公布了 `auth.status` 就问；只公布了 `_meta.authStatus` 的等它推；都没有的问不出来。
                // codex-acp 2.12 两种都公布，但 `auth/status` 的回应里没有种类，种类只在推送里：回应看不出种类时接着等推送
                self.auth_push = caps["_meta"]["authStatus"].is_object() || caps["auth"]["_meta"]["authStatus"].is_object();
                if caps["auth"]["status"].is_object() {
                    return vec![self.request("auth/status", json!({}), Pending::Auth)];
                }
                if self.auth_push {
                    if self.auth.is_none() {
                        self.waiting_auth = true;
                        return vec![];
                    }
                    return self.after_auth();
                }
                self.auth = Some(AgentAuth { kind: AUTH_UNKNOWN.into(), label: "这个版本的 agent 不报身份".into(), ..Default::default() });
                self.after_auth()
            }
            Pending::Options if self.auth_mode == AuthMode::Probe => {
                let mut m = models_of(r);
                // 默认模型的强度已经在这里了；别的模型开个不发提示词的会话逐个切
                self.probe_queue = m.models.iter().map(|x| x.id.clone()).filter(|id| *id != m.model).collect();
                self.probe_model_id = model_option(r).and_then(|o| option_id(o).map(str::to_string)).unwrap_or_default();
                if self.probe_queue.is_empty() || self.probe_model_id.is_empty() {
                    self.models = Some(m);
                    self.probe_done = true;
                    return vec![];
                }
                m.error = String::new();
                self.models = Some(m);
                vec![self.request("session/new", json!({ "cwd": self.cwd, "mcpServers": [] }), Pending::ProbeNew)]
            }
            Pending::Options => {
                // 派发：`@model`、`@thought_level` 按类别找到 configId，当初始值带上（先模型后强度）；找不到的留到开会话后再切
                let mut cats: Vec<&(String, String)> = self.sets.iter().filter(|(k, _)| k.starts_with('@')).collect();
                cats.sort_by_key(|(k, _)| if k == "@model" { 0 } else { 1 });
                for (k, v) in cats {
                    let cat = &k[1..];
                    if let Some(id) = r["configOptions"].as_array().into_iter().flatten().find(|o| o["category"].as_str() == Some(cat)).and_then(option_id) {
                        self.initial.push((id.to_string(), cat.to_string(), v.clone()));
                    }
                }
                let sent: Vec<String> = self.initial.iter().map(|(_, c, _)| format!("@{c}")).collect();
                self.sets.retain(|(k, _)| !sent.contains(k));
                // 一项都对不上：开会话时不带初始值（下面核对时照样会发现设不上）
                if self.initial.is_empty() {
                    self.initial.push((String::new(), String::new(), String::new()));
                }
                self.new_session()
            }
            Pending::ProbeNew => {
                let Some(id) = r["sessionId"].as_str().filter(|s| !s.is_empty()) else {
                    if let Some(m) = self.models.as_mut() {
                        m.error = "开会话没拿到 sessionId，只列出了默认模型的强度".into();
                    }
                    self.probe_done = true;
                    return vec![];
                };
                self.session = Some(id.to_string());
                self.probe_next()
            }
            Pending::ProbeModel => {
                let efforts = efforts_of(r);
                let cur = self.probe_current.clone();
                if let Some(x) = self.models.as_mut().and_then(|m| m.models.iter_mut().find(|x| x.id == cur)) {
                    x.efforts = efforts;
                }
                self.probe_next()
            }
            Pending::Auth => {
                let a = auth_of_status(r);
                if a.kind != AUTH_UNKNOWN {
                    self.auth = Some(a);
                    return self.after_auth();
                }
                // 回应里看不出种类：已经推来了就用推的，否则等推送（最多 AUTH_WAIT）
                if self.auth.as_ref().is_some_and(|x| x.kind != AUTH_UNKNOWN) {
                    return self.after_auth();
                }
                if self.auth_push {
                    self.waiting_auth = true;
                    return vec![];
                }
                self.auth = Some(a);
                self.after_auth()
            }
            Pending::Provider => self.new_session(),
            Pending::Resume | Pending::Load => {
                self.replaying = false;
                let id = v["result"]["sessionId"].as_str().map(str::to_string).or_else(|| self.pending_resume.take()).unwrap_or_default();
                self.session = Some(id.clone());
                self.note.get_or_insert_with(|| format!("接回了之前的会话（{}）", if what == Pending::Resume { "session/resume" } else { "session/load" }));
                let proto = self.proto;
                let mut queue: Vec<(&'static str, Value)> = self.sets.iter().filter_map(|(k, val)| config_set(r, &id, k, val, proto)).collect();
                queue.extend(self.mode.clone().and_then(|m| mode_switch(r, &id, &m, proto)));
                self.queue = queue;
                self.step()
            }
            Pending::New => {
                let Some(s) = r["sessionId"].as_str().filter(|s| !s.is_empty()) else {
                    self.outcome = Some(Outcome::Failed("session/new 的回应里没有 sessionId".into()));
                    return vec![];
                };
                self.session = Some(s.to_string());
                // 查额度：agent 公布了命令表、里面却没有这条本地命令（旧版 agent）时不发，免得被当成提示词交给模型、花额度
                if let (Some(cmd), Some(list)) = (&self.usage_cmd, r["availableCommands"].as_array()) {
                    if !list.iter().any(|c| c["name"].as_str() == Some(cmd.as_str())) {
                        self.outcome = Some(Outcome::Failed(format!("这个版本的 agent 没有 /{cmd} 命令，查不了额度")));
                        return vec![];
                    }
                }
                // 开会话时带的初始值（#666）：回应里是实际用的值，和要的不一样就是设不上，出错结束、不按默认跑
                if let Some(why) = initial_mismatch(r, &self.initial) {
                    self.outcome = Some(Outcome::Failed(why));
                    return vec![];
                }
                let proto = self.proto;
                // 认出了 session/options 的 v2 agent：剩下按类别的（初始值里没对上的）也不能静默跳过
                if self.can_options && proto == Proto::V2 {
                    if let Some((k, v)) = self.sets.iter().find(|(k, v)| k.starts_with('@') && config_set(r, s, k, v, proto).is_none()) {
                        self.outcome = Some(Outcome::Failed(format!("设不上{}：agent 没有「{v}」这一项", if k == "@model" { "模型" } else { "强度" })));
                        return vec![];
                    }
                }
                let mut queue: Vec<(&'static str, Value)> = self.sets.iter().filter_map(|(id, v)| config_set(r, s, id, v, proto)).collect();
                queue.extend(self.mode.clone().and_then(|m| mode_switch(r, s, &m, proto)));
                self.queue = queue;
                self.step()
            }
            Pending::Mode => self.step(),
            Pending::Close => vec![],
            // v1 的回应跨整轮，带着这一轮的结束原因；v2 的回应只表示提示词收下了，这一轮的结束看 `state_update`
            Pending::Prompt if self.proto == Proto::V2 => vec![],
            Pending::Prompt => {
                let reason = r["stopReason"].as_str().unwrap_or("end_turn").to_string();
                self.outcome = Some(Outcome::Stopped(reason));
                vec![]
            }
        }
    }


    /// v2 的 `session/update`：这个会话的 `state_update` 决定这一轮在不在干活（子会话的、别的更新只记日志）。
    /// `running`、`requires_action` 和认不出的状态都算在干活（规范：认不出的状态不能当 `idle`）；`idle` 带 `stopReason`
    /// 是这一轮做完了，不带的是 agent 重报一次（例如后台列表变了），不算新的结束
    fn on_update(&mut self, params: &Value) {
        let u = &params["update"];
        if u["sessionUpdate"].as_str() != Some("state_update") || params["sessionId"].as_str() != self.session.as_deref() {
            return;
        }
        self.background = u["background"].as_array().map(Vec::len);
        if !self.prompting {
            return;
        }
        match u["state"].as_str() {
            Some("idle") => match u["stopReason"].as_str() {
                Some("error") => self.outcome = Some(self.turn_error(&u["error"], format!("这一轮出错：{}", error_text(&u["error"])))),
                Some(reason) => self.outcome = Some(Outcome::Stopped(reason.to_string())),
                None => {}
            },
            _ => self.outcome = None,
        }
    }


    /// `usage_update`：Claude 在 `_meta["_claude/rateLimit"]` 里带额度（#668），记下最近一次的。子会话报的也算（同一个账号）
    fn on_usage(&mut self, params: &Value) {
        let u = &params["update"];
        if u["sessionUpdate"].as_str() != Some("usage_update") {
            return;
        }
        if let Some(q) = rate_of(&u["_meta"]["_claude/rateLimit"]) {
            self.quota = Some(q);
        }
    }


    /// 查额度：收下这个会话里 agent 说的话（本地命令的输出）
    fn on_usage_text(&mut self, params: &Value) {
        if self.usage_cmd.is_none() || params["sessionId"].as_str() != self.session.as_deref() {
            return;
        }
        if let Some((kind, _, text)) = chunk(&json!({ "method": "session/update", "params": params })) {
            if kind == "agent_message_chunk" {
                self.usage_text += &text;
            }
        }
    }


    /// agent 回的这一轮出错（`error` 是 ACP 的错误对象）：额度用完时是 [`Outcome::Quota`]，[`Client::quota`] 记上用完和
    /// 重置时间；别的照旧是 [`Outcome::Failed`]
    fn turn_error(&mut self, error: &Value, why: String) -> Outcome {
        match quota_of(error, self.quota.as_ref(), chrono::Local::now()) {
            Some(q) => {
                self.quota = Some(q);
                Outcome::Quota(why)
            }
            None => Outcome::Failed(why),
        }
    }
}

/* ------------------------------ 模型和强度（#666） ------------------------------ */

/// 配置项里类别是 model 的那一项
fn model_option(r: &Value) -> Option<&Value> {
    r["configOptions"].as_array()?.iter().find(|o| o["category"].as_str() == Some("model"))
}


/// 选项的（值、显示名），分组的也展开
fn option_pairs(v: &Value) -> Vec<(String, String)> {
    let mut out = vec![];
    for o in v.as_array().into_iter().flatten() {
        if let Some(x) = o["value"].as_str() {
            out.push((x.to_string(), o["name"].as_str().unwrap_or_default().to_string()));
        }
        out.extend(option_pairs(&o["options"]));
    }
    out
}


/// 「agent 自己的默认」这种选项：不算一个模型、一档强度
fn is_default(v: &str) -> bool {
    v.eq_ignore_ascii_case("default")
}


/// 会话配置里现在这个模型能设的强度（类别 thought_level 的选项，去掉 default）；没有这一项时为空
fn efforts_of(r: &Value) -> Vec<String> {
    r["configOptions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|o| o["category"].as_str() == Some("thought_level"))
        .map(|o| option_pairs(&o["options"]).into_iter().map(|(v, _)| v).filter(|v| !is_default(v)).collect())
        .unwrap_or_default()
}


/// `session/options` 的回应 → 模型表：模型下拉（去掉 default）、默认模型、默认模型的强度和默认强度
fn models_of(r: &Value) -> LocalModels {
    let at = board_core::now_ms();
    let Some(o) = model_option(r) else {
        return LocalModels { at, error: "agent 没有给模型下拉".into(), ..Default::default() };
    };
    let cur = o["currentValue"].as_str().unwrap_or_default().to_string();
    let efforts = efforts_of(r);
    let effort = r["configOptions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|x| x["category"].as_str() == Some("thought_level"))
        .and_then(|x| x["currentValue"].as_str())
        .filter(|v| !is_default(v))
        .unwrap_or_default()
        .to_string();
    let models = option_pairs(&o["options"])
        .into_iter()
        .filter(|(v, _)| !is_default(v))
        .map(|(id, name)| LocalModel { efforts: if id == cur { efforts.clone() } else { vec![] }, id, name })
        .collect();
    LocalModels { models, model: if is_default(&cur) { String::new() } else { cur }, effort, at, error: String::new() }
}


/// 开会话时带的初始值和回应里实际的值对不上时的原因：「账号里没有模型 X」「模型 X 不支持强度 Y」
fn initial_mismatch(r: &Value, initial: &[(String, String, String)]) -> Option<String> {
    let list = r["configOptions"].as_array().cloned().unwrap_or_default();
    let cur = |cat: &str| list.iter().find(|o| o["category"].as_str() == Some(cat)).and_then(|o| o["currentValue"].as_str()).map(str::to_string);
    for (id, cat, want) in initial.iter().filter(|(id, _, _)| !id.is_empty()) {
        let got = list.iter().find(|o| option_id(o) == Some(id.as_str())).and_then(|o| o["currentValue"].as_str());
        if got == Some(want.as_str()) {
            continue;
        }
        return Some(match cat.as_str() {
            "model" => format!("账号里没有模型 {want}（会话用的是 {}）：没有按默认模型跑", got.unwrap_or("默认")),
            "thought_level" => format!("模型 {} 不支持强度 {want}：没有按默认强度跑", cur("model").unwrap_or_else(|| "默认".into())),
            _ => format!("设不上 {id} = {want}"),
        });
    }
    None
}

/* ------------------------------ 身份（#667） ------------------------------ */

/// 扩展的 `authStatus`（`{kind, label, detail?, account?: {email, plan, …}}`，Claude Code、Codex 一样）→ 身份。
/// 邮箱留着认账号（#669），组织名不留
fn auth_of_ext(v: &Value) -> AgentAuth {
    let s = |x: &Value| x.as_str().unwrap_or_default().trim().to_string();
    let kind = s(&v["kind"]);
    AgentAuth {
        kind: if kind.is_empty() { AUTH_UNKNOWN.into() } else { kind },
        label: s(&v["label"]),
        detail: s(&v["detail"]),
        plan: s(&v["account"]["plan"]),
        email: s(&v["account"]["email"]),
    }
}


/// ACP `auth/status` 的回应（`{state, accounts}`）→ 身份。种类只有 Claude Code 在账号的 `_meta.claudeCode.kind` 里给；
/// 没有时登录了也按问不出来算（看不出是不是订阅）。邮箱在账号的 `email`（#669）
fn auth_of_status(r: &Value) -> AgentAuth {
    let s = |x: &Value| x.as_str().unwrap_or_default().trim().to_string();
    match r["state"].as_str() {
        Some("unauthenticated") => AgentAuth { kind: AUTH_NONE.into(), label: "Not logged in".into(), ..Default::default() },
        Some("authenticated") => {
            let list = r["accounts"].as_array().cloned().unwrap_or_default();
            let a = list.iter().find(|a| a["active"].as_bool() == Some(true)).or(list.first()).cloned().unwrap_or(Value::Null);
            let meta = &a["_meta"]["claudeCode"];
            let kind = s(&meta["kind"]);
            AgentAuth {
                kind: if kind.is_empty() { AUTH_UNKNOWN.into() } else { kind },
                label: s(&a["label"]),
                detail: [s(&a["description"]), s(&meta["detail"])].into_iter().find(|d| !d.is_empty()).unwrap_or_default(),
                plan: s(&meta["plan"]),
                email: s(&a["email"]),
            }
        }
        _ => AgentAuth { kind: AUTH_UNKNOWN.into(), ..Default::default() },
    }
}


/// 毫秒时间戳 → 本机时间「11-26 15:15」（写进日志给人看）
pub(super) fn local_words(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).earliest().map(|t| t.format("%m-%d %H:%M").to_string()).unwrap_or_default()
}


/// 回答 agent 的请求：权限自动允许；其他方法没有（没声明的能力）
fn answer(method: &str, id: &Value, params: &Value) -> Value {
    match method {
        "session/request_permission" => json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": permission(params) } }),
        _ => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Board 不支持 {method}") } }),
    }
}


/// 选「总是允许」，没有就「允许这一次」；都没有时回取消
fn permission(params: &Value) -> Value {
    let opts = params["options"].as_array().cloned().unwrap_or_default();
    let kind = |k: &str| opts.iter().find(|o| o["kind"].as_str() == Some(k)).and_then(|o| o["optionId"].as_str()).map(str::to_string);
    let pick = kind("allow_always").or_else(|| kind("allow_once")).or_else(|| {
        opts.iter().find(|o| o["kind"].as_str().is_some_and(|k| k.starts_with("allow"))).and_then(|o| o["optionId"].as_str()).map(str::to_string)
    });
    match pick {
        Some(id) => json!({ "outcome": "selected", "optionId": id }),
        None => json!({ "outcome": "cancelled" }),
    }
}


/// 按 session/new 的回应决定怎么切到 mode：会话配置里有「模式」这一项、且有这个值时用 `session/set_config_option`；
/// 否则（只有 v1）公布的模式里有它时用 `session/set_mode`；都没有时不切
fn mode_switch(r: &Value, session: &str, mode: &str, proto: Proto) -> Option<(&'static str, Value)> {
    for o in r["configOptions"].as_array().into_iter().flatten() {
        let is_mode = option_id(o) == Some("mode") || o["category"].as_str() == Some("mode");
        if is_mode && option_values(&o["options"]).iter().any(|v| v == mode) {
            return Some(("session/set_config_option", set_option(session, option_id(o).unwrap_or("mode"), mode, proto)));
        }
    }
    let modes = r["modes"]["availableModes"].as_array().into_iter().flatten();
    if proto == Proto::V1 && modes.into_iter().any(|m| m["id"].as_str() == Some(mode)) {
        return Some(("session/set_mode", json!({ "sessionId": session, "modeId": mode })));
    }
    None
}


/// 会话配置里有 id 这一项、而且有这个值时：`session/set_config_option`。id 写成 `@<类别>` 时按配置项的类别找
/// （`@model`、`@thought_level`；找不到类别时 `@thought_level` 再按 dsh 的 `reasoning_effort` 找）
fn config_set(r: &Value, session: &str, id: &str, value: &str, proto: Proto) -> Option<(&'static str, Value)> {
    let list = r["configOptions"].as_array()?;
    let o = match id.strip_prefix('@') {
        Some(cat) => list
            .iter()
            .find(|o| o["category"].as_str() == Some(cat))
            .or_else(|| (cat == "thought_level").then(|| list.iter().find(|o| option_id(o) == Some("reasoning_effort"))).flatten())?,
        None => list.iter().find(|o| option_id(o) == Some(id))?,
    };
    let id = option_id(o)?;
    let offered = option_values(&o["options"]).iter().any(|v| v == value);
    offered.then(|| ("session/set_config_option", set_option(session, id, value, proto)))
}


/// 配置项的标识：v2 叫 `configId`，v1 叫 `id`
fn option_id(o: &Value) -> Option<&str> {
    o["configId"].as_str().or_else(|| o["id"].as_str())
}


/// `session/set_config_option` 的参数：v2 要写明值的类型（下拉选项是 `id`）
fn set_option(session: &str, id: &str, value: &str, proto: Proto) -> Value {
    match proto {
        Proto::V1 => json!({ "sessionId": session, "configId": id, "value": value }),
        Proto::V2 => json!({ "sessionId": session, "configId": id, "type": "id", "value": value }),
    }
}


/// 选项的值：`[{value, name}]`，也可能分组 `[{group, options: [...]}]`
fn option_values(v: &Value) -> Vec<String> {
    let mut out = vec![];
    for o in v.as_array().into_iter().flatten() {
        if let Some(x) = o["value"].as_str() {
            out.push(x.to_string());
        }
        out.extend(option_values(&o["options"]));
    }
    out
}


fn error_text(e: &Value) -> String {
    let msg = e["message"].as_str().unwrap_or("出错了");
    match e["data"].as_str().or_else(|| e["data"]["message"].as_str()) {
        Some(d) if !d.is_empty() && d != msg => format!("{msg}（{d}）"),
        _ => msg.to_string(),
    }
}


/// 结束原因 → 退出码
pub(crate) fn exit_for(o: &Outcome) -> i32 {
    match o {
        Outcome::Stopped(r) if r == "end_turn" => EXIT_OK,
        Outcome::Stopped(r) if r == "cancelled" => EXIT_CANCELLED,
        Outcome::Stopped(_) => EXIT_LIMIT,
        Outcome::Failed(_) => EXIT_PROTOCOL,
        Outcome::Quota(_) => EXIT_QUOTA,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::testutil::{reply, s, state};
    use crate::acp::parse;
    use crate::acp::run::loggable;

    /// 续轮（#454）：这一轮结束后在同一个会话里再发一条 prompt，回到「进行中」，取消照样有效
    #[test]
    fn again_sends_another_prompt_in_the_same_session() {
        let mut c = Client::new("/w", "p", None);
        assert!(c.again("x").is_empty(), "还没有会话：不发");
        c.start();
        c.on_frame(&reply(0, json!({})));
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s" })));
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64()), (Some("session/prompt"), Some(2)));
        assert!(c.again("x").is_empty(), "这一轮还没结束：不发");
        c.on_frame(&reply(2, json!({ "stopReason": "end_turn" })));
        assert_eq!(c.outcome, Some(Outcome::Stopped("end_turn".into())));
        let f = c.again("还持有 #7：接着做完");
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64()), (Some("session/prompt"), Some(3)));
        assert_eq!(f[0]["params"], json!({ "sessionId": "s", "prompt": [{ "type": "text", "text": "还持有 #7：接着做完" }] }));
        assert_eq!(c.outcome, None, "续的这一轮在进行中");
        assert!(!c.handshaking());
        assert_eq!(c.cancel()[0]["method"], "session/cancel");
        // 续的这一轮结束：照常记结果
        c.on_frame(&reply(3, json!({ "stopReason": "end_turn" })));
        assert_eq!(c.outcome, Some(Outcome::Stopped("end_turn".into())));
    }

    /// 走一遍 v1（agent 只会 v1，例如 dsh）：initialize → session/new → 切模式 → prompt → end_turn
    #[test]
    fn handshake_mode_and_prompt() {
        let mut c = Client::new("/w", "/board-skills 开始执行", Some("bypassPermissions"));
        let f = c.start();
        // 要 v2，同时带上 v1 的字段：只会 v1 的 agent 照样认得
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64(), f[0]["params"]["protocolVersion"].as_i64()), (Some("initialize"), Some(0), Some(2)));
        assert_eq!((f[0]["params"]["info"]["name"].as_str(), f[0]["params"]["clientInfo"]["name"].as_str()), (Some("board"), Some("board")));
        assert_eq!((&f[0]["params"]["capabilities"], &f[0]["params"]["clientCapabilities"]["terminal"]), (&json!({}), &json!(false)));
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": {} })));
        assert_eq!(c.proto, Proto::V1, "agent 回 1：按 v1 走");
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["cwd"].as_str()), (Some("session/new"), Some("/w")));
        assert_eq!(f[0]["params"]["mcpServers"], json!([]));
        assert!(c.handshaking());
        // 会话配置里有 mode 这一项（Claude Code）：用 set_config_option
        let opts = json!([{ "id": "mode", "category": "mode", "type": "select", "currentValue": "default", "options": [{ "value": "default" }, { "value": "bypassPermissions" }] }]);
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s1", "configOptions": opts })));
        assert_eq!(f[0]["method"], "session/set_config_option");
        assert_eq!(f[0]["params"], json!({ "sessionId": "s1", "configId": "mode", "value": "bypassPermissions" }));
        assert!(c.cancel().is_empty(), "还没发 prompt：没有可取消的");
        let f = c.on_frame(&reply(2, json!({})));
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["prompt"][0]["text"].as_str()), (Some("session/prompt"), Some("/board-skills 开始执行")));
        assert!(!c.handshaking());
        // 期间的通知不用回
        assert!(c.on_frame(&json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s1", "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "hi" } } } })).is_empty());
        assert_eq!(c.cancel()[0], json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": "s1" } }));
        assert!(c.on_frame(&reply(3, json!({ "stopReason": "end_turn" }))).is_empty());
        assert_eq!(c.outcome, Some(Outcome::Stopped("end_turn".into())));
        assert_eq!(exit_for(c.outcome.as_ref().unwrap()), 0);
        assert!(c.close().is_empty(), "v1 收尾不发 session/close：关标准输入就行");
    }

    /// 对话：agent 会 session/resume 就接回（不回放）；会话开好后不发提示词，等 ask；人写的消息原样发出去；
    /// 一轮结束后同一个会话里接着发
    #[test]
    fn chat_resumes_and_sends_messages_as_is() {
        let mut c = Client::new("/home/me", "", None).for_chat(Some("s1".into()));
        c.start();
        let caps = json!({ "loadSession": true, "sessionCapabilities": { "resume": {}, "close": {} } });
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": caps })));
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["sessionId"].as_str(), f[0]["params"]["cwd"].as_str()), (Some("session/resume"), Some("s1"), Some("/home/me")));
        assert!(!c.replaying, "resume 不回放历史");
        assert!(c.ask("早").is_empty(), "会话还没接回：不发");
        let f = c.on_frame(&reply(1, json!({})));
        assert!(f.is_empty(), "接回后不自己发提示词");
        assert!(c.ready && !c.busy());
        assert_eq!(c.session(), Some("s1"));
        assert!(c.note.as_deref().unwrap().contains("接回"));
        let f = c.ask("看看主目录下哪些文件夹最大");
        assert_eq!(f[0]["method"], "session/prompt");
        assert_eq!(f[0]["params"]["prompt"], json!([{ "type": "text", "text": "看看主目录下哪些文件夹最大" }]), "原样，前后不加说明");
        assert!(c.busy());
        assert!(c.ask("第二条").is_empty(), "这一轮没结束：不发");
        c.on_frame(&reply(2, json!({ "stopReason": "end_turn" })));
        assert!(!c.busy());
        let f = c.ask("清一下 ~/.cache");
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["sessionId"].as_str()), (Some("session/prompt"), Some("s1")));
        // 停止这一轮
        assert_eq!(c.cancel()[0]["method"], "session/cancel");
        c.on_frame(&reply(3, json!({ "stopReason": "cancelled" })));
        assert_eq!(c.outcome, Some(Outcome::Stopped("cancelled".into())));
        assert!(!c.busy());
    }

    /// 对话：只会 session/load 时用它（回放的历史不记日志）；都不会、接不回时开新会话，说明之前的对话没了
    #[test]
    fn chat_falls_back_to_load_then_new_session() {
        let mut c = Client::new("/home/me", "", None).for_chat(Some("s1".into()));
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": { "loadSession": true } })));
        assert_eq!(f[0]["method"], "session/load");
        assert!(c.replaying);
        let f = c.on_frame(&json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32002, "message": "没有这个会话" } }));
        assert!(!c.replaying);
        assert_eq!(f[0]["method"], "session/new", "接不回：开新会话");
        assert!(c.note.as_deref().unwrap().contains("接不回"));
        c.on_frame(&reply(2, json!({ "sessionId": "s2" })));
        assert!(c.ready);
        assert_eq!(c.session(), Some("s2"));

        let mut c = Client::new("/home/me", "", None).for_chat(Some("s1".into()));
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": {} })));
        assert_eq!(f[0]["method"], "session/new");
        assert!(c.note.as_deref().unwrap().contains("不支持接回"));
        // 第一次（没有要接回的）：直接开新会话，不写说明
        let mut c = Client::new("/home/me", "", None).for_chat(None);
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 2, "capabilities": {} })));
        assert_eq!(f[0]["method"], "session/new");
        assert!(c.note.is_none());
    }

    /// 走一遍 v2（Claude Code、Codex）：配置项用 configId、切的时候带 type；prompt 的回应只是收下了，
    /// 这一轮的进展看 state_update；后台列表跟着记；收尾发 session/close
    #[test]
    fn v2_turns_follow_state_updates() {
        let mut c = Client::new("/w", "/board-skills 开始执行", Some("bypassPermissions"));
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 2, "info": { "name": "claude-agent-acp" }, "capabilities": {} })));
        assert_eq!((c.proto, f[0]["method"].as_str()), (Proto::V2, Some("session/new")));
        let opts = json!([{ "configId": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "auto", "options": [{ "value": "auto", "name": "Auto" }, { "value": "bypassPermissions", "name": "Bypass" }] }]);
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s1", "configOptions": opts })));
        assert_eq!(f[0]["params"], json!({ "sessionId": "s1", "configId": "mode", "type": "id", "value": "bypassPermissions" }));
        // 还没发 prompt：会话开始时报的状态不算这一轮
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn" })));
        assert_eq!(c.outcome, None);
        let f = c.on_frame(&reply(2, json!({ "configOptions": opts })));
        assert_eq!(f[0]["method"], "session/prompt");
        assert!(c.on_frame(&reply(3, json!({ "messageId": "m1" }))).is_empty());
        assert_eq!(c.outcome, None, "v2 的 prompt 回应只是收下了");
        c.on_frame(&state("s1", json!({ "state": "running" })));
        assert_eq!(c.outcome, None);
        assert_eq!(c.cancel()[0]["method"], "session/cancel", "在干活：取消有效");
        // 别的会话（子 agent）的状态不算
        c.on_frame(&state("child", json!({ "state": "idle", "stopReason": "end_turn" })));
        assert_eq!(c.outcome, None);
        // 这一轮做完了，后台还有一个终端在跑
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn", "background": [{ "type": "terminal", "terminalId": "t1" }], "usage": { "inputTokens": 5 } })));
        assert_eq!((c.outcome.clone(), c.background), (Some(Outcome::Stopped("end_turn".into())), Some(1)));
        // 后台跑完：agent 重报一次 idle（不带 stopReason），不是新的结束；结束原因照旧
        c.on_frame(&state("s1", json!({ "state": "idle", "background": [] })));
        assert_eq!((c.outcome.clone(), c.background), (Some(Outcome::Stopped("end_turn".into())), Some(0)));
        // agent 自己接着做（处理跑完的后台任务）：又在干活
        c.on_frame(&state("s1", json!({ "state": "running", "background": [] })));
        assert_eq!(c.outcome, None);
        // 认不出的状态也算在干活
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn" })));
        c.on_frame(&state("s1", json!({ "state": "_compacting" })));
        assert_eq!(c.outcome, None);
        // 这一轮出错：退出码 2，原因写明
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": { "code": -32603, "message": "rate limited" } })));
        assert_eq!(c.outcome, Some(Outcome::Failed("这一轮出错：rate limited".into())));
        assert_eq!(exit_for(c.outcome.as_ref().unwrap()), 2);
        // 不报后台：None（不知道），不是 0
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn" })));
        assert_eq!(c.background, None);
        let f = c.close();
        assert_eq!((f[0]["method"].as_str(), &f[0]["params"]), (Some("session/close"), &json!({ "sessionId": "s1" })));
    }

    /// --set：会话配置里有这一项、这个值时先切它，再切模式，最后 prompt；没有的跳过
    /// 角色用供应商：握手后先 `providers/set`（这一个进程换上它），再开会话；模型、强度按配置项的类别切。日志里的 key 遮住
    #[test]
    fn provider_is_set_before_the_session_and_sets_go_by_category() {
        let provider = json!({ "providerId": "main", "apiType": "anthropic", "baseUrl": "https://relay.example", "headers": { "Authorization": "Bearer sk-1" } });
        let mut c = Client::new("/w", "p", None).with_sets(vec![("@model".into(), "model-a".into()), ("@thought_level".into(), "high".into())]).with_provider(Some(provider.clone()));
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 2 })));
        assert_eq!((f[0]["method"].as_str(), &f[0]["params"]), (Some("providers/set"), &provider));
        assert!(loggable(&f[0]).contains("\"Authorization\":\"***\"") && !loggable(&f[0]).contains("sk-1"), "{}", loggable(&f[0]));
        let f = c.on_frame(&reply(1, json!({})));
        assert_eq!(f[0]["method"], "session/new");
        let opts = json!([
            { "configId": "model", "category": "model", "options": [{ "value": "model-a" }, { "value": "model-b" }] },
            { "configId": "effort", "category": "thought_level", "options": [{ "value": "low" }, { "value": "high" }] }
        ]);
        let f = c.on_frame(&reply(2, json!({ "sessionId": "s", "configOptions": opts })));
        assert_eq!(f[0]["params"], json!({ "sessionId": "s", "configId": "model", "type": "id", "value": "model-a" }));
        let f = c.on_frame(&reply(3, json!({})));
        assert_eq!(f[0]["params"]["configId"], "effort");
        // providers/set 失败：不开会话
        let mut c = Client::new("/w", "p", None).with_provider(Some(json!({})));
        c.start();
        c.on_frame(&reply(0, json!({ "protocolVersion": 2 })));
        assert!(c.on_frame(&json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32603, "message": "Model discovery failed" } })).is_empty());
        assert!(matches!(&c.outcome, Some(Outcome::Failed(m)) if m.contains("换供应商")));
        // dsh 的强度没有类别：按 reasoning_effort 找
        let mut c = Client::new("/w", "p", None).with_sets(vec![("@thought_level".into(), "max".into())]);
        c.start();
        c.on_frame(&reply(0, json!({})));
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s", "configOptions": [{ "id": "reasoning_effort", "options": [{ "value": "max" }] }] })));
        assert_eq!(f[0]["params"]["configId"], "reasoning_effort");
    }

    #[test]
    fn config_sets_go_before_the_mode_and_the_prompt() {
        let mut c = Client::new("/w", "p", Some("full")).with_sets(vec![("reasoning_effort".into(), "high".into()), ("model".into(), "nope".into())]);
        c.start();
        c.on_frame(&reply(0, json!({})));
        let opts = json!([
            { "id": "model", "options": [{ "value": "a" }] },
            { "id": "reasoning_effort", "options": [{ "value": "low" }, { "value": "high" }] }
        ]);
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s", "configOptions": opts, "modes": { "availableModes": [{ "id": "full" }] } })));
        assert_eq!(f[0]["params"], json!({ "sessionId": "s", "configId": "reasoning_effort", "value": "high" }));
        let f = c.on_frame(&reply(2, json!({})));
        assert_eq!(f[0]["method"], "session/set_mode", "model=nope 没有这个值：跳过");
        let f = c.on_frame(&reply(3, json!({})));
        assert_eq!(f[0]["method"], "session/prompt");
    }

    #[test]
    fn mode_falls_back_to_set_mode_or_skips() {
        let r = json!({ "sessionId": "s", "modes": { "currentModeId": "auto", "availableModes": [{ "id": "read-only" }, { "id": "agent-full-access" }] } });
        assert_eq!(mode_switch(&r, "s", "agent-full-access", Proto::V1), Some(("session/set_mode", json!({ "sessionId": "s", "modeId": "agent-full-access" }))));
        assert_eq!(mode_switch(&r, "s", "bypassPermissions", Proto::V1), None, "没公布这个模式：不切");
        assert_eq!(mode_switch(&r, "s", "agent-full-access", Proto::V2), None, "v2 没有 session/set_mode：只看配置项");
        // 分组的选项也认得；v2 的配置项叫 configId
        let r = json!({ "configOptions": [{ "id": "perm", "category": "mode", "options": [{ "group": "g", "options": [{ "value": "yolo" }] }] }] });
        assert_eq!(mode_switch(&r, "s", "yolo", Proto::V1).map(|x| x.1["configId"].clone()), Some(json!("perm")));
        let r2 = json!({ "configOptions": [{ "configId": "perm", "category": "mode", "options": [{ "value": "yolo" }] }] });
        assert_eq!(mode_switch(&r2, "s", "yolo", Proto::V2).map(|x| x.1), Some(json!({ "sessionId": "s", "configId": "perm", "type": "id", "value": "yolo" })));
        // 没要模式：session/new 之后直接 prompt
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&reply(0, json!({})));
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s", "modes": r })));
        assert_eq!(f[0]["method"], "session/prompt");
        // 切模式失败也接着干活
        let mut c = Client::new("/w", "p", Some("x"));
        c.start();
        c.on_frame(&reply(0, json!({})));
        c.on_frame(&reply(1, json!({ "sessionId": "s", "modes": { "availableModes": [{ "id": "x" }] } })));
        let f = c.on_frame(&json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32603, "message": "nope" } }));
        assert_eq!(f[0]["method"], "session/prompt");
    }

    #[test]
    fn requests_from_the_agent_get_answers() {
        let mut c = Client::new("/w", "p", None);
        let ask = |id: Value, options: Value| json!({ "jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": { "sessionId": "s", "toolCall": { "toolCallId": "t" }, "options": options } });
        let both = json!([{ "optionId": "once", "kind": "allow_once" }, { "optionId": "always", "kind": "allow_always" }, { "optionId": "no", "kind": "reject_once" }]);
        assert_eq!(c.on_frame(&ask(json!(7), both))[0], json!({ "jsonrpc": "2.0", "id": 7, "result": { "outcome": { "outcome": "selected", "optionId": "always" } } }));
        let once = json!([{ "optionId": "no", "kind": "reject_once" }, { "optionId": "ok", "kind": "allow_once" }]);
        assert_eq!(c.on_frame(&ask(json!("a"), once))[0]["result"]["outcome"]["optionId"], "ok", "字符串的 id 原样带回");
        let none = json!([{ "optionId": "no", "kind": "reject_once" }]);
        assert_eq!(c.on_frame(&ask(json!(8), none))[0]["result"]["outcome"], json!({ "outcome": "cancelled" }));
        let fs = c.on_frame(&json!({ "jsonrpc": "2.0", "id": 9, "method": "fs/read_text_file", "params": {} }));
        assert_eq!(fs[0]["error"]["code"], -32601);
        // 不认识的回应、没有 id 的东西：不理
        assert!(c.on_frame(&reply(99, json!({}))).is_empty());
    }

    #[test]
    fn failures_and_stop_reasons_become_exit_codes() {
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&json!({ "jsonrpc": "2.0", "id": 0, "error": { "code": -32603, "message": "boom", "data": "details" } }));
        assert_eq!(c.outcome, Some(Outcome::Failed("initialize 失败：boom（details）".into())));
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&reply(0, json!({})));
        c.on_frame(&json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32000, "message": "Authentication required" } }));
        assert_eq!(c.outcome, Some(Outcome::Failed("agent 要求先登录：Authentication required".into())));
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&reply(0, json!({})));
        c.on_frame(&reply(1, json!({})));
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.contains("sessionId")));
        assert_eq!(exit_for(&Outcome::Stopped("cancelled".into())), 130);
        assert_eq!(exit_for(&Outcome::Stopped("max_tokens".into())), 3);
        assert_eq!(exit_for(&Outcome::Stopped("refusal".into())), 3);
        assert_eq!(exit_for(&Outcome::Failed("x".into())), 2);
    }

    /// 「官方订阅」（#667）：Claude Code 公布了 auth.status → 先问 auth/status，是订阅才开会话；是 API key 就出错、写明身份
    #[test]
    fn subscription_asks_auth_status_before_the_session() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {} }, "_meta": { "authStatus": {} } } });
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        let f = c.on_frame(&reply(0, init.clone()));
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64()), (Some("auth/status"), Some(1)));
        let max = json!({ "state": "authenticated", "accounts": [{ "label": "Claude Max", "active": true, "email": "a@b.c", "_meta": { "claudeCode": { "kind": "account", "plan": "max" } } }] });
        let f = c.on_frame(&reply(1, max));
        assert_eq!(f[0]["method"], "session/new", "是订阅：开会话");
        assert_eq!(c.auth, Some(AgentAuth { kind: "account".into(), label: "Claude Max".into(), detail: String::new(), plan: "max".into(), email: "a@b.c".into() }));
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, init));
        let key = json!({ "state": "authenticated", "accounts": [{ "label": "Anthropic API key", "description": "ANTHROPIC_API_KEY", "active": true, "_meta": { "claudeCode": { "kind": "api_key", "detail": "ANTHROPIC_API_KEY" } } }] });
        assert!(c.on_frame(&reply(1, key)).is_empty(), "不是订阅：不开会话");
        assert_eq!(c.outcome, Some(Outcome::Failed("按「官方订阅」派发，agent 用的不是订阅：Anthropic API key（ANTHROPIC_API_KEY），没有开会话".into())));
    }

    /// Codex 只公布 `_meta.authStatus`：等它推 `_auth/status_update`；没登录、网关都不开会话；不报身份的、等不到的也不开
    #[test]
    fn subscription_waits_for_the_pushed_auth_status() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "auth": { "_meta": { "authStatus": {} } } } });
        let push = |st: Value| json!({ "jsonrpc": "2.0", "method": "_auth/status_update", "params": { "authStatus": st } });
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        assert!(c.on_frame(&reply(0, init.clone())).is_empty());
        assert!(c.waiting_auth && c.handshaking());
        let f = c.on_frame(&push(json!({ "kind": "account", "label": "ChatGPT Plus", "account": { "email": "a@b.c", "plan": "plus" } })));
        assert_eq!(f[0]["method"], "session/new");
        assert_eq!(c.auth.as_ref().map(|a| (a.label.as_str(), a.plan.as_str(), a.email.as_str())), Some(("ChatGPT Plus", "plus", "a@b.c")));
        for (st, want) in [
            (json!({ "kind": "gateway", "label": "Custom model gateway", "detail": "relay" }), "Custom model gateway（relay）"),
            (json!({ "kind": "none", "label": "Not logged in" }), "Not logged in"),
        ] {
            let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
            c.start();
            c.on_frame(&reply(0, init.clone()));
            assert!(c.on_frame(&push(st)).is_empty());
            assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.contains(want)), "{:?}", c.outcome);
        }
        // 推之前就到了：不用等
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&push(json!({ "kind": "account", "label": "ChatGPT Pro" })));
        assert_eq!(c.on_frame(&reply(0, init.clone()))[0]["method"], "session/new");
        // 等不到
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, init));
        assert!(c.auth_timeout().is_empty());
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.contains("秒内没报身份")), "{:?}", c.outcome);
        // 不报身份的 agent（dsh、旧版适配器）
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, json!({ "protocolVersion": 1 })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.contains("不报身份")), "{:?}", c.outcome);
        // 不问身份（默认）：照旧直接开会话
        let mut c = Client::new("/w", "p", None);
        c.start();
        assert_eq!(c.on_frame(&reply(0, json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {} } } })))[0]["method"], "session/new");
    }

    /// codex-acp 2.12：公布了 auth.status，但回应里没有种类（只有 label），种类在推送里：不能按「不知道」算，要用推送的
    #[test]
    fn auth_status_without_kind_waits_for_the_push() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {}, "_meta": { "authStatus": {} } } } });
        let push = json!({ "jsonrpc": "2.0", "method": "_auth/status_update", "params": { "authStatus": { "kind": "account", "label": "ChatGPT Pro", "account": { "plan": "pro" } } } });
        let answer = json!({ "state": "authenticated", "accounts": [{ "label": "ChatGPT Pro", "active": true }] });
        // 回应先到：接着等推送
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, init.clone()));
        assert!(c.on_frame(&reply(1, answer.clone())).is_empty() && c.waiting_auth);
        assert_eq!(c.on_frame(&push)[0]["method"], "session/new");
        // 推送先到：回应来了直接用推送的
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, init));
        assert!(c.on_frame(&push).is_empty(), "还在等 auth/status 的回应");
        assert_eq!(c.on_frame(&reply(1, answer))[0]["method"], "session/new");
        assert_eq!(c.auth.map(|a| a.kind), Some("account".to_string()));
    }

    /// 会话配置项：模型（当前值、可选值）、强度（当前值、可选值；None = 这个模型没有强度这一项）
    fn opts_of(model: &str, models: &[&str], effort: Option<(&str, &[&str])>) -> Value {
        let mut list = vec![json!({ "configId": "model", "category": "model", "type": "select", "currentValue": model,
            "options": models.iter().map(|m| json!({ "value": m, "name": m.to_uppercase() })).collect::<Vec<_>>() })];
        if let Some((cur, vals)) = effort {
            list.push(json!({ "configId": "effort", "category": "thought_level", "type": "select", "currentValue": cur,
                "options": vals.iter().map(|v| json!({ "value": v, "name": v })).collect::<Vec<_>>() }));
        }
        Value::Array(list)
    }

    /// 列模型（#666）：session/options 给模型和默认模型的强度，再开不发提示词的会话逐个切，读出每个模型的强度；default 不算
    #[test]
    fn probe_lists_models_and_their_efforts() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {} }, "session": { "options": {} } } });
        let mut c = Client::new("/w", "", None).with_auth(AuthMode::Probe);
        c.start();
        assert_eq!(c.on_frame(&reply(0, init))[0]["method"], "auth/status");
        let f = c.on_frame(&reply(1, json!({ "state": "authenticated", "accounts": [{ "label": "Claude Max", "_meta": { "claudeCode": { "kind": "account" } } }] })));
        assert_eq!((f[0]["method"].as_str(), &f[0]["params"]), (Some("session/options"), &json!({ "cwd": "/w" })));
        let f = c.on_frame(&reply(2, json!({ "configOptions": opts_of("opus", &["default", "opus", "haiku", "sonnet"], Some(("high", &["default", "low", "high", "max"]))) })));
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["mcpServers"].clone()), (Some("session/new"), json!([])));
        let f = c.on_frame(&reply(3, json!({ "sessionId": "p1" })));
        assert_eq!(f[0]["params"], json!({ "sessionId": "p1", "configId": "model", "type": "id", "value": "haiku" }));
        let f = c.on_frame(&reply(4, json!({ "configOptions": opts_of("haiku", &["opus", "haiku", "sonnet"], None) })));
        assert_eq!(f[0]["params"]["value"], "sonnet");
        assert!(!c.probe_done);
        assert!(c.on_frame(&reply(5, json!({ "configOptions": opts_of("sonnet", &["opus", "haiku", "sonnet"], Some(("default", &["low", "medium"]))) }))).is_empty());
        assert!(c.probe_done);
        let m = c.models.clone().unwrap();
        assert_eq!((m.model.as_str(), m.effort.as_str(), m.error.as_str()), ("opus", "high", ""));
        let got: Vec<(String, String, Vec<String>)> = m.models.into_iter().map(|x| (x.id, x.name, x.efforts)).collect();
        let e = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(got, vec![
            ("opus".to_string(), "OPUS".to_string(), e(&["low", "high", "max"])),
            ("haiku".to_string(), "HAIKU".to_string(), vec![]),
            ("sonnet".to_string(), "SONNET".to_string(), e(&["low", "medium"])),
        ]);
        assert_eq!(c.close()[0]["method"], "session/close", "列完关掉会话");
        // 不支持 session/options 的 agent：写明原因
        let mut c = Client::new("/w", "", None).with_auth(AuthMode::Probe);
        c.start();
        c.on_frame(&reply(0, json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {} } } })));
        c.on_frame(&reply(1, json!({ "state": "unauthenticated" })));
        assert!(c.probe_done && c.models.unwrap().error.contains("session/options"));
    }

    /// 派发（#666）：v2、公布了 session.options 的 agent，模型和强度当 session/new 的初始值带上（先模型后强度）；回应里的值
    /// 和要的不一样（账号里没有、这个模型不支持）就出错结束，不按默认跑；对得上就照常往下
    #[test]
    fn dispatch_sends_model_and_effort_as_initial_values() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "session": { "options": {} } } });
        let start = || {
            let mut c = Client::new("/w", "做卡", None).with_sets(vec![("@thought_level".into(), "max".into()), ("@model".into(), "sonnet".into())]);
            c.start();
            assert_eq!(c.on_frame(&reply(0, init.clone()))[0]["method"], "session/options");
            let f = c.on_frame(&reply(1, json!({ "configOptions": opts_of("opus", &["opus", "sonnet", "haiku"], Some(("high", &["low", "high", "max"]))) })));
            assert_eq!(f[0]["method"], "session/new");
            assert_eq!(
                f[0]["params"]["configOptions"],
                json!([{ "configId": "model", "type": "id", "value": "sonnet" }, { "configId": "effort", "type": "id", "value": "max" }]),
                "先模型后强度"
            );
            c
        };
        let mut c = start();
        let f = c.on_frame(&reply(2, json!({ "sessionId": "s1", "configOptions": opts_of("sonnet", &["opus", "sonnet"], Some(("max", &["low", "max"]))) })));
        assert_eq!(f[0]["method"], "session/prompt", "设上了：不用再切");
        let mut c = start();
        c.on_frame(&reply(2, json!({ "sessionId": "s1", "configOptions": opts_of("opus", &["opus"], Some(("max", &["max"]))) })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.starts_with("账号里没有模型 sonnet")), "{:?}", c.outcome);
        let mut c = start();
        c.on_frame(&reply(2, json!({ "sessionId": "s1", "configOptions": opts_of("sonnet", &["sonnet"], None) })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.starts_with("模型 sonnet 不支持强度 max")), "{:?}", c.outcome);
        // 没公布 session.options（旧版适配器）：照旧开会话后切
        let mut c = Client::new("/w", "做卡", None).with_sets(vec![("@model".into(), "sonnet".into())]);
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 2, "capabilities": {} })));
        assert!(f[0]["params"].get("configOptions").is_none() && f[0]["method"] == "session/new");
    }

    /// 回归（#672）：Codex 全局 `model` 被网关删掉（这里还是 `gpt-6.2-sol`）时，用 provider 角色指定网关上另一个
    /// 存在的模型派发。旧版适配器先按全局默认开 `session/new`，网关 404，角色模型根本没机会生效；#666 之后
    /// `providers/set`、`session/options` 之后必须把角色模型当初始值带在 `session/new` 里，不能先按默认开会话再切。
    #[test]
    fn codex_role_model_replaces_a_global_default_missing_from_the_gateway() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "session": { "options": {} } } });
        let provider = json!({
            "providerId": "openai",
            "apiType": "openai",
            "baseUrl": "http://127.0.0.1:3425/v1",
            "headers": { "Authorization": "Bearer sk-test" }
        });
        let mut c = Client::new("/w", "做卡", None)
            .with_sets(vec![("@model".into(), "group/4.1f".into())])
            .with_provider(Some(provider));
        c.start();
        let f = c.on_frame(&reply(0, init));
        assert_eq!(f[0]["method"], "providers/set", "先给这一个进程换上角色的供应商");
        let f = c.on_frame(&reply(1, json!({})));
        assert_eq!(f[0]["method"], "session/options");
        // 网关里已经没有全局默认 gpt-6.2-sol，模型下拉里只有现在能用的 group/4.1f
        let f = c.on_frame(&reply(2, json!({
            "configOptions": opts_of("gpt-6.2-sol", &["group/4.1f", "auto/deepseek-v4.1-flash"], None)
        })));
        assert_eq!(f[0]["method"], "session/new");
        assert_eq!(
            f[0]["params"]["configOptions"],
            json!([{ "configId": "model", "type": "id", "value": "group/4.1f" }]),
            "开会话时就带上角色模型，不能先按全局默认 gpt-6.2-sol 开"
        );
        // 回应里实际用的就是角色模型：照常发提示词，不再开会话后切模型
        let f = c.on_frame(&reply(3, json!({
            "sessionId": "s1",
            "configOptions": opts_of("group/4.1f", &["group/4.1f"], None)
        })));
        assert_eq!(f[0]["method"], "session/prompt");
    }

    #[test]
    fn auth_status_payloads() {
        assert_eq!(auth_of_status(&json!({ "state": "unauthenticated" })).kind, "none");
        assert_eq!(auth_of_status(&json!({ "state": "unknown" })).kind, "unknown");
        // 登录了、却看不出种类（没有 claudeCode 的 _meta）：按不知道算
        let a = auth_of_status(&json!({ "state": "authenticated", "accounts": [{ "label": "Someone" }] }));
        assert_eq!((a.kind.as_str(), a.label.as_str()), ("unknown", "Someone"));
        assert_eq!(auth_of_ext(&json!({ "kind": "external", "label": "AWS Bedrock" })).words(), "AWS Bedrock");
        // 邮箱（#669）：推送的在 account.email，auth/status 的在账号的 email；组织名不留，说给人听的那句话里没有邮箱
        let a = auth_of_ext(&json!({ "kind": "account", "label": "Claude Max", "account": { "email": " Someone@Example.com ", "organization": "Org", "plan": "max" } }));
        assert_eq!((a.email.as_str(), a.account_key().as_deref(), a.words().as_str()), ("Someone@Example.com", Some("someone@example.com"), "Claude Max"));
        let a = auth_of_status(&json!({ "state": "authenticated", "accounts": [{ "label": "Claude Pro", "active": true, "email": "x@example.com", "_meta": { "claudeCode": { "kind": "account" } } }] }));
        assert_eq!(a.email, "x@example.com");
        assert_eq!(auth_of_ext(&json!({ "kind": "api_key", "label": "Anthropic API key", "detail": "ANTHROPIC_API_KEY" })).account_key(), None);
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--auth", "only", "--", "x"])).is_ok(), "only 不要提示词");
        assert_eq!(parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--auth", "subscription", "--", "x"])).unwrap().auth, AuthMode::Subscription);
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--auth", "x", "--", "x"])).unwrap_err().contains("--auth"));
    }
}
