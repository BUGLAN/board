//! board host 的单元测试（各模块共用一份）。

use super::*;

fn managed_spec(id: &str, b: &'static Builtin) -> Spec {
    Spec {
        id: id.into(),
        name: b.name.into(),
        model: String::new(),
        bin: b.bin.into(),
        acp: b.acp.iter().map(|s| s.to_string()).collect(),
        acp_bin_env: b.acp_bin_env.into(),
        acp_env: vec![],
        acp_mode: b.acp_mode.into(),
        legacy: false,
        skills: vec![],
        project: vec![],
        checks: vec![],
        no_skill: BTreeMap::new(),
        builtin: Some(b),
        managed: true,
        latest: String::new(),
        checked_at: 0,
        manage_state: String::new(),
        manage_error: String::new(),
    }
}

/// 派发的配置文件：服务端地址、令牌，以及带给 agent 的 `BOARD_*`（`[agent_env]`），agent 从里面补回来
#[test]
fn run_config_carries_the_agent_variables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("7.toml");
    let vars = vec![
        ("BOARD_AS".to_string(), "codex-win/codex".to_string()),
        ("BOARD".to_string(), "oss/main".to_string()),
        ("BOARD_CONFIG".to_string(), path.display().to_string()),
    ];
    write_run_config(&path, "http://127.0.0.1:7493", "tok", &vars).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("url = \"http://127.0.0.1:7493\"") && text.contains("token = \"tok\""), "{text}");
    assert!(text.contains("[agent_env]") && text.contains("BOARD_AS = \"codex-win/codex\""), "{text}");
    // 同名文件再写一次：旧的 agent_env 换掉，不叠加
    write_run_config(&path, "http://127.0.0.1:7493", "tok", &vars[..1]).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("BOARD_AS") && !text.contains("BOARD_CONFIG"), "{text}");
}

fn git(dir: &Path, args: &[&str]) {
    let ok = crate::gitcmd::command()
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// 去掉 Windows 上 `canonicalize()` 加的 `\\?\` 前缀：git 不把带前缀的路径当本地路径
/// （`git clone \\?\C:\…` 会报「hostname contains invalid characters」）
fn plain(p: &Path) -> PathBuf {
    match p.to_string_lossy().strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p.to_path_buf(),
    }
}

/// 派发被服务端结束后找没推的改动：没提交的文件、不在远端的提交；agent 的 worktree 按 board-skills 的约定找
#[test]
fn leftovers_find_unpushed_changes_in_skill_worktree() {
    let t = tempfile::tempdir().unwrap();
    let root = plain(&t.path().canonicalize().unwrap());
    let remote = root.join("remote.git");
    std::fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let repo = root.join("board");
    git(&root, &["clone", "-q", remote.to_str().unwrap(), "board"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "a"]);
    git(&repo, &["push", "-q", "origin", "HEAD:main"]);
    assert!(unpushed(&repo).is_none(), "都推了");
    let wt = skill_worktree(&repo, "claude-mbp/opus-5.5").unwrap();
    assert_eq!(wt, root.join("board.wt").join("claude-mbp-opus-5.5"));
    git(&repo, &["worktree", "add", "-q", "--detach", wt.to_str().unwrap(), "HEAD"]);
    assert_eq!(skill_worktree(&wt, "x/y").unwrap(), root.join("board.wt").join("x-y"), "在 worktree 里也以主仓库为准");
    assert!(leftovers(&repo, "claude-mbp/opus-5.5").is_empty());
    // worktree 里改了没提交、提交了没推
    std::fs::write(wt.join("b.txt"), "b").unwrap();
    assert_eq!(unpushed(&wt).as_deref(), Some("1 个文件没提交"));
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "b"]);
    let got = leftovers(&repo, "claude-mbp/opus-5.5");
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, wt);
    assert!(got[0].1.starts_with("提交 ") && got[0].1.ends_with("不在任何远端分支上"), "{}", got[0].1);
    // 推到 wip 分支之后就不算了
    git(&wt, &["push", "-q", "origin", "HEAD:refs/heads/wip/main/1"]);
    git(&wt, &["fetch", "-q", "origin"]);
    assert!(leftovers(&repo, "claude-mbp/opus-5.5").is_empty());
    // 不是 git 仓库、不存在：不算
    assert!(unpushed(&root).is_none());
    assert!(unpushed(&root.join("nope")).is_none());
}

#[test]
fn offline_note_says_what_happens_to_running_agents() {
    assert_eq!(offline_note(Duration::from_secs(31), 0), "和服务端断开 31 秒了（继续重试）");
    let s = offline_note(Duration::from_secs(30), 2);
    assert!(s.starts_with("和服务端断开 30 秒了：2 个 agent 还在跑"), "{s}");
    assert!(s.contains("50 秒") && s.contains("board hold"), "{s}");
}

/// 卡片 #199：超过 50 MB 的本机日志只留开头 1 MB、最后 10 MB，中间一行省略说明；
/// 不超过的不动
#[test]
fn huge_logs_keep_head_and_tail_with_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("1.log");
    // 20 MB，每行 100 字节：全留下就太占地方
    let line = format!("{}\n", "x".repeat(99));
    let mut body = String::new();
    body.push_str("HEAD-MARK\n");
    while body.len() < 20 * 1024 * 1024 {
        body.push_str(&line);
    }
    body.push_str("TAIL-MARK\n");
    std::fs::write(&f, &body).unwrap();
    let max = 5 * 1024 * 1024;
    let (head, tail) = (1024 * 1024, 2 * 1024 * 1024);
    let len = truncate_log(&f, head, tail, max).unwrap().expect("超过 50 MB 才截断");
    let got = std::fs::read_to_string(&f).unwrap();
    assert_eq!(got.len() as u64, len, "返回的长度和写回去的一致");
    assert!(got.starts_with("HEAD-MARK\n"), "开头留着");
    assert!(got.ends_with("TAIL-MARK\n"), "结尾留着");
    assert!(got.contains("中间") && got.contains("已省略"), "有一行省略说明：{got:.0}");
    assert!((got.len() as u64) < 4 * 1024 * 1024, "截断后小了很多：{}", got.len());
    // 刚好到上限：不截断
    let small = dir.path().join("2.log");
    std::fs::write(&small, vec![b'a'; max as usize]).unwrap();
    assert_eq!(truncate_log(&small, head, tail, max).unwrap(), None);
    assert_eq!(std::fs::metadata(&small).unwrap().len(), max);
}

/// 卡片 #199：结束超过 30 天的本机日志在 board host 启动后被删除，30 天内的还在；
/// 正在跑的派发的日志不删；目录里不是日志的文件不碰
#[test]
fn old_logs_are_removed_but_recent_and_running_ones_stay() {
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now();
    let old = now - Duration::from_secs(31 * 24 * 3600);
    let recent = now - Duration::from_secs(29 * 24 * 3600);
    let files = [
        ("1.log", std::time::SystemTime::now()),
        ("2.log", recent),
        ("3.log", old),
        ("ready-default-main.log", old),
        ("clone-default-main.log", old),
    ];
    for (name, t) in files {
        let p = dir.path().join(name);
        std::fs::write(&p, "x\n").unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
    }
    std::fs::write(dir.path().join("10.pid"), "x").unwrap();
    // 1 号派发还在跑：它的日志再老也留着
    std::fs::File::options().write(true).open(dir.path().join("1.log")).unwrap().set_modified(old).unwrap();
    // 删掉 3.log 和 ready、clone 两个：1.log（还在跑）、2.log（30 天内）、10.pid 留着
    let removed = clean_logs(dir.path(), 30, &[dir.path().join("1.log")], now);
    assert_eq!(removed, 3);
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(left.contains(&"1.log".to_string()) && left.contains(&"2.log".to_string()), "{left:?}");
    assert!(!left.contains(&"3.log".to_string()) && !left.contains(&"clone-default-main.log".to_string()), "{left:?}");
    assert!(left.contains(&"10.pid".to_string()), "不是日志的文件不碰：{left:?}");
    // 天数改成 1：29 天前的也删
    let removed = clean_logs(dir.path(), 1, &[dir.path().join("1.log")], now);
    assert_eq!(removed, 1);
    assert!(dir.path().join("1.log").exists(), "还在跑的留着");
}

/// 正在跑的派发日志按实际路径保护：`board next` 之后目录名仍是第一张卡的 ID；
/// 对话日志 `chat-<ID>.log` 也要保护。即使超过 50 MB，清理也不删、不截断
#[test]
fn live_run_and_chat_logs_are_never_cleaned_or_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now();
    let old = now - Duration::from_secs(31 * 24 * 3600);
    let run_log = dir.path().join("42.log");
    let chat_log = dir.path().join("chat-7.log");
    for path in [&run_log, &chat_log] {
        let f = std::fs::File::create(path).unwrap();
        f.set_len(LOG_MAX + 1).unwrap();
        f.set_modified(old).unwrap();
    }
    let run = test_run(&run_log, None);
    assert_ne!(run.d.id, 42, "派发已经换到下一张，日志还叫第一张的 ID");
    let chat = test_chat_run(&chat_log);
    let keep = live_logs(std::slice::from_ref(&run), std::slice::from_ref(&chat));
    assert_eq!(clean_logs(dir.path(), 30, &keep, now), 0);
    assert_eq!(std::fs::metadata(&run_log).unwrap().len(), LOG_MAX + 1);
    assert_eq!(std::fs::metadata(&chat_log).unwrap().len(), LOG_MAX + 1);
    assert!(trim_logs(dir.path(), &keep, LOG_HEAD, LOG_TAIL, LOG_MAX).is_empty());
    assert_eq!(std::fs::metadata(&run_log).unwrap().len(), LOG_MAX + 1);
    assert_eq!(std::fs::metadata(&chat_log).unwrap().len(), LOG_MAX + 1);
}

