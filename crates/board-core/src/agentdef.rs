//! 内置 agent 的描述（default/main#399）：一个 agent 一条，只在这里定义。
//!
//! 以前同一个事实拆在几处：名称在 board host 的内置模板、`providers.rs`、界面的两份名单里；读 skill 的目录在 board host、
//! `board skills`、桌面端、界面各写一份；支持 MCP 同步的名单在服务端、board host、桌面端、界面各一份。现在都从这里读：
//!
//! - board host 的内置模板（命令、安装、查找位置、skill 目录、Windows 启动环境、结构化输出参数）
//! - 认哪些供应商接口、配置文件能写哪几档强度（`providers.rs`，随 `GET /api/providers` 的 `meta` 给界面）
//! - 支不支持 MCP 同步、MCP 配置文件在哪（`mcp.rs`、board host 的 `mcpsync.rs`、桌面端「从本机导入」）
//! - 读用户级 skill 的目录（`board skills`、桌面端「一键安装」、界面）
//! - 界面上的主色（图标、动作这些画法留在界面里）
//!
//! 按 agent 写代码的只有两段，不在这里：怎么改它的配置文件（`providersync.rs`、`mcpsync.rs`）、怎么解析它的事件流
//! （`logfmt.rs`、`events.rs`）。加一个 agent：这里加一条，再看那两段要不要加它的写法（docs/agents.md「写成内置模板」）。

use std::path::{Path, PathBuf};

/// 一个内置 agent
#[derive(Debug)]
pub struct AgentDef {
    /// 标识（[`crate::hosts::check_agent_id`] 的规则）
    pub id: &'static str,
    pub name: &'static str,
    /// npm 包名（查最新版本、npm 全局安装时用）
    pub package: &'static str,
    /// npm dist-tag：dsh 的 headless 包 alpha/next 比 latest 新
    pub tag: &'static str,
    /// 没找到时给用户的安装命令；board host 也按这条命令在本机执行
    pub install: &'static str,
    /// 命令模板：非交互地做完一个任务就退出，不问权限；`{prompt}` 换成提示词、`{patch}` 换成
    /// 这次派发的 profile 补丁路径，都作为一个参数传入
    pub cmd: &'static [&'static str],
    /// 命令不在 PATH 里时再看这些位置（`~` 是用户目录）
    pub also: &'static [&'static str],
    /// 读用户级 skill 的目录（第一个是装 board-skills、同步 skills 的地方）：启动时看 `board-skills` 装没装
    pub skills: &'static [&'static str],
    /// 在工作目录（仓库根目录）里读 skill 的相对目录：放在这里也算装好
    pub project: &'static [&'static str],
    /// Windows 上启动时把 Git Bash 的目录放到 PATH 前面：pi 从 PATH 找 `bash.exe`，
    /// Git 装在非 `%ProgramFiles%` 位置时（例如 D:\Git）才能不落到 WSL 的 bash
    pub git_bash: bool,
    /// Windows 上支持的启动环境，顺序就是默认选择的顺序（`wsl:*` 是每个 WSL 发行版）；
    /// macOS、Linux 上都是登录 shell → 直接启动
    pub win: &'static [&'static str],
    /// 命令模板里打开结构化输出的参数从哪个版本起才有（`<命令> --version` 的版本号）：本机版本更低时去掉
    /// `json_flags`，按原始输出运行。空表示不看版本
    pub json_since: &'static str,
    pub json_flags: &'static [&'static str],
    /// 认哪些供应商接口（[`crate::providers::INTERFACES`] 里的标识）
    pub protocols: &'static [&'static str],
    /// 配置文件能写的思考强度，从低到高（配置文件的写法按各自 agent 核实过；[`crate::providers::ALL_EFFORTS`] 里的）
    pub efforts: &'static [&'static str],
    /// 支持 MCP 同步时它的用户级 MCP 配置文件（显示用的写法，`~`、`$CODEX_HOME` 开头）；不支持时为空
    pub mcp: Option<&'static str>,
    /// 界面上的主色
    pub color: &'static str,
}

