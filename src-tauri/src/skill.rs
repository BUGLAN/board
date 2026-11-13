//! 「设置 → 本机」的「一键安装 board-skills」：桌面端直接把 skill 写进这台电脑上 agent 读 skill 的用户级目录，
//! 不经过 board host，也不联网。
//!
//! - skill 的文字部分（`SKILL.md` 和按场景的分文件 `work.md`、`plan.md`、`host.md`、`reference.md`，`scripts/`）编译时打进桌面端，`bin/<平台>/board` 用桌面端自带的 board 命令行
//!   （[`crate::cli_path`]），和桌面端、board host 是同一个版本。
//! - 装到哪由 agent 决定（[`user_dir`]，和 `docs/agents.md`「接入前准备」里「每个 agent 读哪个目录」的表、board host 的内置模板一致）；
//!   页面只传 agent 的标识，不能指定任意目录。
//! - 先写到旁边的临时目录，写完再整体换上去：装到一半失败不会留下半个 skill。已经有的是链接（开发时
//!   `ln -sfn` 到仓库）并且看得到 `SKILL.md` 就不动。
//! - board host 在跑时不自己写：通知它（标准输入一行 `reload`）马上按 skills 共用库同步一次（board-skills 跟着服务端版本，
//!   PRD §10.12「共用配置」），它重新检查并上报，界面上的提醒随之消失。board host 没在跑时才由桌面端直接写。

use std::path::{Path, PathBuf};
use tauri::AppHandle;

const SKILL: &str = "board-skills";

/// skill 的文字部分：（相对路径，内容，要不要可执行权限）
const FILES: &[(&str, &[u8], bool)] = &[
    ("SKILL.md", include_bytes!("../../.agents/skills/board-skills/SKILL.md"), false),
    // 按场景的分文件：SKILL.md 的「按场景读哪个文件」指到它们，少一个 agent 就读不到那部分规则
    ("work.md", include_bytes!("../../.agents/skills/board-skills/work.md"), false),
    ("plan.md", include_bytes!("../../.agents/skills/board-skills/plan.md"), false),
    ("host.md", include_bytes!("../../.agents/skills/board-skills/host.md"), false),
    ("reference.md", include_bytes!("../../.agents/skills/board-skills/reference.md"), false),
    ("scripts/board", include_bytes!("../../.agents/skills/board-skills/scripts/board"), true),
    ("scripts/board.cmd", include_bytes!("../../.agents/skills/board-skills/scripts/board.cmd"), false),
    ("scripts/heartbeat.sh", include_bytes!("../../.agents/skills/board-skills/scripts/heartbeat.sh"), true),
    ("scripts/heartbeat.ps1", include_bytes!("../../.agents/skills/board-skills/scripts/heartbeat.ps1"), false),
];

/// 本机平台在 skill 的 `bin/` 下的目录名（和 `scripts/board`、`scripts/build-skill-bin.mjs` 一致）
fn platform_dir() -> Option<&'static str> {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        return Some("windows-x64");
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        return None;
    };
    Some(match (os, arch) {
        ("darwin", "arm64") => "darwin-arm64",
        ("darwin", _) => "darwin-x64",
        ("linux", "arm64") => "linux-arm64",
        _ => "linux-x64",
    })
}

/// agent 读用户级 skill 的目录（board host 内置模板的 `skills` 第一项）；不认识的 agent 返回 None（界面不给按钮）
pub fn user_dir(agent: &str, home: &Path, codex_home: Option<&str>) -> Option<PathBuf> {
    match agent {
        "dsh" | "pi" => Some(home.join(".agents").join("skills")),
        "claude" => Some(home.join(".claude").join("skills")),
        "codex" => Some(match codex_home.map(str::trim).filter(|s| !s.is_empty()) {
            Some(c) => PathBuf::from(c).join("skills"),
            None => home.join(".codex").join("skills"),
        }),
        _ => None,
    }
}

/// 把 skill 装到 `root/board-skills`，返回装到的目录。`cli` 是要放进 `bin/<平台>/` 的 board 命令行
pub fn install_into(root: &Path, cli: Option<&Path>) -> Result<PathBuf, String> {
    let target = root.join(SKILL);
    // 开发时链接到仓库里的 skill：链接有效就不动（坏链接当作没装，换成真的目录）
    if let Ok(m) = std::fs::symlink_metadata(&target) {
        if m.file_type().is_symlink() {
            if target.join("SKILL.md").is_file() {
                return Ok(target);
            }
            remove_link(&target).map_err(|e| format!("删不掉坏掉的链接 {}：{e}", target.display()))?;
        }
    }
    std::fs::create_dir_all(root).map_err(|e| format!("建不了目录 {}：{e}", root.display()))?;
    let tmp = root.join(format!(".{SKILL}.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = write_skill(&tmp, cli) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    // 换上去：旧的先挪开，新的放进来，再删旧的（挪不开就不装，旧的原样留着）
    let old = root.join(format!(".{SKILL}.old-{}", std::process::id()));
    if target.exists() {
        let _ = std::fs::remove_dir_all(&old);
        if let Err(e) = std::fs::rename(&target, &old) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!("换不掉原来的 {}：{e}", target.display()));
        }
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::rename(&old, &target);
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("装不到 {}：{e}", target.display()));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(target)
}