/// 上传的行先转写、再脱敏；本机日志文件保留原文
#[test]
fn uploads_are_redacted_but_the_local_file_keeps_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("7.log");
    let token = "bd_0123456789abcdef0123456789abcdef";
    std::fs::write(&f, format!("BOARD_TOKEN={token}\nAuthorization: Bearer {token}\nworking\n")).unwrap();
    let (lines, _) = new_lines(&f, 0, false);
    let up = upload_lines_redacted(lines, None, &[token]);
    assert_eq!(up, vec!["BOARD_TOKEN=<已隐藏>", "Authorization: Bearer <已隐藏>", "working"]);
    let local = std::fs::read_to_string(&f).unwrap();
    assert!(local.contains(token), "本机日志保留原文：{local}");
}

/// 从服务端取到的供应商 key、MCP 密钥、Git 令牌也进脱敏列表；空的、重复的不重复加
#[test]
fn fetched_secrets_are_added_to_the_redaction_list() {
    let mut secrets = vec!["board-token".to_string()];
    add_secrets(&mut secrets, ["provider-key", "", "mcp-secret", "git-token", "provider-key"].map(str::to_string));
    assert_eq!(secrets, ["board-token", "provider-key", "mcp-secret", "git-token"]);
}

/// 派发和对话的临时文件里，供应商文件、dsh 补丁都有 key：无论正常结束还是启动失败都要删掉
#[test]
fn secret_files_are_removed_on_cleanup_and_start_failure() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("9.log");
    let run = test_run(&log, None);
    let provider = dir.path().join("9.provider.json");
    let patch = dir.path().join("9.dsh-patch.yml");
    let prompt = dir.path().join("9.prompt");
    for path in [&provider, &patch, &prompt] {
        std::fs::write(path, "secret").unwrap();
    }
    run.cleanup();
    assert!(!provider.exists() && !patch.exists() && !prompt.exists(), "派发收尾要删密钥文件");

    let chat = test_chat_run(&dir.path().join("7.log"));
    let provider = dir.path().join("-7.provider.json");
    let patch = dir.path().join("-7.dsh-patch.yml");
    for path in [&provider, &patch] {
        std::fs::write(path, "secret").unwrap();
    }
    chat.cleanup();
    assert!(!provider.exists() && !patch.exists(), "对话收尾要删密钥文件");

    let provider = dir.path().join("9.provider.json");
    let patch = dir.path().join("9.dsh-patch.yml");
    let role = RoleRun { model: String::new(), effort: String::new(), provider_file: Some(provider.clone()), dsh_patch: Some((patch.clone(), String::new())), dsh_key: None, subscription: false };
    {
        let _cleanup = SecretFiles::for_role(dir.path(), 9, Some(&role));
        std::fs::write(&provider, "secret").unwrap();
        std::fs::write(&patch, "secret").unwrap();
    }
    assert!(!provider.exists() && !patch.exists(), "启动中途失败也要删密钥文件");
}

/// #727：Claude Code 按角色用供应商、填了模型时，给这个进程带上自定义模型选项（网关只列自定义模型时才设得上）；
/// 没填模型、本机配置、官方订阅、别的 agent 都不带
#[test]
fn role_vars_add_claude_custom_model_only_for_provider_roles() {
    let role = |model: &str, provider: bool, subscription: bool| RoleRun {
        model: model.into(),
        effort: "high".into(),
        provider_file: provider.then(|| PathBuf::from("/tmp/1.provider.json")),
        dsh_patch: None,
        dsh_key: None,
        subscription,
    };
    let custom = vec![(CLAUDE_CUSTOM_MODEL_ENV.to_string(), "group/4.1f".to_string())];
    assert_eq!(role_vars("claude", Some(&role("group/4.1f", true, false))), custom);
    assert!(role_vars("claude", Some(&role("", true, false))).is_empty(), "没填模型：不切模型，也不带");
    assert!(role_vars("claude", Some(&role("group/4.1f", false, false))).is_empty(), "本机配置：不换供应商，不带");
    assert!(role_vars("claude", Some(&role("opus", false, true))).is_empty(), "官方订阅：不带");
    assert!(role_vars("claude", None).is_empty(), "不按角色派发：不带");
    assert!(role_vars("codex", Some(&role("group/4.1f", true, false))).is_empty());
    assert!(role_vars("dsh", Some(&role("group/4.1f", true, false))).is_empty());
    // WSL 里的 Claude Code 也要收到：经 WSLENV 传进去
    let out = launch_env(custom, "npx", true, Path::new("/tmp/npm-cache"));
    let wslenv = out.iter().find(|(k, _)| k == "WSLENV").map(|(_, v)| v.to_string_lossy().to_string()).unwrap_or_default();
    assert!(wslenv.contains(&format!("{CLAUDE_CUSTOM_MODEL_ENV}/u")), "{wslenv}");
}

/// 日志文件只有自己能读（0600）
#[cfg(unix)]
#[test]
fn log_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("logs").join("1.log");
    create_log(&f).unwrap();
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    // 已经存在的（以前建的 0644）也改成 0600
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
    restrict_log(&f);
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    // 截断之后还是 0600
    std::fs::write(&f, vec![b'x'; 200]).unwrap();
    truncate_log(&f, 10, 10, 100).unwrap().unwrap();
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
}

/// 卡片 #232：agent 在 `$SHELL -l -i -c` 里跑，CI 上没有终端，bash 往 stderr 打作业控制提示、退出时打 `logout`。
/// 这些不能进上报的 tail。真正的错误要留着
#[test]
fn tail_skips_login_shell_noise() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("1.log");
    std::fs::write(
        &f,
        "bash: cannot set terminal process group (-1): Inappropriate ioctl for device\nbash: no job control in this shell\nchecking\nmissing ok.txt\nlogout\n",
    )
    .unwrap();
    let tail = tail_lines(&f, 25, None);
    assert_eq!(tail, vec!["checking", "missing ok.txt"]);
    // 只有噪声时 tail 是空的：调用方按退出码写原因，不显示 logout
    std::fs::write(&f, "bash: no job control in this shell\nlogout\n").unwrap();
    let tail = tail_lines(&f, 25, None);
    assert!(tail.is_empty(), "{tail:?}");
}

/// 卡片 #232：Ubuntu 的 `/etc/bash.bashrc` 在登录 shell 里打 sudo 提示（本仓库 CI 的 ubuntu-latest 就有）。
/// 它和 bash 的作业控制提示一样不能进 tail、不能混进会话的「过程」
#[test]
fn tail_skips_the_sudo_hint_too() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("1.log");
    std::fs::write(
        &f,
        "bash: cannot set terminal process group (-1): Inappropriate ioctl for device\nbash: no job control in this shell\nTo run a command as administrator (user \"root\"), use \"sudo <command>\".\nSee \"man sudo_root\" for details.\n\nchecking\nmissing ok.txt\n",
    )
    .unwrap();
    assert_eq!(tail_lines(&f, 25, None), vec!["checking", "missing ok.txt"]);
    // 会话的「原始输出」里也不出现（空行本来就不算内容）
    let up: Vec<String> = upload_lines(new_lines(&f, 0, true).0, None).into_iter().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(up, vec!["checking", "missing ok.txt"]);
}

#[test]
fn reads_only_whole_lines() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.log");
    std::fs::write(&f, "one\ntwo\nthr").unwrap();
    let (lines, at) = new_lines(&f, 0, false);
    assert_eq!(lines, vec!["one", "two"]);
    assert_eq!(at, 8);
    let (lines, at2) = new_lines(&f, at, true);
    assert_eq!(lines, vec!["thr"]);
    assert_eq!(at2, 11);
    assert_eq!(tail_lines(&f, 2, None), vec!["two", "thr"]);
}