/// 内置 agent，顺序就是默认 agent 的顺序：本机找到的第一个是默认（机器设置里可以改）
pub static AGENT_DEFS: [AgentDef; 4] = [
    AgentDef {
        id: "dsh",
        name: "DeepSeek Harness",
        package: "@deepseek-ai/dsh",
        tag: "next",
        install: "npm install -g @deepseek-ai/dsh@next",
        // 参数分两段：--profile、--patch 是 dsh 启动器的参数，必须放在应用参数前面；
        // --json、--permission-mode 和任务文本是 headless 的参数。--json 之后 dsh 启动器不再解析选项，
        // --patch 放在 --json 后面会被 headless 当成自己的参数，报 unknown option '--patch'
        cmd: &["dsh", "--profile", "headless", "--patch", "{patch}", "--json", "--permission-mode", "danger-full-access", "{prompt}"],
        // 桌面端没在「设置」里装命令行时，用 app 里自带的启动脚本
        also: &[
            "~/.local/bin/dsh",
            "/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh",
            "~/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh",
        ],
        skills: &["~/.agents/skills"],
        project: &[".agents/skills"],
        git_bash: false,
        win: &["pwsh", "powershell", "cmd"],
        // 按 0.2.2-rc.1 的 dsh-headless 核实过 --json；更旧的不认这个参数，去掉它按原始输出运行
        json_since: "0.2.2",
        json_flags: &["--json"],
        protocols: &["anthropic", "openai-chat", "openai-responses"],
        efforts: &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        mcp: None,
        color: "#4D6BFE",
    },
    AgentDef {
        id: "claude",
        name: "Claude Code",
        package: "@anthropic-ai/claude-code",
        tag: "latest",
        install: "npm install -g @anthropic-ai/claude-code@latest",
        // -p 默认只在结束时打印结果：用事件流，运行中每一步都写进日志（logfmt.rs 转写）
        cmd: &["claude", "-p", "{prompt}", "--dangerously-skip-permissions", "--output-format", "stream-json", "--verbose"],
        also: &["~/.local/bin/claude", "~/.claude/local/claude"],
        skills: &["~/.claude/skills"],
        // Claude Code 不读 .agents/skills
        project: &[".claude/skills"],
        git_bash: false,
        win: &["pwsh", "powershell", "git-bash", "wsl:*", "cmd"],
        json_since: "",
        json_flags: &[],
        protocols: &["anthropic"],
        efforts: &["low", "medium", "high", "xhigh", "max"],
        mcp: Some("~/.claude.json"),
        color: "#D97757",
    },
    AgentDef {
        id: "pi",
        name: "pi",
        package: "@mariozechner/pi-coding-agent",
        tag: "latest",
        install: "npm install -g @mariozechner/pi-coding-agent@latest",
        // -p 只在结束时打印最终回答：--mode json 同样跑完就退出，运行中写 JSONL 事件（logfmt.rs 转写）
        cmd: &["pi", "--mode", "json", "{prompt}"],
        also: &["~/.local/bin/pi"],
        skills: &["~/.agents/skills", "~/.pi/agent/skills"],
        project: &[".agents/skills", ".pi/skills"],
        git_bash: true,
        // pi 不支持 cmd
        win: &["git-bash", "wsl:*", "pwsh", "powershell"],
        json_since: "",
        json_flags: &[],
        protocols: &["anthropic", "openai-chat", "openai-responses"],
        efforts: &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        mcp: Some("~/.pi/agent/mcp.json"),
        color: "#E5A93F",
    },
    AgentDef {
        id: "codex",
        name: "Codex",
        package: "@openai/codex",
        tag: "latest",
        install: "npm install -g @openai/codex@latest",
        // exec --json：运行中把事件（命令、改文件、说的话、每一轮的用量）写成 JSONL（docs/exec.md）
        cmd: &["codex", "exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "{prompt}"],
        also: &["~/.local/bin/codex"],
        skills: &["$CODEX_HOME/skills"],
        project: &[".agents/skills"],
        git_bash: false,
        win: &["pwsh", "powershell", "git-bash", "wsl:*", "cmd"],
        json_since: "",
        json_flags: &[],
        // Codex 的 model_providers 走 Responses 接口（wire_api = "responses"）
        protocols: &["openai-responses"],
        efforts: &["minimal", "low", "medium", "high", "xhigh", "max"],
        mcp: Some("$CODEX_HOME/config.toml"),
        color: "#10A37F",
    },
];

/// 这个内置 agent 的描述；配置文件里新加的 agent 没有
pub fn find(id: &str) -> Option<&'static AgentDef> {
    AGENT_DEFS.iter().find(|d| d.id == id)
}

/// 是内置的 agent
pub fn is_builtin(id: &str) -> bool {
    find(id).is_some()
}

/// 内置 agent 的标识，按默认 agent 的顺序
pub fn ids() -> impl Iterator<Item = &'static str> {
    AGENT_DEFS.iter().map(|d| d.id)
}

/// 支持 MCP 同步的内置 agent（默认同步给它们），按默认 agent 的顺序
pub fn mcp_agents() -> Vec<&'static str> {
    AGENT_DEFS.iter().filter(|d| d.mcp.is_some()).map(|d| d.id).collect()
}

