//! Codex 按角色用供应商派发时，这一个进程用的模型目录（#726）。
//!
//! codex-acp 换了供应商（`providers/set`）后，模型下拉是 Codex 的模型目录（app-server 的 `model/list`）和网关
//! `/v1/models` 的交集（`narrowToListedModels`）：网关只列 `group/4.1f` 这类自定义模型时交集为空，设不上模型。
//! 以前能跑的机器靠的是 magpie 写进 `~/.codex/config.toml` 的 `model_catalog_json`。
//!
//! board host 不写 agent 的配置文件（#576），也不换 `CODEX_HOME`（会话、记忆等 sqlite 状态，同步的 MCP、skills 都在里面，
//! 机器对话的 `--resume` 也靠它）。每次派发：
//!
//! 1. 在启动环境里跑 `<codex> debug models --bundled`（Codex 自带的目录，不联网），按 [`entry`] 改出只有角色模型一条的目录；
//!    角色的模型本来就在自带目录里时不用改，照旧。
//! 2. 写一个包装脚本（[`wrapper`]），把 `CODEX_PATH` 换成它：codex-acp 每次启动 app-server（含 `providers/set` 之后的重启）
//!    都经 `CODEX_PATH`，脚本以 `-c model_catalog_json=<目录文件>` 加上原参数执行原来的 Codex。值不加引号：`-c` 的值解析
//!    不成 TOML 时按原文当字符串（路径里有空格、引号都不用转义）。原来的 Codex、目录文件经环境变量 [`BIN_ENV`]、
//!    [`MODELS_ENV`] 交给脚本，WSL 里的 Codex 由 WSLENV 的 `/p` 把 Windows 路径换成 WSL 路径。

use serde_json::{json, Value};

/// 包装脚本执行的、原来的 Codex（`CODEX_PATH` 原来的值）
pub const BIN_ENV: &str = "BOARD_CODEX_BIN";
/// 这次的模型目录文件
pub const MODELS_ENV: &str = "BOARD_CODEX_MODELS";

/// 按 Codex 自带的目录（`codex debug models --bundled` 的输出）给角色的模型生成一条目录。角色的模型本来就在自带目录里时
/// 为 None（不用包装）。
///
/// 模板取 `visibility` 为 `list`、`priority` 最小的那条（带着这个版本 Codex 的 `base_instructions`、工具设置），改：
/// `slug`、`display_name` 是角色的模型，`description` 清空，去掉指向官方模型的 `upgrade`、`availability_nux`；
/// `supported_reasoning_levels` 是 `efforts`（供应商给这个模型标的、Codex 能设的强度）；`default_reasoning_level`
/// 是角色的强度，没填时用模板的默认（在列表里时），否则列表第一个，列表为空时 `null`（Codex 请求里带 `effort: none`）。
/// `context_window` 等用模板的值：供应商不记上下文长度
pub fn entry(bundled: &str, model: &str, efforts: &[String], effort: &str) -> Result<Option<Value>, String> {
    // 登录 shell 可能在前后打别的字：从第一个 `{` 起取一个 JSON 值
    let from = bundled.find('{').map(|i| &bundled[i..]).unwrap_or(bundled);
    let d: Value = serde_json::Deserializer::from_str(from)
        .into_iter::<Value>()
        .next()
        .unwrap_or_else(|| serde_json::from_str::<Value>(""))
        .map_err(|e| format!("Codex 自带的模型目录解析不了：{e}"))?;
    let models = d["models"].as_array().ok_or("Codex 自带的模型目录里没有 models")?;
    if models.iter().any(|m| m["slug"].as_str() == Some(model)) {
        return Ok(None);
    }
    let template = models
        .iter()
        .filter(|m| m["visibility"].as_str() == Some("list") && m.is_object())
        .min_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX))
        .ok_or("Codex 自带的模型目录里没有可选的模型")?;
    let mut e = template.clone();
    let o = e.as_object_mut().expect("上面只取了对象");
    let default = if !effort.is_empty() && efforts.iter().any(|x| x == effort) {
        Some(effort.to_string())
    } else if let Some(t) = template["default_reasoning_level"].as_str().filter(|t| efforts.iter().any(|x| x == t)) {
        Some(t.to_string())
    } else {
        efforts.first().cloned()
    };
    o.insert("slug".into(), json!(model));
    o.insert("display_name".into(), json!(model));
    o.insert("description".into(), json!(""));
    o.remove("upgrade");
    o.remove("availability_nux");
    o.insert("supported_reasoning_levels".into(), Value::Array(efforts.iter().map(|x| json!({"effort": x, "description": ""})).collect()));
    o.insert("default_reasoning_level".into(), json!(default));
    Ok(Some(e))
}