/// 卡片 #211、#184：日志里是 board acp 写的 ACP 帧。上传和异常退出评论里的「日志最后 20 行」是转写后的一行一行；
/// 本机日志文件里还是原样的帧
#[test]
fn acp_logs_are_rendered_for_upload() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("7.log");
    let ev = |v: serde_json::Value| v.to_string() + "\n";
    let upd = |u: serde_json::Value| ev(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s", "update": u } }));
    let mut raw = String::from("[claude-code:unrecognized_model] {\"model\":\"x\"}\n");
    raw += &ev(json!({ "jsonrpc": "2.0", "id": 0, "result": { "protocolVersion": 1, "agentInfo": { "name": "claude-agent-acp", "title": "Claude Agent", "version": "0.107.3" } } }));
    raw += &upd(json!({ "sessionUpdate": "available_commands_update", "availableCommands": [] }));
    raw += &upd(json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "kind": "execute", "title": "git status", "rawInput": { "command": "git status" } }));
    // 工具结果整段在一行里（这里 1 MB）：转写后只剩一行
    let big = "x".repeat(1024 * 1024);
    raw += &upd(json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed", "rawOutput": { "output": format!("On branch main\n{big}") } }));
    std::fs::write(&f, &raw).unwrap();
    let (lines, at) = new_lines(&f, 0, false);
    assert_eq!(at, raw.len() as u64, "一行很长也整行读到");
    let up = upload_lines(lines, Some(Stream::Acp));
    assert_eq!(
        up,
        vec![
            "[claude-code:unrecognized_model] {\"model\":\"x\"}",
            "● 开始 · Claude Agent 0.107.3",
            "▸ shell  git status",
            "  ↳ On branch main  （共 2 行）",
        ]
    );
    assert_eq!(tail_lines(&f, 2, Some(Stream::Acp)), vec!["▸ shell  git status", "  ↳ On branch main  （共 2 行）"]);
    // 不转写的 agent 原样上传，最多最后 500 行
    let many: Vec<String> = (0..800).map(|i| format!("line {i}")).collect();
    let up = upload_lines(many, None);
    assert_eq!((up.len(), up[0].as_str()), (LOG_KEEP, "line 300"));
}

#[test]
fn builtin_order_and_overrides() {
    // 内置顺序就是默认顺序：dsh 第一
    assert_eq!(BUILTIN.iter().map(|b| b.id).collect::<Vec<_>>(), vec!["dsh", "claude", "codex"]);
    for b in &BUILTIN {
        check_agent_id(b.id).unwrap();
        assert!(!b.win.contains(&"direct"), "内置模板在 Windows 上都经过 shell 启动");
    }
    // dsh 用自带的 acp profile，带禁用 goal 的 profile 补丁；其他 agent 不带
    let dsh = BUILTIN.iter().find(|b| b.id == "dsh").unwrap();
    assert_eq!(dsh.package, "@deepseek-ai/dsh", "装带 dsh 可执行文件的 CLI 包，不是没有 bin 的 headless 运行库");
    assert!(dsh.also.contains(&"~/.local/bin/dsh"), "装进用户目录后即使 PATH 还没刷新也找得到");
    assert_eq!(dsh.acp, &["{bin}", "--profile", "acp", "--patch", "{patch}"]);
    assert!(DSH_PATCH.contains("goal-round-driver"), "补丁要关掉 goal-round-driver");
    assert!(DSH_PATCH.contains("tool-goal"), "补丁要关掉 goal 工具");
    assert!(BUILTIN.iter().filter(|b| b.id != "dsh").all(|b| !b.acp.iter().any(|a| a.contains("{patch}"))));
    // 派发名字的第二段（模型名）不再取内置模板的默认值：上报 agent 配置文件里现在的模型（见 providersync::current）
    // 内置 agent 全部换成找不到的命令：本机真装了 dsh、codex 也不去执行它们的 --version
    // （本机执行 codex 会被 macOS 拦下、反复弹「包含恶意软件」的提示）
    let nope = |id: &str| AgentConf { id: id.into(), name: None, model: None, cmd: vec![format!("/nope/{id}")] };
    let conf = vec![
        nope("dsh"),
        nope("claude"),
        nope("codex"),
        AgentConf { id: "mycli".into(), name: Some("My CLI".into()), model: Some("gpt-5".into()), cmd: vec!["/nope/mycli".into(), "--acp".into()] },
    ];
    let specs = agent_specs(&conf, &[Env::direct()]).unwrap();
    assert_eq!(specs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["dsh", "claude", "codex", "mycli"]);
    let claude = specs.iter().find(|s| s.id == "claude").unwrap();
    assert_eq!(claude.name, "Claude Code");
    assert_eq!(claude.model, "", "配置文件没写 model：空（读 agent 自己的配置文件）");
    assert!(claude.default_check().is_none(), "换了命令就不再看内置的备用位置");
    assert_eq!(specs[3].name, "My CLI");
    assert_eq!(specs[3].model, "gpt-5");
    assert_eq!(specs[3].checks.len(), 1, "只有直接启动");
    assert!(!specs[3].misses_skill(&Env::direct(), &[]), "配置文件里的 agent 不检查 skill");
    let r = specs[3].report(None);
    assert_eq!((r.found, r.envs.len(), r.env, r.no_skill, r.model.as_str()), (false, 1, None, None, "gpt-5"));
    // 读得到 agent 配置文件里的模型、强度时报它们；读不到时用配置文件里的 model
    let now = ("cn:deepseek-v4.1".to_string(), "high".to_string());
    let r = specs[3].report(Some(&now));
    assert_eq!((r.model.as_str(), r.effort.as_str()), ("cn:deepseek-v4.1", "high"));
    assert!(r.run_mode.is_none(), "派发时能选什么不再上报");
    let conf = vec![AgentConf { id: "dsh".into(), name: None, model: Some("deepseek-reasoner".into()), cmd: vec!["/nope/dsh".into()] }];
    let specs = agent_specs(&conf, &[Env::direct()]).unwrap();
    assert_eq!(specs[0].model, "deepseek-reasoner");
}

fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// ACP 入口：`{bin}` 换成找到的 agent 命令、`{patch}` 换成补丁路径；适配器拿到找到的 agent 路径
#[test]
fn acp_entry_fills_bin_and_patch() {
    let specs = agent_specs(&[], &[]).unwrap();
    let get = |id: &str| specs.iter().find(|s| s.id == id).unwrap();
    let check = |prog: &str, path: &str| Check { env: "login".into(), found: true, version: "0.2.2-rc.3".into(), prog: Some(prog.into()), path: Some(path.into()), missing: None };
    let (p, a) = get("dsh").entry(&check("/Applications/x/dsh", "/Applications/x/dsh"), "/tmp/patch.yml").unwrap();
    assert_eq!((p.as_str(), a), ("/Applications/x/dsh", sv(&["--profile", "acp", "--patch", "/tmp/patch.yml"])));
    let vars = get("dsh").acp_vars(&check("dsh", "/usr/local/bin/dsh"));
    assert_eq!(vars, vec![("DSH_PERMISSION_MODE".to_string(), "danger-full-access".to_string())], "入口就是 dsh 自己：不传路径，只关沙箱和审批");
    let claude = get("claude");
    let (p, a) = claude.entry(&check("claude", "/Users/me/.local/bin/claude"), "").unwrap();
    assert_eq!((p.as_str(), a.len()), ("npx", 2));
    // 后台任务不关（#461：v2 下 board acp 等后台跑完），只告诉适配器 claude 在哪。
    // Windows 上这两个是别的系统的路径（不是 .exe，父目录也不在）：不交给适配器，它用自带的那份（#491）；
    // Windows 上真的找到 npm 包里的程序时照样传，见 acp_vars_finds_the_npm_exe_on_windows
    let claude_vars = claude.acp_vars(&check("claude", "/Users/me/.local/bin/claude"));
    if cfg!(windows) {
        assert!(claude_vars.is_empty(), "{claude_vars:?}");
    } else {
        assert_eq!(claude_vars, vec![("CLAUDE_CODE_EXECUTABLE".to_string(), "/Users/me/.local/bin/claude".to_string())]);
    }
    let codex = get("codex");
    let vars = codex.acp_vars(&check("codex", "/opt/homebrew/bin/codex"));
    if cfg!(windows) {
        assert_eq!(vars, vec![("INITIAL_AGENT_MODE".to_string(), "agent-full-access".to_string())], "mac 路径在 Windows 上不交给适配器");
    } else {
        assert!(vars.contains(&("CODEX_PATH".to_string(), "/opt/homebrew/bin/codex".to_string())) && vars.iter().any(|(k, v)| k == "INITIAL_AGENT_MODE" && v == "agent-full-access"), "{vars:?}");
    }
    assert_eq!((claude.acp_mode.as_str(), codex.acp_mode.as_str()), ("bypassPermissions", "agent-full-access"));
    // 配置文件覆盖：npx 换版本时照旧找 claude；换成自己的入口时找它自己；以前的一次性写法不用
    let conf = |cmd: &[&str]| vec![AgentConf { id: "claude".into(), cmd: sv(cmd), ..Default::default() }];
    let specs = agent_specs(&conf(&["npx", "-y", "@agentclientprotocol/claude-agent-acp@9.9.9"]), &[]).unwrap();
    let c = specs.iter().find(|s| s.id == "claude").unwrap();
    assert_eq!((c.bin.as_str(), c.acp_bin_env.as_str(), c.managed, c.acp[2].as_str()), ("claude", "CLAUDE_CODE_EXECUTABLE", true, "@agentclientprotocol/claude-agent-acp@9.9.9"));
    let specs = agent_specs(&conf(&["/opt/my-acp", "--stdio"]), &[]).unwrap();
    let c = specs.iter().find(|s| s.id == "claude").unwrap();
    assert_eq!((c.bin.as_str(), c.acp_bin_env.as_str(), c.managed), ("/opt/my-acp", "", false));
    let specs = agent_specs(&conf(&["claude", "-p", "{prompt}"]), &[]).unwrap();
    let c = specs.iter().find(|s| s.id == "claude").unwrap();
    assert_eq!((c.acp[0].as_str(), c.legacy), ("npx", false), "以前的写法覆盖内置模板时不用，照内置的跑");
    // 配置文件里新加的 agent 是以前的写法：派发时报错说明
    let specs = agent_specs(&[AgentConf { id: "mine".into(), cmd: sv(&["mine", "{prompt}"]), ..Default::default() }], &[]).unwrap();
    let err = specs.iter().find(|s| s.id == "mine").unwrap().entry(&check("mine", "mine"), "").unwrap_err();
    assert!(err.contains("旧写法") && err.contains("ACP"), "{err}");
}