fn write_skill(dir: &Path, cli: Option<&Path>) -> Result<(), String> {
    for (rel, body, exec) in FILES {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建不了目录 {}：{e}", parent.display()))?;
        }
        std::fs::write(&p, body).map_err(|e| format!("写不了 {}：{e}", p.display()))?;
        if *exec {
            set_exec(&p)?;
        }
    }
    if let (Some(cli), Some(plat)) = (cli, platform_dir()) {
        let name = if cfg!(target_os = "windows") { "board.exe" } else { "board" };
        let to = dir.join("bin").join(plat).join(name);
        std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| format!("建不了目录 {}：{e}", to.display()))?;
        std::fs::copy(cli, &to).map_err(|e| format!("复制 board 命令行失败：{e}"))?;
        set_exec(&to)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_exec(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("改不了 {} 的权限：{e}", p.display()))
}

#[cfg(not(unix))]
fn set_exec(_: &Path) -> Result<(), String> {
    Ok(())
}

/// 删掉一个链接本身（不碰它指向的目录）。Windows 上目录链接要用 remove_dir
fn remove_link(p: &Path) -> std::io::Result<()> {
    std::fs::remove_file(p).or_else(|_| std::fs::remove_dir(p))
}

/// 页面调用：给这几个 agent 装 board-skills（同一个目录只装一次），返回装到的目录。
/// 装好后通知在跑的 board host 重新检查
#[tauri::command]
pub async fn skill_install(app: AppHandle, agents: Vec<String>) -> Result<Vec<String>, String> {
    let home = board_core::config::home().ok_or("找不到用户目录")?;
    let codex = std::env::var("CODEX_HOME").ok();
    let mut roots: Vec<PathBuf> = Vec::new();
    for a in &agents {
        let r = user_dir(a, &home, codex.as_deref()).ok_or_else(|| format!("不知道 {a} 从哪个目录读 skill，请按 docs/agents.md 手动安装"))?;
        if !roots.contains(&r) {
            roots.push(r);
        }
    }
    // board host 在跑：交给它按 skills 共用库同步（board-skills 跟着服务端的版本），通知它马上做一次
    if crate::host::notify_reload(&app) {
        return Ok(roots.iter().map(|r| r.join(SKILL).display().to_string()).collect());
    }
    let cli = crate::cli_path();
    let mut done = Vec::new();
    for r in &roots {
        done.push(install_into(r, cli.as_deref())?.display().to_string());
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("board-skill-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn dirs_match_builtin_agents() {
        let h = Path::new("/h");
        assert_eq!(user_dir("claude", h, None), Some(h.join(".claude/skills")));
        assert_eq!(user_dir("dsh", h, None), Some(h.join(".agents/skills")));
        assert_eq!(user_dir("pi", h, None), Some(h.join(".agents/skills")));
        assert_eq!(user_dir("codex", h, None), Some(h.join(".codex/skills")));
        assert_eq!(user_dir("codex", h, Some("/c")), Some(PathBuf::from("/c/skills")));
        assert_eq!(user_dir("codex", h, Some("  ")), Some(h.join(".codex/skills")), "空的 CODEX_HOME 当没设");
        assert_eq!(user_dir("mine", h, None), None);
    }

    #[test]
    fn installs_and_replaces() {
        let d = temp("install");
        let cli = d.join("board-cli");
        std::fs::write(&cli, b"bin").unwrap();
        let root = d.join("skills");
        let to = install_into(&root, Some(&cli)).unwrap();
        assert_eq!(to, root.join(SKILL));
        assert!(to.join("SKILL.md").is_file());
        for f in ["work.md", "plan.md", "host.md", "reference.md"] {
            assert!(to.join(f).is_file(), "一键安装要带上分文件 {f}");
        }
        assert!(to.join("scripts/board").is_file());
        if let Some(p) = platform_dir() {
            let name = if cfg!(target_os = "windows") { "board.exe" } else { "board" };
            assert_eq!(std::fs::read(to.join("bin").join(p).join(name)).unwrap(), b"bin");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(std::fs::metadata(to.join("scripts/board")).unwrap().permissions().mode() & 0o111, 0, "脚本要能执行");
        }
        // 再装一次：旧的整个换掉（多出来的文件不留），旁边不留临时目录
        std::fs::write(to.join("stale.txt"), b"x").unwrap();
        install_into(&root, None).unwrap();
        assert!(!to.join("stale.txt").exists());
        let names: Vec<String> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec![SKILL.to_string()]);
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[test]
    fn keeps_working_link_and_replaces_broken_one() {
        let d = temp("link");
        let repo = d.join("repo-skill");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("SKILL.md"), b"dev").unwrap();
        let root = d.join("skills");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&repo, root.join(SKILL)).unwrap();
        install_into(&root, None).unwrap();
        assert_eq!(std::fs::read(root.join(SKILL).join("SKILL.md")).unwrap(), b"dev", "有效的链接不动");

        std::fs::remove_dir_all(&repo).unwrap();
        install_into(&root, None).unwrap();
        let m = std::fs::symlink_metadata(root.join(SKILL)).unwrap();
        assert!(!m.file_type().is_symlink() && root.join(SKILL).join("SKILL.md").is_file(), "坏链接换成真的目录");
        std::fs::remove_dir_all(&d).ok();
    }
}
