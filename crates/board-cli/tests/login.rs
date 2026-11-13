//! 配置文件与登录（PRD §9）：`board login` 校验令牌后写 `~/.board/config.toml`（0600），之后不设环境变量也能用；
//! 环境变量优先于配置文件；`board logout` 只删令牌。HOME 指向临时目录（tests/common）。

mod common;

use common::{board, board_stdin, config_shown, start, Server};
use std::path::PathBuf;

fn config(s: &Server) -> PathBuf {
    s.home.join(".board").join("config.toml")
}

fn read(s: &Server) -> String {
    std::fs::read_to_string(config(s)).unwrap_or_default()
}

/// 只用参数（不设任何环境变量）
const ARGS: [&str; 4] = ["--as", "test/alpha", "--board", "default"];

fn with_args<'a>(cmd: &[&'a str]) -> Vec<&'a str> {
    let mut v = cmd.to_vec();
    v.extend(ARGS);
    v
}

#[test]
fn login_writes_config_and_commands_work_without_env() {
    let s = start();
    let r = board(&s.home, &[], &["login", "--url", &s.url, "--token", &s.token]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("已登录") && r.out.contains(config_shown()), "{}", r.out);
    assert!(!r.out.contains(&s.token) && !r.err.contains(&s.token), "输出里不打印令牌原文");
    let text = read(&s);
    assert!(text.contains(&format!("url = \"{}\"", s.url)) && text.contains(&s.token), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&config(&s)), 0o600, "文件权限 0600");
        assert_eq!(mode(config(&s).parent().unwrap()), 0o700, "目录权限 0700");
    }

    // 不设任何环境变量：读命令、check、写命令都可以
    let r = board(&s.home, &[], &with_args(&["ls", "--json"]));
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(board(&s.home, &[], &with_args(&["check", "--json"])).code, 0);
    let r = board(&s.home, &[], &with_args(&["new", "配置文件建的", "--json"]));
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);

    // 改写时保留其他配置项（和注释）；不带 --url 沿用文件里的地址，不带 --token 从标准输入读一行
    std::fs::write(config(&s), format!("{text}\n# 以后的配置\neditor = \"vim\"\n")).unwrap();
    let r = board_stdin(&s.home, &[], &["login", "--json"], Some(&format!("{}\n", s.token)));
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["url"], s.url.as_str());
    assert!(!r.out.contains(&s.token));
    let text = read(&s);
    assert!(text.contains("# 以后的配置") && text.contains("editor = \"vim\""), "{text}");
    assert_eq!(text.matches("token =").count(), 1, "{text}");
}

#[test]
fn login_rejects_bad_token_and_keeps_file() {
    let s = start();
    assert_eq!(board(&s.home, &[], &["login", "--url", &s.url, "--token", &s.token]).code, 0);
    let before = read(&s);
    // 令牌错误：INVALID_TOKEN，配置文件不变
    let r = board(&s.home, &[], &["login", "--url", &s.url, "--token", "bd_nope", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "INVALID_TOKEN"));
    assert_eq!(read(&s), before);
    // 连不上：UNREACHABLE，配置文件不变
    let r = board(&s.home, &[], &["login", "--url", "http://127.0.0.1:9", "--token", &s.token, "--json"]);
    assert_eq!(r.error_code(), "UNREACHABLE");
    assert_eq!(read(&s), before);
    // 地址格式不对
    let r = board(&s.home, &[], &["login", "--url", "board.example.com", "--token", "x", "--json"]);
    assert_eq!(r.error_code(), "USAGE");
    assert_eq!(read(&s), before);

    // 没有 --url，文件里也没有：USAGE
    let fresh = tempfile::tempdir().unwrap();
    let r = board(fresh.path(), &[], &["login", "--token", &s.token, "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("--url"));
    assert!(!fresh.path().join(".board").exists());
}

#[test]
fn env_overrides_config_file() {
    let s = start();
    assert_eq!(board(&s.home, &[], &["login", "--url", &s.url, "--token", &s.token]).code, 0);
    // 环境变量的地址优先：指向连不上的地址就报 UNREACHABLE
    let r = board(&s.home, &[("BOARD_URL", "http://127.0.0.1:9".into())], &with_args(&["ls", "--json"]));
    assert_eq!(r.error_code(), "UNREACHABLE");
    // 环境变量的令牌优先
    let r = board(&s.home, &[("BOARD_TOKEN", "bd_nope".into())], &with_args(&["ls", "--json"]));
    assert_eq!(r.error_code(), "INVALID_TOKEN");
    // 登录时设置了环境变量：提醒删掉 shell 配置文件里的 export
    let r = board(&s.home, &[("BOARD_TOKEN", s.token.clone())], &["login", "--url", &s.url, "--token", &s.token]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("环境变量 BOARD_TOKEN 会覆盖配置文件") && r.out.contains("export BOARD_TOKEN"), "{}", r.out);
    assert!(!r.out.contains(&s.token));
    let r = board(&s.home, &[("BOARD_URL", s.url.clone())], &["login", "--url", &s.url, "--token", &s.token, "--json"]);
    assert_eq!(r.json()["env_overrides"], serde_json::json!(["BOARD_URL"]));
}

#[test]
fn login_recovers_invalid_toml_config() {
    let s = start();
    let dir = s.home.join(".board");
    std::fs::create_dir_all(&dir).unwrap();
    let bad = "# 旧配置\n[agents.claude]\ncmd = [\"bash\", \"-c\", \"echo agent \\; exec sleep 300\", \"-\"]\n";
    std::fs::write(config(&s), bad).unwrap();

    // 配置坏了也要能登录：带 --url 时不先读文件，校验通过后备份原文件再写新配置
    let r = board(&s.home, &[], &["login", "--url", &s.url, "--token", &s.token, "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(!r.out.contains(&s.token));
    let text = read(&s);
    assert!(text.contains(&format!("url = \"{}\"", s.url)) && text.contains(&format!("token = \"{}\"", s.token)), "{text}");
    let backups: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("config.toml.bad-"))
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(std::fs::read_to_string(dir.join(&backups[0])).unwrap(), bad);

    // 写完之后配置文件能正常读：ls 不报 TOML parse error
    let r = board(&s.home, &[], &with_args(&["ls", "--json"]));
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
}

#[test]
fn logout_removes_token_and_keeps_url() {
    let s = start();
    assert_eq!(board(&s.home, &[], &["login", "--url", &s.url, "--token", &s.token]).code, 0);
    let r = board(&s.home, &[], &["logout"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let text = read(&s);
    assert!(text.contains(&s.url) && !text.contains("token"), "{text}");
    let r = board(&s.home, &[], &with_args(&["ls", "--json"]));
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_LOGGED_IN"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("board login"));
    // 再退出一次也可以
    assert_eq!(board(&s.home, &[], &["logout", "--json"]).json()["removed"], false);
    // 文件里留着地址：login 只给令牌就行
    let r = board_stdin(&s.home, &[], &["login"], Some(&s.token));
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(board(&s.home, &[], &with_args(&["ls", "--json"])).code, 0);
}