/// 读用户级 skill 的第一个目录（装 board-skills、同步 skills 的地方），显示用的写法；不认识的 agent 为空
pub fn user_skill_dir(id: &str) -> Option<&'static str> {
    find(id).and_then(|d| d.skills.first().copied())
}

/// 把描述里显示用的路径换成真实路径：`~` 是 `home`，`$CODEX_HOME` 是那个环境变量（`codex_home`，没设或为空时是
/// `~/.codex`）。不读环境：调用方传进来，测试里好换
pub fn expand_home(p: &str, home: &Path, codex_home: Option<&str>) -> PathBuf {
    if let Some(rest) = p.strip_prefix("$CODEX_HOME") {
        let base = codex_home.map(str::trim).filter(|c| !c.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
        return join(base, rest);
    }
    if let Some(rest) = p.strip_prefix('~') {
        return join(home.to_path_buf(), rest);
    }
    PathBuf::from(p)
}

fn join(mut base: PathBuf, rest: &str) -> PathBuf {
    for part in rest.split(['/', '\\']).filter(|s| !s.is_empty()) {
        base.push(part);
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 描述里每个 agent：标识合法、不重复，接口都是认识的三种，强度都在 7 档之内（从低到高），有 skill 目录
    #[test]
    fn every_builtin_agent_is_well_formed() {
        let mut seen = vec![];
        for d in &AGENT_DEFS {
            crate::hosts::check_agent_id(d.id).unwrap_or_else(|e| panic!("{}：{e}", d.id));
            assert!(!seen.contains(&d.id), "{} 写了两次", d.id);
            seen.push(d.id);
            assert!(!d.name.is_empty() && !d.cmd.is_empty() && !d.install.is_empty(), "{}", d.id);
            assert!(d.cmd.contains(&"{prompt}"), "{}：命令里要有 {{prompt}}", d.id);
            assert!(!d.protocols.is_empty(), "{}", d.id);
            for p in d.protocols {
                assert!(crate::providers::INTERFACES.iter().any(|(k, _)| k == p), "{} 的接口 {p} 不认识", d.id);
            }
            let rank = |e: &str| crate::providers::ALL_EFFORTS.iter().position(|x| *x == e);
            let ranks: Vec<usize> = d.efforts.iter().map(|e| rank(e).unwrap_or_else(|| panic!("{} 的强度 {e} 不在 7 档里", d.id))).collect();
            assert!(ranks.windows(2).all(|w| w[0] < w[1]), "{} 的强度要从低到高", d.id);
            assert!(!d.skills.is_empty() && !d.win.is_empty(), "{}", d.id);
            assert!(d.color.starts_with('#') && d.color.len() == 7, "{}", d.id);
        }
        assert_eq!(ids().collect::<Vec<_>>(), vec!["dsh", "claude", "pi", "codex"], "默认 agent 的顺序");
        assert_eq!(mcp_agents(), vec!["claude", "pi", "codex"]);
        // 随 GET /api/providers 给界面的说明：每条都从这里来
        let meta = crate::providers::agent_meta();
        assert_eq!(meta.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ids().collect::<Vec<_>>());
        let codex = meta.iter().find(|m| m.id == "codex").unwrap();
        assert_eq!((codex.skills.clone(), codex.mcp, codex.color.as_str()), (vec!["$CODEX_HOME/skills".to_string()], true, "#10A37F"));
        assert!(!meta[0].mcp, "dsh 不支持 MCP");
        assert_eq!(crate::providers::efforts_of("claude"), &["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(crate::providers::protocols_of("codex"), &["openai-responses"]);
        assert_eq!(crate::providers::agent_name("pi"), "pi");
        assert!(crate::providers::protocols_of("nope").is_empty());
    }

    #[test]
    fn expands_home_and_codex_home() {
        let h = Path::new("/home/me");
        assert_eq!(expand_home("~/.claude/skills", h, None), PathBuf::from("/home/me/.claude/skills"));
        assert_eq!(expand_home("$CODEX_HOME/skills", h, None), PathBuf::from("/home/me/.codex/skills"));
        assert_eq!(expand_home("$CODEX_HOME/skills", h, Some(" ")), PathBuf::from("/home/me/.codex/skills"), "空的当没设");
        assert_eq!(expand_home("$CODEX_HOME/config.toml", h, Some("/c")), PathBuf::from("/c/config.toml"));
        assert_eq!(expand_home("/abs/x", h, None), PathBuf::from("/abs/x"));
        assert_eq!(user_skill_dir("pi"), Some("~/.agents/skills"));
        assert_eq!(user_skill_dir("nope"), None);
    }
}
