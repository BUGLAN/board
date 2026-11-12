//! 开机自启的服务定义（PRD §9「让机器重启后自动接任务」）：只生成文件内容，写文件、注册由调用方做。
//!
//! - `board host --install`：macOS 的 launchd 用户代理、Linux 的 systemd 用户服务、Windows 登录时运行的计划任务，
//!   都运行 `board host --service …`（守护进程：异常退出 10 秒后重启 board host，输出写进 `~/.board/host/host.log` 并轮转）。
//!   守护进程自己崩溃时由 launchd（KeepAlive、ThrottleInterval 10 秒）、systemd（Restart=on-failure、RestartSec=10）、
//!   计划任务（失败后 1 分钟重试）再拉起来。
//! - 桌面端「开机时自动启动」：macOS 也是 launchd 用户代理（登录时执行一次，不保活）。

/// `board host --install` 的 launchd 标签（`~/Library/LaunchAgents/<标签>.plist`）
pub const LAUNCHD_HOST: &str = "dev.board.host";
/// 桌面端开机自启的 launchd 标签
pub const LAUNCHD_DESKTOP: &str = "dev.board.desktop";
/// systemd 用户服务名（`~/.config/systemd/user/<名字>`）
pub const SYSTEMD_HOST: &str = "board-host.service";
/// Windows 计划任务名
pub const WINDOWS_TASK: &str = "Board host";
/// 异常退出后多久重启（秒）
pub const RESTART_SECS: u64 = 10;

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// launchd 用户代理。`keep_alive`：异常退出（非 0）后重新拉起，间隔 [`RESTART_SECS`]；否则只在登录时执行一次。
/// `log`：launchd 把程序自己的标准输出、标准错误写到这里（守护进程平时不往这里写，只有它自己崩溃时有内容）
pub fn launchd_plist(label: &str, args: &[String], env: &[(String, String)], keep_alive: bool, log: Option<&str>) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n",
    );
    s += &format!("  <key>Label</key>\n  <string>{}</string>\n", xml_escape(label));
    s += "  <key>ProgramArguments</key>\n  <array>\n";
    for a in args {
        s += &format!("    <string>{}</string>\n", xml_escape(a));
    }
    s += "  </array>\n  <key>RunAtLoad</key>\n  <true/>\n";
    if keep_alive {
        s += "  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n";
        s += &format!("  <key>ThrottleInterval</key>\n  <integer>{RESTART_SECS}</integer>\n");
    }
    if !env.is_empty() {
        s += "  <key>EnvironmentVariables</key>\n  <dict>\n";
        for (k, v) in env {
            s += &format!("    <key>{}</key>\n    <string>{}</string>\n", xml_escape(k), xml_escape(v));
        }
        s += "  </dict>\n";
    }
    if let Some(l) = log {
        s += &format!("  <key>StandardOutPath</key>\n  <string>{0}</string>\n  <key>StandardErrorPath</key>\n  <string>{0}</string>\n", xml_escape(l));
    }
    s += "</dict>\n</plist>\n";
    s
}

/// systemd 的 ExecStart 里的一个参数：双引号包起来，`\`、`"` 转义；`%`（说明符）写成 `%%`、`$`（变量）写成 `$$`
pub fn systemd_quote(arg: &str) -> String {
    let inner = arg.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$");
    format!("\"{inner}\"")
}

/// systemd 用户服务：异常退出 [`RESTART_SECS`] 秒后重启；登录后（开了 linger 时开机后）启动
pub fn systemd_unit(args: &[String], env: &[(String, String)]) -> String {
    let mut s = String::from("[Unit]\nDescription=Board host：在这台机器上接派发的卡片（board host --install 生成）\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\n");
    s += &format!("ExecStart={}\n", args.iter().map(|a| systemd_quote(a)).collect::<Vec<_>>().join(" "));
    s += &format!("Restart=on-failure\nRestartSec={RESTART_SECS}\n");
    for (k, v) in env {
        s += &format!("Environment={}\n", systemd_quote(&format!("{k}={v}")));
    }
    s += "\n[Install]\nWantedBy=default.target\n";
    s
}

/// PowerShell 单引号字符串
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Windows 计划任务（`schtasks /Create /XML`）：这个用户登录时运行，不限运行时长，失败 1 分钟后重试（守护进程自己崩溃时）。
/// 控制台程序在登录时直接运行会弹出黑窗口：经 `powershell -WindowStyle Hidden` 启动
pub fn windows_task_xml(user: &str, args: &[String]) -> String {
    let (prog, rest) = args.split_first().map(|(p, r)| (p.as_str(), r)).unwrap_or(("board.exe", &[]));
    let cmd = format!("& {} {}", ps_quote(prog), rest.iter().map(|a| ps_quote(a)).collect::<Vec<_>>().join(" "));
    let ps_args = format!("-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -Command \"{}\"", cmd.replace('"', "\\\""));
    let user = xml_escape(user);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Board host：在这台机器上接派发的卡片（board host --install 生成）</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>powershell.exe</Command>
      <Arguments>{}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        xml_escape(&ps_args)
    )
}

