//! 内置 agent 的描述（default/main#399）：一个 agent 一条，只在这里定义。
//!
//! 以前同一个事实拆在几处：名称在 board host 的内置模板、`providers.rs`、界面的两份名单里；读 skill 的目录在 board host、
//! `board skills`、桌面端、界面各写一份；支持 MCP 同步的名单在服务端、board host、桌面端、界面各一份。现在都从这里读：
//!
//! - board host 的内置模板（命令、ACP 入口、安装、查找位置、skill 目录、Windows 启动环境）
//! - 认哪些供应商接口、配置文件能写哪几档强度（`providers.rs`，随 `GET /api/providers` 的 `meta` 给界面）
//! - 支不支持 MCP 同步、MCP 配置文件在哪（`mcp.rs`、board host 的 `mcpsync.rs`、桌面端「从本机导入」）
//! - 读用户级 skill 的目录（`board skills`、桌面端「一键安装」、界面）
//! - 界面上的主色（图标、动作这些画法留在界面里）
//!
//! board host 用 ACP 驱动所有 agent（`board acp`，见 board-cli 的 acp.rs），日志和「过程」只解析一套 ACP，不按 agent 分。
//! 按 agent 写代码的只剩怎么改它的配置文件（`providersync.rs`、`mcpsync.rs`）。加一个 agent：这里加一条，
//! 再看那两处要不要加它的写法（docs/agents.md「写成内置模板」）。

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
    /// agent 自己的命令：找命令、取版本号、判断安装方式、显示版本用
    pub bin: &'static str,
    /// ACP 入口：`board acp` 在这次派发的启动环境里执行的命令，讲 ACP（标准输入、输出）。`{bin}` 换成找到的 agent 命令，
    /// `{patch}` 换成这次派发的 profile 补丁路径。agent 自己不讲 ACP 的，用 ACP 组织维护的适配器，`npx -y` 固定版本运行：
    /// 不用另外安装，升级跟着 Board 发版
    pub acp: &'static [&'static str],
    /// 告诉适配器用哪个 agent 的环境变量，值是 board host 找到的 agent 路径（适配器就用机器上装的、Board 管着升级的那份，
    /// 不用它自带的）。ACP 入口就是 agent 自己时为空
    pub acp_bin_env: &'static str,
    /// Windows 上 npm 全局装的这个 agent，包里真正的程序（`.exe`）在哪：相对 npm 的全局前缀（`PATH` 里找到的垫片所在的
    /// 目录），`/` 分段，`*` 匹配一段里的其余部分。找到的是 npm 的垫片（无扩展名的 sh 脚本、`.cmd`、`.ps1`，适配器执行不了）
    /// 时按这些位置找，传给适配器的是它（#456）；都找不到就不传，适配器用它自带的那份
    pub npm_exe: &'static [&'static str],
    /// 另外传给 ACP 入口的环境变量
    pub acp_env: &'static [(&'static str, &'static str)],
    /// 会话开始后切到的模式（不问权限）；agent 没公布这个模式时不切，权限请求照样自动允许。空表示不切
    pub acp_mode: &'static str,
    /// agent 自己的 ACP 从哪个版本起才有（`<命令> --version` 的版本号）：本机版本更旧时不启动，提示升级。空表示不看版本
    pub acp_since: &'static str,
    /// 命令不在 PATH 里时再看这些位置（`~` 是用户目录）。第一项是 board host 装进用户目录的位置：
    /// PATH 里还有旧版本时比较版本号，用较新的那个
    pub also: &'static [&'static str],
    /// 读用户级 skill 的目录（第一个是装 board-skills、同步 skills 的地方）：启动时看 `board-skills` 装没装
    pub skills: &'static [&'static str],
    /// 在工作目录（仓库根目录）里读 skill 的相对目录：放在这里也算装好
    pub project: &'static [&'static str],
    /// Windows 上支持的启动环境，顺序就是默认选择的顺序（`wsl:*` 是每个 WSL 发行版）；
    /// macOS、Linux 上都是登录 shell → 直接启动
    pub win: &'static [&'static str],
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
pub static AGENT_DEFS: [AgentDef; 3] = [
    AgentDef {
        id: "dsh",
        name: "DeepSeek Harness",
        package: "@deepseek-ai/dsh",
        tag: "next",
        install: "npm install -g @deepseek-ai/dsh@next",
        bin: "dsh",
        // dsh 自带 ACP：acp profile（`@deepseek-ai/dsh-acp-app`，0.2.2-rc.1 起）。--patch 关掉 goal 的自动续轮
        acp: &["{bin}", "--profile", "acp", "--patch", "{patch}"],
        acp_bin_env: "",
        npm_exe: &[],
        // 不开沙箱、不审批（和以前 headless 的 --permission-mode danger-full-access 一样）：acp profile 按这个环境变量定
        acp_env: &[("DSH_PERMISSION_MODE", "danger-full-access")],
        // 没有可切的模式；权限请求自动允许
        acp_mode: "",
        acp_since: "0.2.2",
        // 桌面端没在「设置」里装命令行时，用 app 里自带的启动脚本
        also: &[
            "~/.local/bin/dsh",
            "/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh",
            "~/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh",
        ],
        skills: &["~/.agents/skills"],
        project: &[".agents/skills"],
        win: &["pwsh", "powershell", "cmd"],
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
        bin: "claude",
        // ACP 组织维护的适配器（Claude Agent SDK）；CLAUDE_CODE_EXECUTABLE 指向机器上的 claude
        acp: &["npx", "-y", "@agentclientprotocol/claude-agent-acp@0.107.3"],
        acp_bin_env: "CLAUDE_CODE_EXECUTABLE",
        // npm 包的 bin 是 bin/claude.exe：postinstall 把平台包里的原生程序换上去（2.1.326 核对过，包里没有 cli.js）
        npm_exe: &["node_modules/@anthropic-ai/claude-code/bin/claude.exe"],
        // 后台任务照常用：走 ACP v2，idle 时报 background，board acp 等后台跑完、Claude Code 自己接着做（#461）
        acp_env: &[],
        acp_mode: "bypassPermissions",
        acp_since: "",
        also: &["~/.local/bin/claude", "~/.claude/local/claude"],
        skills: &["~/.claude/skills"],
        // Claude Code 不读 .agents/skills
        project: &[".claude/skills"],
        win: &["pwsh", "powershell", "git-bash", "wsl:*", "cmd"],
        protocols: &["anthropic"],
        efforts: &["low", "medium", "high", "xhigh", "max"],
        mcp: Some("~/.claude.json"),
        color: "#D97757",
    },
    AgentDef {
        id: "codex",
        name: "Codex",
        package: "@openai/codex",
        tag: "latest",
        install: "npm install -g @openai/codex@latest",
        bin: "codex",
        // ACP 组织维护的适配器（Codex App Server）；CODEX_PATH 指向机器上的 codex
        acp: &["npx", "-y", "@agentclientprotocol/codex-acp@2.7.0"],
        acp_bin_env: "CODEX_PATH",
        // npm 包的 bin 是 bin/codex.js（node 再启动原生程序）；原生程序在平台包 @openai/codex-win32-<架构> 的 vendor/<目标>/bin 里，
        // 平台包装在 @openai/codex 自己的 node_modules 里，或者被 npm 提到上一层
        npm_exe: &[
            "node_modules/@openai/codex/node_modules/@openai/codex-win32-*/vendor/*/bin/codex.exe",
            "node_modules/@openai/codex-win32-*/vendor/*/bin/codex.exe",
        ],
        acp_env: &[("INITIAL_AGENT_MODE", "agent-full-access")],
        acp_mode: "agent-full-access",
        acp_since: "",
        also: &["~/.local/bin/codex"],
        skills: &["$CODEX_HOME/skills"],
        project: &[".agents/skills"],
        win: &["pwsh", "powershell", "git-bash", "wsl:*", "cmd"],
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
            assert!(!d.name.is_empty() && !d.bin.is_empty() && !d.acp.is_empty() && !d.install.is_empty(), "{}", d.id);
            assert!(!d.acp.iter().any(|a| a.contains("{prompt}")), "{}：提示词走 session/prompt，ACP 入口里不带 {{prompt}}", d.id);
            // 用适配器的要固定版本（npx -y <包>@<版本>），升级跟着 Board 发版
            if d.acp[0] == "npx" {
                assert!(d.acp.iter().any(|a| a.rfind('@').is_some_and(|i| i > 0)), "{}：适配器要固定版本", d.id);
                assert!(!d.acp_bin_env.is_empty(), "{}：要告诉适配器用机器上的 agent", d.id);
                // Windows 上 npm 的垫片适配器执行不了：要写明包里真正的程序在哪（#456）
                assert!(!d.npm_exe.is_empty(), "{}：Windows 上要能找到 npm 包里的 .exe", d.id);
                for p in d.npm_exe {
                    assert!(p.starts_with("node_modules/") && p.ends_with(".exe") && !p.contains('\\'), "{}：{p}", d.id);
                }
            } else {
                assert_eq!(d.acp[0], "{bin}", "{}：不用适配器时入口就是 agent 自己", d.id);
            }
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
        assert_eq!(ids().collect::<Vec<_>>(), vec!["dsh", "claude", "codex"], "默认 agent 的顺序");
        assert_eq!(mcp_agents(), vec!["claude", "codex"]);
        // 随 GET /api/providers 给界面的说明：每条都从这里来
        let meta = crate::providers::agent_meta();
        assert_eq!(meta.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ids().collect::<Vec<_>>());
        let codex = meta.iter().find(|m| m.id == "codex").unwrap();
        assert_eq!((codex.skills.clone(), codex.mcp, codex.color.as_str()), (vec!["$CODEX_HOME/skills".to_string()], true, "#10A37F"));
        assert!(!meta[0].mcp, "dsh 不支持 MCP");
        assert_eq!(crate::providers::efforts_of("claude"), &["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(crate::providers::protocols_of("codex"), &["openai-responses"]);
        assert_eq!(crate::providers::agent_name("codex"), "Codex");
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
        assert_eq!(user_skill_dir("dsh"), Some("~/.agents/skills"));
        assert_eq!(user_skill_dir("nope"), None);
    }
}
