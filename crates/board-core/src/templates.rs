//! agent 模板共用配置（PRD §9.1「本机 agent」、§10.12「共用配置」，卡片 default/main#188）。
//!
//! 服务端存一份 agent 模板（自定义 agent，或者覆盖内置模板的命令），随任务（`HostWork.templates`）下发给所有 board host：
//! 它们把模板合进本机 agent（内置 → 这里的模板 → 本机配置文件 `[agents.<标识>]`，后面的覆盖前面的），重新检查后上报，
//! 不用重启。某台机器本地配置文件里有同标识的 `[agents.*]` 时以本地为准，上报里标出「本机配置覆盖」。
//! 改了模板算机器有变化：在线的 board host 收到推送马上重新取任务。

use crate::hosts::{check_agent_id, check_model};
use crate::store::{R, W};
use crate::{BoardError, ErrorCode, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA: &str = r#"
-- agent 模板共用配置
CREATE TABLE IF NOT EXISTS agent_templates (
  id         TEXT PRIMARY KEY,
  spec       TEXT NOT NULL,             -- JSON：AgentTemplate
  version    TEXT NOT NULL,             -- 内容的哈希
  updated_at INTEGER NOT NULL,
  by_name    TEXT NOT NULL
);
"#;

/// Windows 上能选的启动环境（`wsl:*` 是每个 WSL 发行版）
pub const WIN_ENVS: [&str; 6] = ["pwsh", "powershell", "git-bash", "wsl:*", "cmd", "direct"];

/// 一个 agent 模板：字段和配置文件的 `[agents.<标识>]` 一样（docs/agents.md「用配置文件接上」）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTemplate {
    /// 标识：同标识的覆盖内置模板
    pub id: String,
    /// 名称；空时用标识（覆盖内置模板时用内置的名称）
    #[serde(default)]
    pub name: String,
    /// 命令模板：第一个是命令，`{prompt}` 换成提示词（作为一个参数传入）
    pub cmd: Vec<String>,
    /// 派发名字的第二段（模型名）；空时用内置模板的默认值，再没有用标识
    #[serde(default)]
    pub model: String,
    /// 选了模型时插到 `{prompt}` 前面的参数（`{model}` 换成模型）；空表示不能选模型（覆盖内置模板时用内置的）
    #[serde(default)]
    pub model_args: Vec<String>,
    /// 选了思考强度时插的参数（`{effort}` 换成档位）；空表示不能选（覆盖内置模板时用内置的）
    #[serde(default)]
    pub effort_args: Vec<String>,
    /// 能选的思考强度，从低到高；空时是 low、medium、high
    #[serde(default)]
    pub efforts: Vec<String>,
    /// Windows 上支持的启动环境，按默认选择的顺序；空时新加的 agent 用配置文件 agent 的默认顺序，覆盖内置模板的不变
    #[serde(default)]
    pub win: Vec<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub by: String,
}