/// 找到了 claude、启动环境里没有 npx：这个环境记成不可用、写明缺 Node.js，版本照报，不当成「没安装」（#458）
#[test]
fn agent_without_npx_is_reported_as_missing_node() {
    let claude = BUILTIN.iter().find(|b| b.id == "claude").unwrap();
    let needs: Vec<(String, String)> = claude.needs.iter().map(|(c, w)| (c.to_string(), w.to_string())).collect();
    let found = || Check {
        env: "login".into(),
        found: true,
        version: "2.1.326 (Claude Code)".into(),
        prog: Some("claude".into()),
        path: Some("/Users/me/.local/bin/claude".into()),
        missing: None,
    };
    let c = found().needs(&needs, |_| false);
    assert_eq!((c.found, c.missing.as_deref()), (false, Some("Node.js（npx）")));
    assert!(found().needs(&needs, |cmd| cmd == "npx").found, "有 npx 时不变");
    let gone = Check { found: false, ..found() }.needs(&needs, |_| false);
    assert_eq!((gone.found, gone.missing), (false, None), "agent 本身没找到：照旧是没找到，不说缺 Node.js");
    assert!(found().needs(&[], |_| false).found, "不要别的命令的 agent（dsh）不受影响");
    // 上报：版本照写、写明缺什么；管理方式不是「没安装」，没有安装按钮
    let mut spec = managed_spec("claude", claude);
    spec.checks = vec![c];
    let r = spec.report(None);
    assert_eq!((r.found, r.missing.as_deref(), r.version.as_str()), (false, Some("Node.js（npx）"), "2.1.326 (Claude Code)"));
    assert_eq!((r.envs[0].found, r.envs[0].missing.as_deref()), (false, Some("Node.js（npx）")));
    let m = r.manage.unwrap();
    assert_eq!((m.method.as_str(), m.installable, m.install_command.as_str()), ("needs", false, ""));
    assert!(m.method_label.contains("Node.js") && m.reason.contains("重新检查"), "{m:?}");
}

/// Windows 上 npm 的垫片（无扩展名的 sh 脚本、.cmd、.ps1）不直接传给适配器：换成 npm 包里真正的 .exe，找不到就不传（#456）
#[test]
fn adapter_gets_the_real_exe_behind_npm_shims() {
    let d = tempfile::tempdir().unwrap();
    let npm = d.path().join("npm");
    let def = |id: &str| BUILTIN.iter().find(|b| b.id == id).unwrap().npm_exe;
    let shim = |name: &str| npm.join(name).display().to_string();
    let put = |rel: &str, body: &[u8]| {
        let p = npm.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    };
    let pe = b"MZ\x90\x00\x03";
    // Claude Code：postinstall 没跑时 bin/claude.exe 还是占位的 sh 脚本，不算
    let exe = put("node_modules/@anthropic-ai/claude-code/bin/claude.exe", b"#!/bin/sh\necho not installed >&2\n");
    assert_eq!(adapter_bin(&shim("claude"), true, def("claude")), None);
    std::fs::write(&exe, pe).unwrap();
    for s in ["claude", "claude.cmd", "claude.ps1", "CLAUDE.CMD"] {
        assert_eq!(adapter_bin(&shim(s), true, def("claude")).map(PathBuf::from), Some(exe.clone()), "{s}");
    }
    // 原生安装的 .exe、不在 Windows 上（含 WSL 里）的路径原样传
    assert_eq!(adapter_bin(r"C:\Users\me\.local\bin\claude.exe", true, def("claude")).as_deref(), Some(r"C:\Users\me\.local\bin\claude.exe"));
    assert_eq!(adapter_bin("/home/me/.npm-global/bin/claude", false, def("claude")).as_deref(), Some("/home/me/.npm-global/bin/claude"));
    // 找不到 npm 包（别处的垫片、没写位置的 agent）：不传，适配器用它自带的
    assert_eq!(adapter_bin(&d.path().join("other").join("claude.cmd").display().to_string(), true, def("claude")), None);
    assert_eq!(adapter_bin(&shim("claude"), true, &[]), None);
    assert_eq!(adapter_bin("", false, def("claude")), None);
    // Codex：原生程序在平台包里，平台包在 @openai/codex 自己的 node_modules 里，或者被提到上一层
    assert_eq!(adapter_bin(&shim("codex"), true, def("codex")), None);
    let hoisted = put("node_modules/@openai/codex-win32-arm64/vendor/aarch64-pc-windows-msvc/bin/codex.exe", pe);
    assert_eq!(adapter_bin(&shim("codex"), true, def("codex")).map(PathBuf::from), Some(hoisted));
    let nested = put("node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe", pe);
    assert_eq!(adapter_bin(&shim("codex.cmd"), true, def("codex")).map(PathBuf::from), Some(nested), "两处都有时先用 @openai/codex 自己带的");
}

/// Windows 上 `acp_vars` 走的是「找 npm 垫片背后的 .exe」这条（#491）：找到就传真实程序，找不到就不传
#[cfg(windows)]
#[test]
fn acp_vars_finds_the_npm_exe_on_windows() {
    let d = tempfile::tempdir().unwrap();
    let npm = d.path().join("npm");
    std::fs::create_dir_all(&npm).unwrap();
    let shim = npm.join("claude.cmd");
    std::fs::write(&shim, "@echo off\r\n").unwrap();
    let exe = npm.join("node_modules/@anthropic-ai/claude-code/bin/claude.exe");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"MZ\x90\x00\x03").unwrap();
    let specs = agent_specs(&[], &[]).unwrap();
    let claude = specs.iter().find(|s| s.id == "claude").unwrap();
    let check = Check {
        env: "login".into(),
        found: true,
        version: "2.1.326".into(),
        prog: Some("claude".into()),
        path: Some(shim.display().to_string()),
        missing: None,
    };
    let npm_exe = BUILTIN.iter().find(|b| b.id == "claude").unwrap().npm_exe;
    let found = adapter_bin(&check.path.clone().unwrap(), true, npm_exe).expect("找到包里的真实程序");
    assert_eq!(PathBuf::from(&found), exe.clone());
    assert_eq!(claude.acp_vars(&check), vec![("CLAUDE_CODE_EXECUTABLE".to_string(), found)]);
    // 垫片在、但包里还没有真正的程序（postinstall 没跑）：不传，适配器用它自带的
    std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
    assert!(claude.acp_vars(&check).is_empty(), "{:?}", claude.acp_vars(&check));
}

/// npx 跑适配器：npm 不打更新、fund、audit 的提示（#459），缓存放在 board host 的目录里；WSL 里经 WSLENV 传进去
#[test]
fn npx_entry_keeps_npm_quiet() {
    let cache = Path::new("/h/npm-cache");
    let get = |v: &[(String, std::ffi::OsString)], k: &str| v.iter().find(|(x, _)| x == k).map(|(_, v)| v.to_string_lossy().into_owned());
    let path = || vec![("CODEX_PATH".to_string(), "/bin/codex".to_string())];
    let out = launch_env(path(), "npx", false, cache);
    for k in ["npm_config_update_notifier", "npm_config_fund", "npm_config_audit"] {
        assert_eq!(get(&out, k).as_deref(), Some("false"), "{k}");
    }
    assert_eq!((get(&out, "npm_config_cache").as_deref(), get(&out, "CODEX_PATH").as_deref()), (Some("/h/npm-cache"), Some("/bin/codex")));
    assert!(get(&out, "WSLENV").is_none());
    let out = launch_env(path(), "npx", true, cache);
    let w = get(&out, "WSLENV").unwrap();
    assert!(w.contains("npm_config_update_notifier/u") && w.contains("npm_config_fund/u") && w.contains("CODEX_PATH/u"), "{w}");
    assert_eq!((get(&out, "npm_config_update_notifier").as_deref(), get(&out, "npm_config_cache")), (Some("false"), None), "WSL 里的 npm 用它自己的缓存");
    // 入口不是 npx（dsh 自己讲 ACP）：只带适配器要的
    let out = launch_env(vec![("DSH_PERMISSION_MODE".into(), "danger-full-access".into())], "dsh", false, cache);
    assert_eq!(out.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["DSH_PERMISSION_MODE"]);
}

#[test]
fn expands_home() {
    let home = expand("~");
    assert!(home.is_absolute() || home == PathBuf::from("~"));
    assert_eq!(expand("/abs/x"), PathBuf::from("/abs/x"));
}