/// 只有这一条的目录文件内容
pub fn catalog(entry: &Value) -> String {
    json!({ "models": [entry] }).to_string()
}

/// 包装脚本：`windows` 时是 `.cmd`（codex-acp 在 Windows 上用 cross-spawn 启动 `CODEX_PATH`，能直接起 `.cmd`，
/// 和 npm 的垫片一样把参数原样接在后面），否则是 sh（WSL 里也是）。内容只用 ASCII：cmd 按本地代码页读批处理
pub fn wrapper(windows: bool) -> String {
    if windows {
        format!(
            "@echo off\r\nrem Board: run Codex with this dispatch's model catalog (#726)\r\n\"%{BIN_ENV}%\" -c \"model_catalog_json=%{MODELS_ENV}%\" %*\r\nexit /b %ERRORLEVEL%\r\n"
        )
    } else {
        format!("#!/bin/sh\n# Board: run Codex with this dispatch's model catalog (#726)\nexec \"${BIN_ENV}\" -c \"model_catalog_json=${MODELS_ENV}\" \"$@\"\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled() -> String {
        json!({"models": [
            {"slug": "hidden-x", "visibility": "hide", "priority": 0, "base_instructions": "hidden"},
            {"slug": "gpt-b", "visibility": "list", "priority": 2, "base_instructions": "B", "default_reasoning_level": "medium"},
            {"slug": "gpt-a", "visibility": "list", "priority": 1, "base_instructions": "A", "context_window": 400000,
             "default_reasoning_level": "medium", "supported_reasoning_levels": [{"effort": "medium", "description": "m"}],
             "upgrade": {"model": "gpt-z"}, "availability_nux": {"message": "new"}, "description": "官方的"}
        ]})
        .to_string()
    }

    fn sv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 自定义模型：模板是可选的里 priority 最小的，改 slug、名称、强度，去掉指向官方模型的提示，别的照模板
    #[test]
    fn entry_from_template_for_custom_model() {
        let e = entry(&bundled(), "group/4.1f", &sv(&["low", "high", "max"]), "max").unwrap().unwrap();
        assert_eq!(e["slug"], "group/4.1f");
        assert_eq!(e["display_name"], "group/4.1f");
        assert_eq!(e["description"], "");
        assert_eq!(e["base_instructions"], "A", "模板是 visibility=list 里 priority 最小的");
        assert_eq!(e["context_window"], 400000);
        assert!(e.get("upgrade").is_none() && e.get("availability_nux").is_none());
        assert_eq!(e["supported_reasoning_levels"], json!([{"effort": "low", "description": ""}, {"effort": "high", "description": ""}, {"effort": "max", "description": ""}]));
        assert_eq!(e["default_reasoning_level"], "max", "默认强度是角色的强度");
        assert_eq!(catalog(&e), json!({"models": [e]}).to_string());
    }

    /// 默认强度：角色没填时用模板的（在列表里时），否则列表第一个；没标强度时列表为空、默认为 null
    #[test]
    fn entry_default_effort() {
        let e = entry(&bundled(), "m", &sv(&["low", "medium"]), "").unwrap().unwrap();
        assert_eq!(e["default_reasoning_level"], "medium");
        let e = entry(&bundled(), "m", &sv(&["low", "high"]), "").unwrap().unwrap();
        assert_eq!(e["default_reasoning_level"], "low");
        let e = entry(&bundled(), "m", &sv(&["low", "high"]), "xhigh").unwrap().unwrap();
        assert_eq!(e["default_reasoning_level"], "low", "角色的强度不在列表里：不当默认（--set 会报设不上）");
        let e = entry(&bundled(), "m", &[], "").unwrap().unwrap();
        assert_eq!(e["supported_reasoning_levels"], json!([]));
        assert_eq!(e["default_reasoning_level"], Value::Null);
    }

    /// 角色的模型本来就在自带目录里（网关转的是 Codex 自己的模型）：不用包装，照旧
    #[test]
    fn entry_none_for_bundled_model() {
        assert_eq!(entry(&bundled(), "gpt-b", &sv(&["high"]), "high").unwrap(), None);
        assert_eq!(entry(&bundled(), "hidden-x", &[], "").unwrap(), None);
    }

    /// 解析不了、没有可选的模型：出错（派发不按默认模型跑）
    #[test]
    fn entry_errors() {
        assert!(entry("not json", "m", &[], "").unwrap_err().contains("解析不了"));
        assert!(entry("", "m", &[], "").unwrap_err().contains("解析不了"));
        // 登录 shell 前后多打的字不影响
        let noisy = format!("Last login: today\n{}\nlogout\n", bundled());
        assert_eq!(entry(&noisy, "m", &[], "").unwrap().unwrap()["slug"], "m");
        assert!(entry("{}", "m", &[], "").unwrap_err().contains("没有 models"));
        assert!(entry(r#"{"models":[{"slug":"h","visibility":"hide"}]}"#, "m", &[], "").unwrap_err().contains("没有可选的模型"));
    }

    /// 包装脚本：原来的 Codex、目录文件都从环境变量取；Windows 是 CRLF 的 .cmd，只有 ASCII
    #[test]
    fn wrapper_scripts() {
        let sh = wrapper(false);
        assert!(sh.starts_with("#!/bin/sh\n"));
        assert!(sh.contains(r#"exec "$BOARD_CODEX_BIN" -c "model_catalog_json=$BOARD_CODEX_MODELS" "$@""#), "{sh}");
        let cmd = wrapper(true);
        assert!(cmd.starts_with("@echo off\r\n") && cmd.ends_with("exit /b %ERRORLEVEL%\r\n"));
        assert!(cmd.contains(r#""%BOARD_CODEX_BIN%" -c "model_catalog_json=%BOARD_CODEX_MODELS%" %*"#), "{cmd}");
        assert!(sh.is_ascii() && cmd.is_ascii());
    }

    /// sh 包装脚本真的这样执行：-c 在前，原参数（带空格的也）原样接在后面
    #[cfg(unix)]
    #[test]
    fn sh_wrapper_runs_codex_with_catalog() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("fake codex");
        std::fs::write(&fake, "#!/bin/sh\nfor a in \"$@\"; do echo \"[$a]\"; done\n").unwrap();
        let script = d.path().join("7.codex");
        std::fs::write(&script, wrapper(false)).unwrap();
        for f in [&fake, &script] {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let models = d.path().join("it's here").join("7.codex-models.json");
        let out = std::process::Command::new(&script)
            .args(["app-server", "-c", "model_provider=\"openai\"", "a b"])
            .env(BIN_ENV, &fake)
            .env(MODELS_ENV, &models)
            .output()
            .unwrap();
        let got = String::from_utf8_lossy(&out.stdout);
        assert_eq!(got, format!("[-c]\n[model_catalog_json={}]\n[app-server]\n[-c]\n[model_provider=\"openai\"]\n[a b]\n", models.display()));
    }
}