impl AgentTemplate {
    /// 内容的版本号（不含版本号和元数据本身）
    pub fn content_version(&self) -> String {
        let body = serde_json::json!({
            "id": self.id, "name": self.name, "cmd": self.cmd, "model": self.model, "modelArgs": self.model_args,
            "effortArgs": self.effort_args, "efforts": self.efforts, "win": self.win,
        });
        Sha256::digest(body.to_string().as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
    }

    /// 去掉首尾空白、空的参数
    fn tidy(&mut self) {
        let clean = |v: &mut Vec<String>| {
            for x in v.iter_mut() {
                *x = x.trim().to_string();
            }
            v.retain(|x| !x.is_empty());
        };
        self.id = self.id.trim().to_string();
        self.name = self.name.trim().to_string();
        self.model = self.model.trim().to_string();
        for v in [&mut self.cmd, &mut self.model_args, &mut self.effort_args, &mut self.efforts, &mut self.win] {
            clean(v);
        }
    }

    /// 校验：标识、命令里正好一个 `{prompt}`（不在第一个）、参数写法、强度档位、启动环境
    pub fn check(&self) -> Result<()> {
        check_agent_id(&self.id).map_err(BoardError::usage)?;
        if self.name.chars().count() > 32 {
            return Err(BoardError::usage("名称最长 32 个字符"));
        }
        let Some(first) = self.cmd.first() else { return Err(BoardError::usage("命令模板不能为空：第一个是命令，再写一个 {prompt}")) };
        if first.contains("{prompt}") {
            return Err(BoardError::usage("命令模板的第一个要是命令本身，{prompt} 写在后面"));
        }
        let n = self.cmd.iter().filter(|a| a.contains("{prompt}")).count();
        if n != 1 {
            return Err(BoardError::usage(format!("命令模板里要正好有一个 {{prompt}}（现在有 {n} 个）")));
        }
        if self.cmd.iter().any(|a| a.contains('\n') || a.chars().count() > 500) {
            return Err(BoardError::usage("命令模板的每个参数只能一行，最长 500 个字符"));
        }
        if !self.model.is_empty() {
            check_model(&self.model).map_err(|e| BoardError::usage(format!("默认模型：{e}")))?;
        }
        if !self.model_args.is_empty() && !self.model_args.iter().any(|a| a.contains("{model}")) {
            return Err(BoardError::usage("传模型的参数里要有 {model}，例如 --model {model}"));
        }
        if !self.effort_args.is_empty() && !self.effort_args.iter().any(|a| a.contains("{effort}")) {
            return Err(BoardError::usage("传思考强度的参数里要有 {effort}，例如 --effort {effort}"));
        }
        for e in &self.efforts {
            if e.chars().count() > 16 || !e.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(BoardError::usage(format!("思考强度「{e}」不对：只能用字母、数字、-、_，最长 16 个字符")));
            }
        }
        let mut seen: Vec<&str> = vec![];
        for w in &self.win {
            if !WIN_ENVS.contains(&w.as_str()) {
                return Err(BoardError::usage(format!("启动环境「{w}」不对：只能是 {}", WIN_ENVS.join("、"))));
            }
            if seen.contains(&w.as_str()) {
                return Err(BoardError::usage(format!("启动环境「{w}」写了两次")));
            }
            seen.push(w);
        }
        Ok(())
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTemplate> {
    let spec: String = r.get(1)?;
    let mut t: AgentTemplate = serde_json::from_str(&spec).unwrap_or_default();
    t.id = r.get(0)?;
    t.version = r.get(2)?;
    t.updated_at = r.get(3)?;
    t.by = r.get(4)?;
    Ok(t)
}

impl R<'_> {
    /// 全部 agent 模板，按标识
    pub fn agent_templates(&self) -> Result<Vec<AgentTemplate>> {
        let mut st = self.conn.prepare("SELECT id, spec, version, updated_at, by_name FROM agent_templates ORDER BY id")?;
        let rows = st.query_map([], row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn agent_template(&self, id: &str) -> Result<Option<AgentTemplate>> {
        Ok(self.conn.query_row("SELECT id, spec, version, updated_at, by_name FROM agent_templates WHERE id = ?1", [id], row).optional()?)
    }
}

impl W<'_> {
    /// 新增或修改一个模板。内容没变时什么也不做
    pub fn template_put(&mut self, mut t: AgentTemplate, by: &str) -> Result<AgentTemplate> {
        t.tidy();
        t.check()?;
        t.version = t.content_version();
        t.updated_at = self.now;
        t.by = if by.trim().is_empty() { "人".into() } else { by.trim().into() };
        let old = self.read().agent_template(&t.id)?;
        if let Some(o) = old.filter(|o| o.version == t.version) {
            return Ok(o);
        }
        self.tx.execute(
            "INSERT INTO agent_templates (id, spec, version, updated_at, by_name) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET spec = ?2, version = ?3, updated_at = ?4, by_name = ?5",
            params![t.id, serde_json::to_string(&t).unwrap_or_default(), t.version, t.updated_at, t.by],
        )?;
        self.hosts_changed = true;
        self.changed = true;
        Ok(t)
    }

    /// 删掉一个模板：各台机器重新检查后，新加的 agent 不再出现，覆盖的内置模板换回内置的命令
    pub fn template_rm(&mut self, id: &str) -> Result<()> {
        if self.tx.execute("DELETE FROM agent_templates WHERE id = ?1", [id])? == 0 {
            return Err(BoardError::new(ErrorCode::NotFound, format!("没有 agent 模板「{id}」")));
        }
        self.hosts_changed = true;
        self.changed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Origin, Store};

    fn fake(id: &str) -> AgentTemplate {
        AgentTemplate { id: id.into(), name: "假 agent".into(), cmd: vec!["/tmp/fake.sh".into(), "{prompt}".into()], ..Default::default() }
    }

    #[test]
    fn templates_are_checked_saved_and_removed() {
        let mut s = Store::open_in_memory().unwrap();
        let put = |s: &mut Store, t: AgentTemplate| s.write(1000, Origin::ui(None), |w| w.template_put(t, "octocat"));
        let out = put(&mut s, AgentTemplate { cmd: vec![" /tmp/fake.sh ".into(), "".into(), "{prompt}".into()], ..fake("kimi") }).unwrap();
        assert_eq!(out.cmd, ["/tmp/fake.sh", "{prompt}"], "去掉空白和空参数");
        assert_eq!((out.by.as_str(), out.updated_at), ("octocat", 1000));
        assert!(!out.version.is_empty());
        // 内容没变：版本、时间都不变
        let again = s.write(2000, Origin::ui(None), |w| w.template_put(fake("kimi"), "x")).unwrap();
        assert_eq!((again.version, again.updated_at), (out.version.clone(), 1000));
        assert_eq!(s.read(|r| r.agent_templates()).unwrap().len(), 1);
        // 校验
        let bad = |t: AgentTemplate| put(&mut Store::open_in_memory().unwrap(), t).unwrap_err().message;
        assert!(bad(fake("Kimi")).contains("标识"), "{}", bad(fake("Kimi")));
        assert!(bad(AgentTemplate { cmd: vec!["x".into()], ..fake("a") }).contains("正好有一个 {prompt}"));
        assert!(bad(AgentTemplate { cmd: vec!["{prompt}".into()], ..fake("a") }).contains("第一个要是命令"));
        assert!(bad(AgentTemplate { model_args: vec!["--model".into()], ..fake("a") }).contains("{model}"));
        assert!(bad(AgentTemplate { effort_args: vec!["--effort".into()], ..fake("a") }).contains("{effort}"));
        assert!(bad(AgentTemplate { win: vec!["bash".into()], ..fake("a") }).contains("启动环境"));
        assert!(bad(AgentTemplate { win: vec!["cmd".into(), "cmd".into()], ..fake("a") }).contains("两次"));
        assert!(bad(AgentTemplate { efforts: vec!["very high".into()], ..fake("a") }).contains("思考强度"));
        // 删掉
        s.write(3000, Origin::ui(None), |w| w.template_rm("kimi")).unwrap();
        assert!(s.read(|r| r.agent_templates()).unwrap().is_empty());
        assert_eq!(s.write(3000, Origin::ui(None), |w| w.template_rm("kimi")).unwrap_err().code, ErrorCode::NotFound);
    }
}