/// Linux 桌面环境的开机自启（`~/.config/autostart/<名字>.desktop`）
pub fn desktop_entry(name: &str, args: &[String]) -> String {
    let exec = args
        .iter()
        .map(|a| if a.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c)) { format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`")) } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ");
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nX-GNOME-Autostart-enabled=true\nNoDisplay=false\n")
}

/// UTF-16LE（带 BOM）：schtasks 读的任务 XML 用它
pub fn utf16le(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn launchd_host_agent_restarts_after_failure() {
        let args = sv(&["/Users/me/.local/bin/board", "host", "--service", "--name", "mbp", "--dir", "a/b=~/repo/<x>&y"]);
        let env = vec![("PATH".to_string(), "/opt/homebrew/bin:/usr/bin".to_string())];
        let p = launchd_plist(LAUNCHD_HOST, &args, &env, true, Some("/Users/me/.board/host/service.out"));
        assert!(p.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(p.contains("<key>Label</key>\n  <string>dev.board.host</string>"));
        assert!(p.contains("    <string>/Users/me/.local/bin/board</string>\n    <string>host</string>\n    <string>--service</string>"));
        assert!(p.contains("<string>a/b=~/repo/&lt;x&gt;&amp;y</string>"), "XML 转义：{p}");
        assert!(p.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(p.contains("<key>SuccessfulExit</key>\n    <false/>"), "只在异常退出时拉起");
        assert!(p.contains("<key>ThrottleInterval</key>\n  <integer>10</integer>"));
        assert!(p.contains("<key>PATH</key>\n    <string>/opt/homebrew/bin:/usr/bin</string>"));
        assert!(p.contains("<key>StandardErrorPath</key>"));
        assert!(p.trim_end().ends_with("</plist>"));
        // 桌面端开机自启：只在登录时执行一次
        let d = launchd_plist(LAUNCHD_DESKTOP, &sv(&["/usr/bin/open", "-g", "-a", "/Applications/Board.app", "--args", "--autostart"]), &[], false, None);
        assert!(!d.contains("KeepAlive") && !d.contains("StandardOutPath"));
    }

    /// macOS 上用系统自带的 plutil 校验生成的 plist 格式（只校验文件，不注册服务）
    #[cfg(target_os = "macos")]
    #[test]
    fn launchd_plist_passes_plutil() {
        let d = std::env::temp_dir().join(format!("board-plist-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("dev.board.host.plist");
        let args = sv(&["/x/board", "host", "--service", "--dir", "a/b=~/r & <q>"]);
        std::fs::write(&f, launchd_plist(LAUNCHD_HOST, &args, &[("PATH".into(), "/usr/bin".into())], true, Some("/tmp/o.log"))).unwrap();
        let out = std::process::Command::new("plutil").arg("-lint").arg(&f).output().unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    }

    #[test]
    fn systemd_unit_quotes_and_restarts() {
        let args = sv(&["/home/me/.local/bin/board", "host", "--service", "--dir", "a/b=/srv/my repo/50%\"$x\""]);
        let u = systemd_unit(&args, &[("PATH".into(), "/usr/local/bin:/usr/bin".into())]);
        assert!(u.contains("ExecStart=\"/home/me/.local/bin/board\" \"host\" \"--service\" \"--dir\" \"a/b=/srv/my repo/50%%\\\"$$x\\\"\""), "{u}");
        assert!(u.contains("Restart=on-failure\nRestartSec=10\n"));
        assert!(u.contains("Environment=\"PATH=/usr/local/bin:/usr/bin\""));
        assert!(u.contains("[Install]\nWantedBy=default.target"));
        assert_eq!(systemd_quote("C:\\x"), "\"C:\\\\x\"");
    }

    #[test]
    fn windows_task_runs_hidden_at_logon_without_time_limit() {
        let args = sv(&["C:\\Users\\me\\AppData\\Local\\Board\\bin\\board.exe", "host", "--service", "--dir", "a/b=C:\\repo\\it's"]);
        let x = windows_task_xml("PC\\me", &args);
        assert!(x.contains("<LogonTrigger>") && x.contains("<UserId>PC\\me</UserId>"));
        assert!(x.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"), "默认 72 小时会被结束");
        assert!(x.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(x.contains("<Command>powershell.exe</Command>"));
        assert!(x.contains("-WindowStyle Hidden"));
        // XML 里的 Arguments 还原出来就是 PowerShell 命令行
        let raw = x.split("<Arguments>").nth(1).unwrap().split("</Arguments>").next().unwrap();
        let args_text = raw.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
        assert!(
            args_text.contains("-Command \"& 'C:\\Users\\me\\AppData\\Local\\Board\\bin\\board.exe' 'host' '--service' '--dir' 'a/b=C:\\repo\\it''s'\""),
            "{args_text}"
        );
        assert!(!raw.contains('"') && !raw.contains('\''), "Arguments 里的引号都转义了：{raw}");
        let b = utf16le("a");
        assert_eq!(b, vec![0xFF, 0xFE, b'a', 0]);
    }

    #[test]
    fn linux_desktop_autostart_entry() {
        let e = desktop_entry("Board", &sv(&["/opt/Board/board-desktop", "--autostart"]));
        assert!(e.contains("Exec=/opt/Board/board-desktop --autostart"));
        let e = desktop_entry("Board", &sv(&["/opt/my apps/board", "--autostart"]));
        assert!(e.contains("Exec=\"/opt/my apps/board\" --autostart"));
    }
}