/// 目录状态缓存：刚看过的沿用，隔 DIR_CHECK_EVERY 才重新跑 git 子进程（心跳 10 秒一次，不能每次都跑）；
/// 目录变了（或克隆完）就丢掉旧结果。上报的 checkedAt 是真正跑 git 的那一次的时间，两次重新看之间一直是同一个数
/// （以前每次心跳现算，会差 1 毫秒，服务端当成机器有变化、给所有界面推送，default/main#469）
#[test]
fn dir_checks_are_cached_between_beats() {
    let t = tempfile::tempdir().unwrap();
    let repo = plain(&t.path().canonicalize().unwrap());
    git(&repo, &["init", "-q", "-b", "main"]);
    let taken = |checks: &[DirCheck]| checks.iter().find(|c| c.path == repo).map(|c| (c.status, c.at, c.checked_at));
    // 还没有结果：当成不存在（不会跑 git 以外的东西，也不会让上报出错）
    assert!(Host::dir_check_due(&[], &repo));
    let mut checks = vec![];
    let (status, origin, branch) = check_git_dir(&repo);
    assert_eq!(status, "ok");
    let before = now_ms();
    checks.push(DirCheck::new(repo.clone(), status, origin, branch));
    let first = taken(&checks).unwrap();
    assert!(first.2 >= before && first.2 <= now_ms());
    // 刚看过：不到时间，不再跑 git；结果和看的时间都不变
    std::thread::sleep(Duration::from_millis(3));
    assert!(!Host::dir_check_due(&checks, &repo));
    assert_eq!(taken(&checks).unwrap().1, first.1);
    assert_eq!(taken(&checks).unwrap().2, first.2);
    // 到时间了：重新看，看的时间换新的
    checks[0].at = Instant::now() - DIR_CHECK_EVERY;
    assert!(Host::dir_check_due(&checks, &repo));
    let (status, origin, branch) = check_git_dir(&repo);
    checks[0] = DirCheck::new(repo.clone(), status, origin, branch);
    assert_eq!(taken(&checks).unwrap().0, "ok");
    assert!(taken(&checks).unwrap().1 > first.1);
    assert!(taken(&checks).unwrap().2 > first.2);
    // 目录没了：到时间之后看到「不存在」
    std::fs::remove_dir_all(&repo).unwrap();
    checks[0].at = Instant::now() - DIR_CHECK_EVERY;
    let (status, _, _) = check_git_dir(&repo);
    assert_eq!(status, "missing");
}

#[test]
fn missing_dirs_are_reported_instead_of_failing_startup() {
    let missing = "~/board-test-no-such-dir";
    let ws = to_workspace(Some(missing), &[("default/main".into(), missing.into())]).unwrap();
    assert_eq!(ws.overrides[0].path, missing, "配置里的 ~ 原样保留");
    assert_eq!(ws.overrides[0].board, "default/main");
    assert!(!workspace_path(&ws.overrides[0].path).unwrap().is_dir(), "目录可以暂时不存在，交给界面显示并克隆");
}

#[test]
fn skill_check_sees_project_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".agents").join("skills").join(SKILL)).unwrap();
    std::fs::write(repo.join(".agents").join("skills").join(SKILL).join("SKILL.md"), "x").unwrap();
    let spec = Spec {
        id: "codex".into(),
        name: "Codex".into(),
        model: String::new(),
        bin: "codex".into(),
        acp: vec!["codex-acp".into()],
        acp_bin_env: String::new(),
        acp_env: vec![],
        acp_mode: String::new(),
        legacy: false,
        skills: vec!["/board-test-no-such-skills".into()],
        project: vec![".agents/skills".into()],
        checks: vec![Check { env: "direct".into(), found: true, version: String::new(), prog: Some("codex".into()), path: Some("codex".into()), missing: None }],
        no_skill: BTreeMap::new(),
        builtin: None,
        managed: false,
        latest: String::new(),
        checked_at: 0,
        manage_state: String::new(),
        manage_error: String::new(),
    };
    let direct = Env::direct();
    let dirs = [("default/main".into(), repo.clone())];
    assert!(!spec.misses_skill(&direct, &dirs), "项目里的 .agents/skills 算装好了");
    assert!(spec.misses_skill(&direct, &[]), "没有项目目录时仍然提醒");

    // 上报给服务端：看不到的标出该装到哪里，看得到的不标
    let mut spec = spec;
    spec.check_skills(std::slice::from_ref(&direct), &[]);
    let r = spec.report(None);
    let to = r.no_skill.clone().expect("看不到 board-skills 要上报");
    assert!(to.contains("/board-test-no-such-skills/board-skills") && to.contains(".agents/skills/board-skills"), "{to}");
    assert_eq!(r.envs[0].no_skill.as_deref(), Some(to.as_str()), "每个启动环境各自上报");
    spec.check_skills(std::slice::from_ref(&direct), &dirs);
    let r = spec.report(None);
    assert_eq!((r.no_skill, r.envs[0].no_skill.clone()), (None, None), "装了的不标");

    // Claude Code 不读 .agents/skills：仓库里只有 .agents/skills 时仍然提醒，提醒里写 .claude/skills
    let mut claude = Spec { id: "claude".into(), project: vec![".claude/skills".into()], ..spec };
    assert!(claude.misses_skill(&direct, &dirs), "Claude Code 看不到仓库的 .agents/skills");
    claude.check_skills(std::slice::from_ref(&direct), &dirs);
    let to = claude.report(None).no_skill.unwrap();
    assert!(to.contains("仓库的 .claude/skills/board-skills") && !to.contains(".agents/skills"), "{to}");
}

/// dsh 的 acp profile 从 0.2.2 起才有：本机更旧时不启动，提示升级
#[test]
fn old_dsh_has_no_acp() {
    assert!(older_than("0.2.1", "0.2.2") && older_than("dsh 0.1.9", "0.2.2") && older_than("0.2", "0.2.2"));
    assert!(!older_than("0.2.2-rc.1", "0.2.2") && !older_than("0.10.0", "0.2.2") && !older_than("1.0", "0.2.2"));
    assert!(!older_than("", "0.2.2") && !older_than("unknown", "0.2.2"), "看不出版本时当作够新");
    let dsh = BUILTIN.iter().find(|b| b.id == "dsh").unwrap();
    assert_eq!(dsh.acp_since, "0.2.2");
    assert!(BUILTIN.iter().filter(|b| b.id != "dsh").all(|b| b.acp_since.is_empty()));
}

/// agent 的版本比较要认预发布：0.2.2-rc.1 < 0.2.2-rc.3 < 0.2.2
#[test]
fn agent_versions_compare_prereleases() {
    assert!(manage_older("0.2.2-rc.1", "0.2.2-rc.3"));
    assert!(manage_older("0.2.2-rc.3", "0.2.2"));
    assert!(manage_older("codex-cli 0.168.0", "0.169.0"));
    assert!(!manage_older("0.2.2", "0.2.2-rc.3"));
    assert!(!manage_older("0.169.0", "0.169.0+build.2"));
}

/// npm registry 的返回只取 version，缺字段、坏 JSON 都报原因
#[test]
fn npm_latest_reads_version() {
    assert_eq!(npm_latest(r#"{"version":"1.2.3"}"#).unwrap(), "1.2.3");
    assert!(npm_latest(r#"{"name":"x"}"#).unwrap_err().contains("version"));
    assert!(npm_latest("not json").unwrap_err().contains("JSON"));
}

/// npm 全局安装的 prefix 要从实际路径反推；更新必须沿这个目录，不能改用 npm 现在的全局配置
#[test]
fn npm_global_prefix_and_update_args_follow_the_installed_copy() {
    assert_eq!(npm_global_prefix("/usr/local/node-v22/lib/node_modules/@openai/codex/bin/codex.js").as_deref(), Some("/usr/local/node-v22"));
    assert_eq!(npm_global_prefix("/Users/me/.local/lib/node_modules/@deepseek-ai/dsh/lib/bin.js").as_deref(), Some("/Users/me/.local"));
    assert_eq!(
        npm_global_prefix(r"C:\Users\me\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.js").as_deref(),
        Some("C:/Users/me/AppData/Roaming/npm")
    );
    assert_eq!(
        npm_update_args(Some("/Users/me/.local"), "@openai/codex", "0.169.0"),
        ["install", "-g", "--prefix", "/Users/me/.local", "@openai/codex@0.169.0"]
    );
    assert_eq!(npm_update_args(None, "@openai/codex", ""), ["install", "-g", "@openai/codex@latest"]);
}

/// Unix 上缺少的 agent 装进用户目录，不需要管理员权限；界面上的复制命令和实际执行一致
#[cfg(unix)]
#[test]
fn agent_install_uses_the_user_npm_prefix_on_unix() {
    let b = BUILTIN.iter().find(|b| b.id == "dsh").unwrap();
    let (program, args) = install_plan(b).unwrap();
    assert_eq!(program, "npm");
    let prefix = expand("~/.local").display().to_string();
    assert!(args.windows(2).any(|w| w == ["--prefix", prefix.as_str()]), "{args:?}");
    assert!(install_command(b).contains("npm install -g --prefix ~/.local @deepseek-ai/dsh@next"));

    // npm 在 Unix 上要求 prefix/lib 先存在：第一次装进 ~/.local 时由 board host 建好
    let dir = tempfile::tempdir().unwrap();
    let args = vec!["install".into(), "-g".into(), "--prefix".into(), dir.path().display().to_string()];
    prepare_npm_install("npm", &args).unwrap();
    assert!(dir.path().join("lib").is_dir());
}

/// 装进用户目录后把它的 bin 目录写进 shell 配置，新开终端也能直接敲命令；重复安装不重复追加
#[test]
fn user_prefix_adds_the_bin_dir_to_shell_path_once() {
    let d = tempfile::tempdir().unwrap();
    let rc = d.path().join(".zshrc");
    std::fs::write(&rc, "export FOO=1").unwrap();
    let dir = Path::new("/Users/me/.local/bin");
    assert!(append_path_line(&rc, dir, false, false).unwrap());
    let text = std::fs::read_to_string(&rc).unwrap();
    assert_eq!(text, "export FOO=1\nexport PATH=\"/Users/me/.local/bin:$PATH\"\n");
    assert!(!append_path_line(&rc, dir, false, false).unwrap(), "同一行只追加一次");
    assert_eq!(std::fs::read_to_string(&rc).unwrap(), text);
    assert!(append_path_line(&rc, dir, true, false).unwrap(), "fish 的写法不同，算另一行");
    assert!(std::fs::read_to_string(&rc).unwrap().contains("fish_add_path \"/Users/me/.local/bin\""));

    // 目录已经在 PATH 里但被前面的系统目录盖住：同一行还要再放到文件末尾一次，之后才会排在前面
    assert!(append_path_line(&rc, dir, false, true).unwrap());
    assert_eq!(std::fs::read_to_string(&rc).unwrap().matches("export PATH=\"/Users/me/.local/bin:$PATH\"").count(), 2);

    // PATH 的冒号分隔是 Unix 语义；Windows 上 split_paths 用分号，不能拿这串 Unix 路径做断言
    #[cfg(unix)]
    {
        let path = "/usr/local/node-v22/bin:/Users/me/.local/bin:/usr/bin";
        assert!(!path_has_dir_before(path, dir, Some(Path::new("/usr/local/node-v22/bin"))));
        assert!(path_has_dir_before("/Users/me/.local/bin:/usr/local/node-v22/bin:/usr/bin", dir, Some(Path::new("/usr/local/node-v22/bin"))));
        assert!(path_has_dir_before("/usr/bin:/Users/me/.local/bin", dir, None));
    }
}

/// 加进 PATH 的是 npm 的 --prefix 下面的 bin：装完直接从那里找命令
#[test]
fn npm_bin_dir_follows_the_install_prefix() {
    let args = ["install", "-g", "--prefix", "/Users/me/.local", "@deepseek-ai/dsh@next"].map(str::to_string);
    assert_eq!(npm_bin_dir("npm", &args), Some(Path::new("/Users/me/.local").join("bin")));
    assert_eq!(npm_bin_dir("brew", &args), None);
    let update = ["install", "-g", "@openai/codex@latest"].map(str::to_string);
    assert_eq!(npm_bin_dir("npm", &update), None, "沿已有 prefix 的更新不用再改 PATH");
}

/// 写了 --prefix 的 npm 安装/更新先装进 prefix 里的临时目录；没写 prefix、不是 npm 的照原样（#536）
#[test]
fn npm_jobs_install_into_a_stage_first() {
    let args = npm_update_args(Some("/Users/me/.local"), "@openai/codex", "0.171.0");
    let (staged, stage) = stage_npm("npm", args, "@openai/codex", "codex");
    let stage = stage.expect("写了 --prefix");
    assert_eq!(stage.prefix, Path::new("/Users/me/.local"));
    assert_eq!(stage.dir, Path::new("/Users/me/.local").join(".board-stage-codex"));
    assert_eq!(staged, ["install", "-g", "--prefix", &stage.dir.display().to_string(), "@openai/codex@0.171.0"]);
    let plain = npm_update_args(None, "@openai/codex", "");
    assert_eq!(stage_npm("npm", plain.clone(), "@openai/codex", "codex"), (plain.clone(), None));
    assert_eq!(stage_npm("brew", plain.clone(), "@openai/codex", "codex").1, None);
    assert_eq!(npm_pkg_dir(Path::new("/p"), "@openai/codex", false), Path::new("/p/lib/node_modules/@openai/codex"));
    assert_eq!(npm_pkg_dir(Path::new("/p"), "@openai/codex", true), Path::new("/p/node_modules/@openai/codex"));
}

/// 在 prefix 里摆一个 npm 全局包：包目录（带 package.json 和一个文件）+ bin 里的相对软链
#[cfg(unix)]
fn fake_npm_pkg(prefix: &Path, version: &str) {
    let pkg = npm_pkg_dir(prefix, "@openai/codex", false);
    std::fs::create_dir_all(pkg.join("bin")).unwrap();
    std::fs::write(pkg.join("package.json"), format!("{{\"version\":\"{version}\"}}")).unwrap();
    std::fs::write(pkg.join("bin/codex.js"), version).unwrap();
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::os::unix::fs::symlink("../lib/node_modules/@openai/codex/bin/codex.js", prefix.join("bin/codex")).unwrap();
}

/// 装好了：新包换进真正的 prefix，命令照样能用、指向新版本；备份和临时目录都删掉
#[cfg(unix)]
#[test]
fn stage_commit_swaps_in_the_new_version() {
    let d = tempfile::tempdir().unwrap();
    let prefix = d.path().join(".local");
    fake_npm_pkg(&prefix, "0.170.0");
    let (_, stage) = stage_npm("npm", npm_update_args(Some(&prefix.display().to_string()), "@openai/codex", ""), "@openai/codex", "codex");
    let stage = stage.unwrap();
    stage.reset(false).unwrap();
    assert!(stage.dir.join("lib").is_dir(), "npm 要求 prefix 下有 lib");
    fake_npm_pkg(&stage.dir, "0.171.0");
    // 再放一个指到临时目录的绝对路径软链：挪过去改指真正的 prefix
    std::os::unix::fs::symlink(npm_pkg_dir(&stage.dir, "@openai/codex", false).join("bin/codex.js"), stage.dir.join("bin/codex-abs")).unwrap();
    stage.commit(false).unwrap();
    assert_eq!(std::fs::read_to_string(prefix.join("bin/codex")).unwrap(), "0.171.0");
    assert_eq!(std::fs::read_to_string(prefix.join("bin/codex-abs")).unwrap(), "0.171.0");
    assert!(std::fs::read_link(prefix.join("bin/codex-abs")).unwrap().starts_with(&prefix));
    assert!(!stage.dir.exists(), "临时目录删掉");
    let scope = prefix.join("lib/node_modules/@openai");
    let left: Vec<_> = std::fs::read_dir(&scope).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, ["codex"], "旧版本的备份删掉");
    // 第一次装（prefix 里还没有这个包）也一样
    let fresh = d.path().join("fresh");
    let (_, stage) = stage_npm("npm", npm_update_args(Some(&fresh.display().to_string()), "@openai/codex", ""), "@openai/codex", "codex");
    let stage = stage.unwrap();
    stage.reset(false).unwrap();
    fake_npm_pkg(&stage.dir, "0.171.0");
    stage.commit(false).unwrap();
    assert_eq!(std::fs::read_to_string(fresh.join("bin/codex")).unwrap(), "0.171.0");
}

/// 超时、失败、装了一半（npm 报成功但包不完整）：只删临时目录，旧命令原样能用
#[cfg(unix)]
#[test]
fn stage_failure_keeps_the_old_version() {
    let d = tempfile::tempdir().unwrap();
    let prefix = d.path().join(".local");
    fake_npm_pkg(&prefix, "0.170.0");
    let (_, stage) = stage_npm("npm", npm_update_args(Some(&prefix.display().to_string()), "@openai/codex", ""), "@openai/codex", "codex");
    let stage = stage.unwrap();
    // 上次 host 被结束留下的临时目录：开始前清掉
    std::fs::create_dir_all(stage.dir.join("junk")).unwrap();
    stage.reset(false).unwrap();
    assert!(!stage.dir.join("junk").exists());
    // npm 装了一半就被结束：包目录在，package.json 没有
    let half = npm_pkg_dir(&stage.dir, "@openai/codex", false);
    std::fs::create_dir_all(&half).unwrap();
    let err = stage.commit(false).unwrap_err();
    assert!(err.contains("旧版本没动"), "{err}");
    assert!(!stage.dir.exists());
    assert_eq!(std::fs::read_to_string(prefix.join("bin/codex")).unwrap(), "0.170.0");
    // 超时、退出码不是 0：discard 只删临时目录
    stage.reset(false).unwrap();
    fake_npm_pkg(&stage.dir, "0.171.0");
    stage.discard();
    assert!(!stage.dir.exists());
    assert_eq!(std::fs::read_to_string(prefix.join("bin/codex")).unwrap(), "0.170.0");
}

/// 找到过的命令现在不在了（软链断了）才算失效；相对的命令名（靠 PATH 找）不算
#[cfg(unix)]
#[test]
fn path_gone_follows_broken_links() {
    let d = tempfile::tempdir().unwrap();
    let link = d.path().join("codex");
    std::os::unix::fs::symlink(d.path().join("missing.js"), &link).unwrap();
    assert!(path_gone(&link.display().to_string()));
    std::fs::write(d.path().join("missing.js"), "").unwrap();
    assert!(!path_gone(&link.display().to_string()));
    assert!(!path_gone("codex"));
}

/// 登录 shell 的 PATH 是读启动脚本之后的那份；能据此判断用户目录在不在里面
#[cfg(unix)]
#[test]
fn login_shell_path_is_readable() {
    let env = Env { id: "login".into(), name: "登录 shell".into(), kind: envs::Kind::Login, prog: PathBuf::from("/bin/sh") };
    let path = login_path(&env).expect("登录 shell 能打印 PATH");
    assert!(std::env::split_paths(&path).any(|p| p.as_path() == Path::new("/bin")), "{path}");
    assert!(!path.contains("__BOARD_PATH__"), "标记不能留在结果里：{path}");
}

/// 按用户实际用的 shell 选配置文件；bash 登录 shell 读的是 profile，不是 .bashrc
#[cfg(unix)]
#[test]
fn shell_rc_follows_the_login_shell() {
    let env = |p: &str| Env { id: "login".into(), name: String::new(), kind: envs::Kind::Login, prog: PathBuf::from(p) };
    assert!(login_rc(&env("/bin/zsh")).0.ends_with(".zshrc"));
    assert!(login_rc(&env("/usr/local/bin/fish")).0.ends_with("config.fish"));

    // bash 登录 shell 按 .bash_profile → .bash_login → .profile 的顺序读第一个存在的
    let d = tempfile::tempdir().unwrap();
    assert_eq!(bash_rc(d.path()), d.path().join(".bash_profile"));
    std::fs::write(d.path().join(".profile"), "").unwrap();
    assert_eq!(bash_rc(d.path()), d.path().join(".profile"));
    std::fs::write(d.path().join(".bash_login"), "").unwrap();
    assert_eq!(bash_rc(d.path()), d.path().join(".bash_login"));
}

/// npm 权限失败时不要把最后一行的日志路径当成原因：说清 EACCES 和怎么办
#[test]
fn agent_job_error_explains_permission_failures() {
    let tail = vec![
        "npm error code EACCES".into(),
        "npm error path /usr/local/node-v22/lib/node_modules/@deepseek-ai".into(),
        "npm error The operation was rejected by your operating system.".into(),
        "npm error A complete log of this run can be found in: /Users/me/.npm/_logs/debug-0.log".into(),
    ];
    let why = agent_job_error(243, &tail);
    assert!(why.contains("没有权限写 npm 全局目录") && why.contains("EACCES"), "{why}");
    assert!(!why.contains("npm/_logs/"), "日志路径不是原因：{why}");
}

/// 安装方式按命令实际路径判断：npm 全局 / Homebrew / 官方脚本 / 桌面 app / 配置覆盖 / WSL
#[test]
fn agent_install_methods_are_detected_from_the_real_command() {
    let mut spec = managed_spec("claude", BUILTIN.iter().find(|b| b.id == "claude").unwrap());
    spec.latest = "2.1.325".into();
    spec.checked_at = 42;
    let check = |env: &str, path: &str| Check { env: env.into(), found: true, version: "2.1.300 (Claude Code)".into(), prog: Some(path.into()), path: Some(path.into()), missing: None };
    let npm = spec.manage_for(Some(&check("direct", "/usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js")));
    assert_eq!((npm.method.as_str(), npm.method_label.as_str(), npm.updatable), ("npm-global", "npm 全局", true));
    assert!(npm.update_available);
    let brew = spec.manage_for(Some(&check("direct", "/opt/homebrew/Cellar/claude-code/2.1.300/bin/claude")));
    assert_eq!(brew.method, "brew");
    assert!(brew.method_label.contains("claude-code"));
    let script = spec.manage_for(Some(&check("direct", "/Users/me/.local/bin/claude")));
    assert_eq!(script.method, "script");
    assert!(script.updatable);
    let app = spec.manage_for(Some(&check("direct", "/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh")));
    assert_eq!(app.method, "app");
    assert!(!app.updatable && app.reason.contains("桌面 app"));
    let wsl = spec.manage_for(Some(&check("wsl:Ubuntu", "/home/me/.local/bin/claude")));
    assert_eq!(wsl.method, "wsl");
    assert!(!wsl.updatable);
    let missing = spec.manage_for(None);
    assert_eq!(missing.method, "missing");
    assert!(missing.installable && missing.install_command.contains("npm install -g"));
    spec.managed = false;
    let cfg = spec.manage_for(Some(&check("direct", "/usr/local/bin/claude")));
    assert_eq!(cfg.method, "config");
    assert!(cfg.reason.contains("配置文件"));
}

/// 结构化事件：按行解析、脱敏，异常退出评论里的最后几个事件把工具调用和结果合成一条
#[test]
fn events_are_read_from_the_log_and_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("9.log");
    let upd = |u: serde_json::Value| json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s", "update": u } }).to_string() + "\n";
    let mut raw = String::from("warning: something\n");
    raw += &upd(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "令牌是 sekrit-token-123" } }));
    raw += &upd(json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "kind": "execute", "title": "cargo test", "rawInput": { "command": "cargo test" } }));
    std::fs::write(&log, &raw).unwrap();
    let mut r = test_run(&log, Some(Stream::Acp));
    read_events(&mut r, false, &["sekrit-token-123"]);
    assert_eq!(r.ev_read, raw.len() as u64);
    assert_eq!(r.ev_queue.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["text", "message", "tool"]);
    assert_eq!(r.ev_queue[1].text.as_deref(), Some("令牌是 <已隐藏>"));
    // 结果后到：最后几个事件里合进同一条；最后一行没有换行，last 时也读
    let more = upd(json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "failed", "rawOutput": { "output": "test failed" } }));
    std::fs::write(&log, raw.clone() + more.trim_end()).unwrap();
    read_events(&mut r, false, &[]);
    assert_eq!(r.ev_queue.len(), 3, "不完整的一行先不读");
    read_events(&mut r, true, &[]);
    assert_eq!(r.ev_queue.len(), 4);
    let lines = recent_lines(&r);
    assert_eq!(lines.last().unwrap(), "▸ shell  cargo test  ✗ test failed");
    assert_eq!(lines.len(), 3);
    // 只有原始输出的 agent：只挪位置，不出事件
    let mut r = test_run(&log, None);
    read_events(&mut r, true, &[]);
    assert!(r.ev_queue.is_empty() && r.ev_read > 0);
}

fn test_run(log: &Path, fmt: Option<Stream>) -> Run {
    // 跑一个马上结束的进程：Windows 上没有 `true`，用 cmd 的 exit（这里只在 Windows 上编它，别的平台用 `true`）
    #[cfg(windows)]
    let child = Command::new("cmd").args(["/C", "exit"]).spawn().unwrap();
    #[cfg(not(windows))]
    let child = Command::new("true").spawn().unwrap();
    Run {
        d: Dispatch {
            id: 9,
            board: "default/main".into(),
            card: 1,
            title: String::new(),
            host_id: 1,
            host: "mbp".into(),
            agent: "claude".into(),
            agent_label: "Claude Code".into(),
            name: None,
            env: None,
            env_name: None,
            source: "manual".into(),
            by: board_core::Actor::system(),
            state: "running".into(),
            result: None,
            created_at: 0,
            started_at: None,
            ended_at: None,
            exit_code: None,
            stop_by: None,
            beat_at: None,
            session: 9,
            seq: 1,
            total: 1,
            model: None,
            effort: None,
            kind: String::new(),
            path: None,
            any: false,
            repo: String::new(),
            branch: String::new(),
            profile: String::new(),
            profile_name: String::new(),
            provider: String::new(),
            note: String::new(),
        },
        child,
        _stdin: None,
        job: None,
        env: Env::direct(),
        pidfile: None,
        log: log.to_path_buf(),
        conf: log.with_extension("toml"),
        procfile: log.with_extension("proc"),
        sent: 0,
        fmt,
        parser: fmt.map(events::Parser::new),
        ev_read: 0,
        ev_queue: vec![],
        ev_recent: VecDeque::new(),
        fmt_due: true,
        term_at: None,
        killed: false,
        spawned: Instant::now(),
        cwd: PathBuf::new(),
        managed: false,
        lost: false,
        timeout: None,
        output_at: Instant::now(),
        seen_len: 0,
    }
}

fn test_chat_run(log: &Path) -> ChatRun {
    #[cfg(windows)]
    let child = Command::new("cmd").args(["/C", "exit"]).spawn().unwrap();
    #[cfg(not(windows))]
    let child = Command::new("true").spawn().unwrap();
    ChatRun {
        id: 7,
        child,
        job: None,
        env: Env::direct(),
        stdin: None,
        state: log.with_extension("state"),
        log: log.to_path_buf(),
        procfile: log.with_extension("proc"),
        pidfile: None,
        sent: 0,
        ev_read: 0,
        parser: events::Parser::new(Stream::Acp),
        fmt_due: true,
        took: 0,
        took_sent: 0,
        canceled: 0,
        canceled_sent: 0,
        idle_sent: 0,
        acp_sent: None,
        round_at: None,
        output_at: Instant::now(),
        seen_len: 0,
        limited: None,
    }
}

/// 卡片 #251：`reap` 与 `shutdown` 共用的结束逻辑。兜底停止过的一律报「超时」（原因照原样带上），
/// 否则用调用方给的结果；错误摘录里不出现令牌
#[test]
fn ended_takes_the_timeout_reason_and_redacts() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("9.log");
    std::fs::write(&log, "BOARD_TOKEN=secret-token\nboom\n").unwrap();
    let r = test_run(&log, None);
    let e = ended_of(&r, Some(3), "exit", &["secret-token"], None);
    assert_eq!((e.id, e.code, e.how, e.why.as_deref()), (9, Some(3), "exit", None));
    assert!(e.tail.iter().all(|l| !l.contains("secret-token")), "{:?}", e.tail);
    // board host 退出时 agent 还活着：主进程退出后用调用方给的结果
    let e = ended_of(&r, None, "host_exit", &[], None);
    assert_eq!((e.how, e.why.as_deref()), ("host_exit", None));
    // 兜底停止过：报「超时」，原因写清楚
    let mut r = r;
    r.timeout = Some("运行超过 30 分钟".into());
    let e = ended_of(&r, None, "host_exit", &[], None);
    assert_eq!((e.how, e.why.as_deref()), ("timeout", Some("运行超过 30 分钟")));
}

/// 卡片 #633：board host 退出（升级 / 重启 / 退出桌面端）时也走同一份半成品交接——
/// board host 备的 worktree 有没推的改动就保存成 wip 分支，干净的不产生空分支，不是它备的目录不碰。
#[test]
fn host_exit_handoff_saves_wip_for_managed_worktree() {
    let t = tempfile::tempdir().unwrap();
    let root = plain(&t.path().canonicalize().unwrap());
    let remote = root.join("remote.git");
    std::fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let repo = root.join("board");
    git(&root, &["clone", "-q", remote.to_str().unwrap(), "board"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "a"]);
    git(&repo, &["push", "-q", "origin", "HEAD:main"]);
    let wt = root.join("board.wt").join("win");
    git(&repo, &["worktree", "add", "-q", "--detach", wt.to_str().unwrap(), "HEAD"]);

    let ended = |cwd: &Path, managed: bool| RunSummary {
        board: "default/main".into(),
        card: 9,
        name: Some("win/codex".into()),
        lost: false,
        why: None,
        id: 77,
        cwd: cwd.to_path_buf(),
        managed,
        stopped: true,
        tail: vec![],
    };

    assert_eq!(save_run_wip(&ended(&wt, true), "win/test").unwrap(), None, "干净时不产生空分支");
    std::fs::write(wt.join("half.txt"), "做了一半").unwrap();
    let w = save_run_wip(&ended(&wt, true), "win/test").unwrap().unwrap();
    assert_eq!(w.branch, "wip/main/9");
    assert!(w.changes.contains("half.txt"), "{}", w.changes);
    assert!(crate::code::unpushed(&wt).is_none(), "保存后 worktree 算干净的");
    git(&remote, &["rev-parse", "--verify", "wip/main/9"]);

    std::fs::write(wt.join("not-managed.txt"), "x").unwrap();
    assert_eq!(save_run_wip(&ended(&wt, false), "win/test").unwrap(), None, "不是 board host 备的目录不保存");
    assert!(crate::code::unpushed(&wt).is_some(), "不碰它的改动");
}

#[test]
fn end_reports_retry_only_on_transient_errors() {
    assert!(permanent_end_error(ErrorCode::NotFound), "派发不存在：不用再报");
    assert!(permanent_end_error(ErrorCode::Usage));
    assert!(!permanent_end_error(ErrorCode::Unreachable), "连不上要重试");
    assert!(!permanent_end_error(ErrorCode::Internal), "服务端 5xx 要重试");
}

/// board next 之后同一个进程换到会话里新的那一张；别的会话、旧版服务端（没有 session）不跟随
#[test]
fn run_follows_its_session() {
    let d = |id: i64, session: i64| Dispatch {
        id,
        board: "default/main".into(),
        card: id,
        title: String::new(),
        host_id: 1,
        host: "mbp".into(),
        agent: "claude".into(),
        agent_label: "Claude Code".into(),
        name: Some("claude-mbp/opus-5.5".into()),
        env: None,
        env_name: None,
        source: "cron".into(),
        by: board_core::Actor::system(),
        state: "running".into(),
        result: None,
        created_at: 0,
        started_at: None,
        ended_at: None,
        exit_code: None,
        stop_by: None,
        beat_at: None,
        session,
        seq: 1,
        total: 1,
        model: None,
        effort: None,
        kind: String::new(),
        path: None,
        any: false,
        repo: String::new(),
        branch: String::new(),
        profile: String::new(),
        profile_name: String::new(),
        provider: String::new(),
        note: String::new(),
    };
    let cur = d(1, 1);
    assert_eq!(successor(&cur, &[d(1, 1), d(5, 1), d(3, 3)]).map(|x| x.id), Some(5));
    assert!(successor(&cur, &[d(1, 1), d(3, 3)]).is_none(), "别的会话不跟随");
    assert!(successor(&d(1, 0), &[d(2, 0)]).is_none(), "旧版服务端不跟随");
}

/// 拆解「未整理」里的条目用拆解句子，不让 agent board next；做卡照旧
#[test]
fn split_dispatch_uses_split_prompt() {
    let mut d: Dispatch = serde_json::from_value(json!({
        "id": 7, "board": "default/main", "card": 12, "title": "需求", "hostId": 1, "host": "mbp", "agent": "claude",
        "agentLabel": "Claude Code", "name": "claude-mbp/opus-5.5", "source": "manual", "by": { "kind": "human", "name": "人" },
        "state": "running", "result": null, "createdAt": 0, "startedAt": null, "endedAt": null, "exitCode": null, "stopBy": null, "beatAt": null
    }))
    .unwrap();
    assert_eq!(d.kind, "", "旧服务端没有 kind：按做卡");
    let work = dispatch_prompt(&d, "default", "main", "claude-mbp/opus-5.5", "mbp", None);
    assert!(work.starts_with("/board-skills 开始执行 default项目之中的main白板中的#12任务") && work.contains("board next"), "{work}");
    d.kind = board_core::hosts::KIND_SPLIT.into();
    let split = dispatch_prompt(&d, "default", "main", "claude-mbp/opus-5.5", "mbp", None);
    assert!(split.starts_with("/board-skills 拆解 default项目之中的main白板中的#12需求（board host 派发：BOARD_AS=claude-mbp/opus-5.5，BOARD_HOST=mbp"), "{split}");
    assert!(split.contains("board new --inbox --from 12") && split.contains("不要 board next"), "{split}");
    assert!(split.contains("先按粒度预检") && split.contains("任务粒度合适，未进行拆解"), "粒度预检写在提示词里：{split}");
    // 够小但只是原始需求：整理成一张能直接做的卡，挂在原条目下面（default/main#374）
    assert!(split.contains("只是原始需求的整理成一张能直接做的卡（board new --inbox --from 12）"), "{split}");
}

#[cfg(windows)]
#[test]
fn finds_git_bash_beside_git() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("cmd")).unwrap();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("mingw64").join("bin")).unwrap();
    std::fs::write(root.join("cmd").join("git.exe"), "").unwrap();
    std::fs::write(root.join("mingw64").join("bin").join("git.exe"), "").unwrap();
    std::fs::write(root.join("bin").join("bash.exe"), "").unwrap();
    assert_eq!(envs::git_bash_beside(&root.join("cmd").join("git.exe")), Some(root.join("bin")));
    // PATH 里先找到的是 mingw64\bin 里的 git（#226）：也要找到 <Git>\bin\bash.exe
    assert_eq!(envs::git_bash_beside(&root.join("mingw64").join("bin").join("git.exe")), Some(root.join("bin")));
    assert_eq!(envs::git_bash_beside(&root.join("bin").join("git.exe")), Some(root.join("bin")));
    // 只有 usr\bin\bash.exe 的老布局
    let usr = tempfile::tempdir().unwrap();
    let u = usr.path().join("mingw64").join("bin");
    std::fs::create_dir_all(&u).unwrap();
    std::fs::create_dir_all(usr.path().join("usr").join("bin")).unwrap();
    std::fs::write(u.join("git.exe"), "").unwrap();
    std::fs::write(usr.path().join("usr").join("bin").join("bash.exe"), "").unwrap();
    assert_eq!(envs::git_bash_beside(&u.join("git.exe")), Some(usr.path().join("usr").join("bin")));
    // 哪里都没有 bash.exe
    let bare = tempfile::tempdir().unwrap();
    let b = bare.path().join("mingw64").join("bin");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("git.exe"), "").unwrap();
    assert_eq!(envs::git_bash_beside(&b.join("git.exe")), None);
}
