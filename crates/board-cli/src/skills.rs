//! skills 共用库在本机这一侧（PRD §9「board skills」、§10.12「共用配置」）：
//!
//! - `board skills ls`：库里的 skill，以及这台机器上每个 agent skill 目录里的状态
//! - `board skills push <目录>…`：把本机的 skill 目录传进库里（目录名就是 skill 名，里面要有 SKILL.md），同名的覆盖成新版本
//! - `board skills sync`：按库更新本机；本机有 board host 在跑时由它来做
//! - `board skills rm <名称>`：从库里删掉，各台机器上同步装的那份随后被删掉
//! - board host 启动时、库变了时（服务端推送）、重新检查后、每 10 分钟，在后台调 [`sync`]：把库里的 skill 写进本机每个
//!   找到的 agent 读用户级 skill 的目录（内置模板的第一个：`~/.agents/skills`、`~/.claude/skills`、`$CODEX_HOME/skills`；
//!   WSL 里的 agent 写到 WSL 用户目录下），结果按 agent、skill 逐项上报。
//!
//! 写法：先写进同一目录下的临时目录，写完整体换上；目录里放标记文件 `.board-sync`（名称、版本）。只动带标记的：
//! 本机已有同名、没有标记的目录（包括开发用的链接）不覆盖，状态是「本机有同名，没有覆盖」；库里删掉的、带标记的删掉。
//! board-skills 例外：不是链接的旧拷贝（以前桌面端一键安装、解压 board-skills.zip 装的，没有标记）换成同步装的。
//! board-skills 写好后把 `bin/<平台>/board` 换成 board host 自己的 board（WSL 里不放：那里用 PATH 里的 Linux 版）。
//! 本机自己的同名目录内容和库里的一样（例如就是从这台机器导入的）时算已同步，也不动它。
//!
//! 从机器导入（界面「从机器导入」）：board host 每次同步后列出各目录里不是同步装的 skill（[`local_skills`]）随上报带上；
//! 人勾了要导入的，board host 用 [`import_one`] 把那几个目录传进库（和 `board skills push` 一样），本机的目录不动。

use crate::backend::Remote;
use crate::{Output, Paint};
use board_core::skills::{self, LocalSkill, SkillFile, SkillInfo, SkillPack, SkillState, BUILTIN, MARK, MAX_BYTES};
use board_core::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 一个要同步的目录
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// 读这个目录的 agent（几个 agent 共用一个目录时都在这里）
    pub agents: Vec<String>,
    /// 启动环境
    pub env: String,
    /// 本机能读写的路径（WSL 的是 \\wsl.localhost\…）
    pub dir: PathBuf,
    /// 显示用（WSL 里写 WSL 里的写法）
    pub shown: String,
    pub wsl: bool,
}

/// 把 (agent, 环境, 路径, 显示, wsl) 按路径合并成目录列表
pub fn merge(list: Vec<(String, String, PathBuf, String, bool)>) -> Vec<Target> {
    let mut out: Vec<Target> = vec![];
    for (agent, env, dir, shown, wsl) in list {
        match out.iter_mut().find(|t| t.dir == dir) {
            Some(t) => {
                if !t.agents.contains(&agent) {
                    t.agents.push(agent);
                }
            }
            None => out.push(Target { agents: vec![agent], env, dir, shown, wsl }),
        }
    }
    out
}

/// 不在 board host 里时（`board skills ls / sync`）：内置模板里每个 agent 的第一个用户级 skill 目录，上一级目录在的
/// （装了这个 agent 才有 ~/.claude、~/.codex 这些）
pub fn local_targets() -> Vec<Target> {
    let list = [("dsh", "~/.agents/skills"), ("pi", "~/.agents/skills"), ("claude", "~/.claude/skills"), ("codex", "$CODEX_HOME/skills")];
    merge(
        list.iter()
            .map(|(a, d)| (a.to_string(), "direct".to_string(), crate::envs::expand(d), d.to_string(), false))
            .filter(|(_, _, p, _, _)| p.parent().is_some_and(Path::is_dir))
            .collect(),
    )
}

/// board host 用：每个找到的 agent（标识、实际用的启动环境、模板里第一个用户级 skill 目录）→ 要同步的目录。
/// WSL 里的换成 Windows 上能访问的路径；取不到的那几个 agent 记成失败
pub fn host_targets(list: Vec<(String, crate::envs::Env, String)>, lib: &[SkillInfo]) -> (Vec<Target>, Vec<SkillState>) {
    let mut ok = vec![];
    let mut bad = vec![];
    for (agent, env, dir) in list {
        if env.wsl().is_some() {
            match crate::envs::wsl_unc(&env, &dir) {
                Some(p) => ok.push((agent, env.id.clone(), p, format!("{}：{dir}", env.name), true)),
                None => {
                    for s in lib {
                        bad.push(SkillState {
                            skill: s.name.clone(),
                            agent: agent.clone(),
                            env: env.id.clone(),
                            dir: format!("{}：{dir}", env.name),
                            state: "failed".into(),
                            version: None,
                            why: Some(format!("取不到{}里的 {dir}（WSL 没启动或没有这个发行版）", env.name)),
                        });
                    }
                }
            }
        } else {
            ok.push((agent, env.id.clone(), crate::envs::expand(&dir), dir, false));
        }
    }
    (merge(ok), bad)
}

/// 一个目录里同步装的 skill：名称 → 版本
fn synced_in(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if let Some(v) = mark_of(&e.path()) {
            out.insert(name, v);
        }
    }
    out
}

/// 同步装的 skill 目录（不是链接、有标记文件）的版本
fn mark_of(dir: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(MARK)).ok()?).ok()?;
    v["version"].as_str().map(str::to_string)
}

/// 一个 skill 在一个目录里的状态（不联网，`ls` 用）：synced | behind | local | missing
pub fn local_state(dir: &Path, skill: &SkillInfo) -> (&'static str, Option<String>) {
    let dest = dir.join(&skill.name);
    if std::fs::symlink_metadata(&dest).is_err() {
        return ("missing", None);
    }
    match mark_of(&dest) {
        Some(v) if v == skill.version => ("synced", Some(v)),
        Some(v) => ("behind", Some(v)),
        None => ("local", None),
    }
}

/// 写一个 skill：先写进临时目录，写完整体换上
fn install(dir: &Path, pack: &SkillPack, exe: Option<&Path>) -> std::result::Result<(), String> {
    let name = &pack.info.name;
    let pid = std::process::id();
    let tmp = dir.join(format!(".{name}.sync-{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    let res = (|| -> std::result::Result<(), String> {
        for f in &pack.files {
            skills::check_path(&f.path).map_err(|e| e.message)?;
            let p = tmp.join(&f.path);
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| format!("建不了 {}：{e}", d.display()))?;
            }
            std::fs::write(&p, skills::unb64(&f.data).map_err(|e| e.message)?).map_err(|e| format!("写不了 {}：{e}", p.display()))?;
            #[cfg(unix)]
            if f.exec {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
            }
        }
        // board-skills：bin/<平台>/board 用 board host 自己的
        if let (true, Some(exe), Some(plat)) = (name == BUILTIN, exe, crate::update::platform_of(std::env::consts::OS, std::env::consts::ARCH)) {
            let bin = tmp.join("bin").join(plat);
            std::fs::create_dir_all(&bin).map_err(|e| format!("建不了 {}：{e}", bin.display()))?;
            let to = bin.join(format!("board{}", std::env::consts::EXE_SUFFIX));
            // 同一个磁盘上用硬链接（不占空间，瞬间完成），不行再复制
            if std::fs::hard_link(exe, &to).is_err() {
                std::fs::copy(exe, &to).map_err(|e| format!("放不进自己的 board（{}）：{e}", to.display()))?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755));
            }
        }
        let mark = json!({ "name": name, "version": pack.info.version, "at": now_ms(), "by": "board skills（共用库同步装的，改动会被下一次同步换掉）" });
        std::fs::write(tmp.join(MARK), serde_json::to_string_pretty(&mark).unwrap_or_default()).map_err(|e| format!("写不了标记文件：{e}"))?;
        let dest = dir.join(name);
        if std::fs::symlink_metadata(&dest).is_ok() {
            let old = dir.join(format!(".{name}.old-{pid}"));
            let _ = std::fs::remove_dir_all(&old);
            std::fs::rename(&dest, &old).map_err(|e| format!("换不下旧版本：{e}"))?;
            if let Err(e) = std::fs::rename(&tmp, &dest) {
                let _ = std::fs::rename(&old, &dest);
                return Err(format!("换不上新版本：{e}"));
            }
            let _ = std::fs::remove_dir_all(&old);
        } else {
            std::fs::rename(&tmp, &dest).map_err(|e| format!("换不上新版本：{e}"))?;
        }
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    res
}

/// 一次同步的结果：每个 agent、每个 skill 的状态，以及做了哪些改动（打印用）
pub struct Synced {
    pub states: Vec<SkillState>,
    pub changes: Vec<String>,
    /// 各目录里不是同步装的 skill（board host 在同步之后填上，「从机器导入」用）
    pub local: Vec<LocalSkill>,
    /// 这次同步的目录（导入时按显示的路径找回去）
    pub targets: Vec<Target>,
}

/// 按库同步这些目录。`fetch` 取一个 skill 的内容（board host、命令行都从服务端取）；`exe` 是放进 board-skills 的 board
pub fn sync(lib: &[SkillInfo], targets: &[Target], exe: Option<&Path>, mut fetch: impl FnMut(&str) -> Result<SkillPack>) -> Synced {
    let mut states = vec![];
    let mut changes = vec![];
    let mut packs: BTreeMap<String, std::result::Result<SkillPack, String>> = BTreeMap::new();
    for t in targets {
        let state = |skill: &str, state: &str, version: Option<String>, why: Option<String>| -> Vec<SkillState> {
            t.agents
                .iter()
                .map(|a| SkillState { skill: skill.into(), agent: a.clone(), env: t.env.clone(), dir: t.shown.clone(), state: state.into(), version: version.clone(), why: why.clone() })
                .collect()
        };
        if let Err(e) = std::fs::create_dir_all(&t.dir) {
            for s in lib {
                states.extend(state(&s.name, "failed", None, Some(format!("建不了目录 {}：{e}", t.shown))));
            }
            continue;
        }
        let mine = synced_in(&t.dir);
        // 库里删掉的：同步装的那份删掉
        for name in mine.keys().filter(|n| !lib.iter().any(|s| &s.name == *n)) {
            if std::fs::remove_dir_all(t.dir.join(name)).is_ok() {
                changes.push(format!("删掉 {}/{name}（库里已经删了）", t.shown));
            }
        }
        for s in lib {
            let dest = t.dir.join(&s.name);
            let link = std::fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink());
            // board-skills 例外：不是链接的旧拷贝（以前桌面端一键安装、解压 board-skills.zip 装的，没有标记）换成同步装的
            let adopt = s.name == BUILTIN && !link && dest.join("SKILL.md").is_file();
            if std::fs::symlink_metadata(&dest).is_ok() && mark_of(&dest).is_none() && !adopt {
                // 本机自己的这份和库里的内容一样（例如就是从这台机器导入的）：算已同步，不动它
                if scan(&dest).is_ok_and(|(_, _, v)| v == s.version) {
                    states.extend(state(&s.name, "synced", Some(s.version.clone()), None));
                    continue;
                }
                let what = if std::fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink()) { "链接" } else { "目录" };
                states.extend(state(&s.name, "local", None, Some(format!("本机已有同名的{what}（不是同步装的），没有覆盖"))));
                continue;
            }
            let had = mark_of(&dest);
            // board-skills 里的 board 要是 board host 自己的：版本一样但 bin 不是这个 board 时也重写
            let bin_ok = s.name != BUILTIN || exe.is_none() || t.wsl || bin_matches(&dest, exe);
            if had.as_deref() == Some(s.version.as_str()) && bin_ok {
                states.extend(state(&s.name, "synced", had, None));
                continue;
            }
            let pack = packs.entry(s.name.clone()).or_insert_with(|| fetch(&s.name).map_err(|e| e.message));
            let res = match pack {
                Ok(p) if p.info.version == s.version => install(&t.dir, p, if t.wsl { None } else { exe }),
                Ok(p) => Err(format!("取到的版本 {} 和库里的 {} 对不上（刚改过？下一次同步会跟上）", p.info.version, s.version)),
                Err(e) => Err(format!("取不到内容：{e}")),
            };
            match res {
                Ok(()) => {
                    changes.push(format!("{} {}/{} → {}", if had.is_some() { "更新" } else { "装上" }, t.shown, s.name, s.version));
                    states.extend(state(&s.name, "synced", Some(s.version.clone()), None));
                }
                Err(why) => states.extend(state(&s.name, if had.is_some() { "behind" } else { "failed" }, had, Some(why))),
            }
        }
    }
    Synced { states, changes, local: vec![], targets: vec![] }
}

/// 各目录里不是同步装的 skill：有 SKILL.md、没有标记文件（开发用的链接也算），board-skills 不算。
/// 同名同版本、在几个目录里的合成一项；读不了、太大、名称不对的写明原因
pub fn local_skills(targets: &[Target]) -> Vec<LocalSkill> {
    let mut out: Vec<LocalSkill> = vec![];
    for t in targets {
        let Ok(rd) = std::fs::read_dir(&t.dir) else { continue };
        let mut entries: Vec<std::fs::DirEntry> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let raw = e.file_name().to_string_lossy().into_owned();
            let p = e.path();
            if raw.starts_with('.') || !p.join("SKILL.md").is_file() || mark_of(&p).is_some() {
                continue;
            }
            let name = raw.to_lowercase();
            if name == BUILTIN {
                continue;
            }
            let mut l = LocalSkill { name: name.clone(), agents: t.agents.clone(), dirs: vec![t.shown.clone()], ..Default::default() };
            match skills::check_name(&name).and_then(|_| scan(&p)) {
                Ok((files, size, version)) => {
                    l.files = files;
                    l.size = size;
                    l.version = version;
                }
                Err(err) => l.why = Some(err.message),
            }
            match out.iter_mut().find(|x| x.name == l.name && x.version == l.version && x.why == l.why) {
                Some(x) => {
                    for a in &l.agents {
                        if !x.agents.contains(a) {
                            x.agents.push(a.clone());
                        }
                    }
                    if !x.dirs.contains(&t.shown) {
                        x.dirs.push(t.shown.clone());
                    }
                }
                None => out.push(l),
            }
        }
    }
    out
}

/// 读一个 skill 目录：文件数、大小、版本号（和库里的算法一样）
fn scan(dir: &Path) -> Result<(i64, i64, String)> {
    let mut files = vec![];
    let mut size = 0usize;
    walk(dir, dir, &mut files, &mut size)?;
    let raw: Vec<(String, bool, Vec<u8>)> = files.iter().map(|f| Ok((f.path.clone(), f.exec, skills::unb64(&f.data)?))).collect::<Result<_>>()?;
    Ok((raw.len() as i64, size as i64, skills::pack_version(&raw)))
}

/// 从机器导入一个 skill：在显示路径是 `shown` 的那个目录里找名称对上的 skill，读出来交给 `push`（传进库），返回 skill 名。
/// 本机的目录不动（之后同步时内容和库里一样就算已同步）
pub fn import_one(targets: &[Target], name: &str, shown: &str, push: impl FnOnce(&str, &[SkillFile]) -> Result<SkillInfo>) -> std::result::Result<String, String> {
    let t = targets.iter().find(|t| t.shown == shown).ok_or_else(|| format!("这台机器上已经没有 {shown} 这个目录"))?;
    let rd = std::fs::read_dir(&t.dir).map_err(|e| format!("读不了 {shown}：{e}"))?;
    let path = rd
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == name))
        .ok_or_else(|| format!("{shown} 里已经没有 {name}"))?;
    if mark_of(&path).is_some() {
        return Err(format!("{shown}/{name} 是从库里同步装的，不用导入"));
    }
    let (n, files) = read_dir(&path).map_err(|e| e.message)?;
    push(&n, &files).map_err(|e| e.message)?;
    Ok(n)
}

/// board-skills 目录里的 bin/<平台>/board 和 exe 是同一个文件内容（大小一样就算）
fn bin_matches(dest: &Path, exe: Option<&Path>) -> bool {
    let (Some(exe), Some(plat)) = (exe, crate::update::platform_of(std::env::consts::OS, std::env::consts::ARCH)) else { return true };
    let bin = dest.join("bin").join(plat).join(format!("board{}", std::env::consts::EXE_SUFFIX));
    match (std::fs::metadata(&bin), std::fs::metadata(exe)) {
        (Ok(a), Ok(b)) => a.len() == b.len(),
        _ => false,
    }
}

/// 读一个本机的 skill 目录（`board skills push`）：目录名是 skill 名，里面要有 SKILL.md；
/// 跳过以 . 开头的（.git、.DS_Store、标记文件）；board-skills 不能传
pub fn read_dir(dir: &Path) -> Result<(String, Vec<SkillFile>)> {
    let dir = std::fs::canonicalize(dir).map_err(|e| BoardError::usage(format!("找不到目录 {}：{e}", dir.display())))?;
    let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    skills::check_name(&name)?;
    if name == BUILTIN {
        return Err(BoardError::usage("board-skills 随服务端版本，不能上传"));
    }
    if !dir.join("SKILL.md").is_file() {
        return Err(BoardError::usage(format!("{} 里没有 SKILL.md：传的是 skill 目录本身", dir.display())));
    }
    let mut files = vec![];
    let mut size = 0usize;
    walk(&dir, &dir, &mut files, &mut size)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((name, files))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<SkillFile>, size: &mut usize) -> Result<()> {
    let rd = std::fs::read_dir(dir).map_err(|e| BoardError::usage(format!("读不了 {}：{e}", dir.display())))?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let p = e.path();
        let meta = std::fs::metadata(&p).map_err(|e| BoardError::usage(format!("读不了 {}：{e}", p.display())))?;
        if meta.is_dir() {
            walk(root, &p, out, size)?;
            continue;
        }
        *size += meta.len() as usize;
        if *size > MAX_BYTES {
            return Err(BoardError::usage(format!("{} 超过 {} MB：放进库里的 skill 合计不能超过 {} MB", root.display(), MAX_BYTES / 1024 / 1024, MAX_BYTES / 1024 / 1024)));
        }
        let data = std::fs::read(&p).map_err(|e| BoardError::usage(format!("读不了 {}：{e}", p.display())))?;
        #[cfg(unix)]
        let exec = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let exec = false;
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
        out.push(SkillFile { path: rel, exec, data: skills::b64(&data) });
    }
    Ok(())
}

/// 本机有 board host 在跑
fn host_running() -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let Ok(mut sys) = System::new() else { return false };
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always));
    sys.processes().values().any(|p| {
        let cmd: Vec<String> = p.cmd().iter().map(|s| s.to_string_lossy().into_owned()).collect();
        let is_board = cmd.first().is_some_and(|c| Path::new(c).file_stem().is_some_and(|s| s == "board"));
        is_board && cmd.iter().skip(1).find(|a| !a.starts_with('-')).is_some_and(|a| a == "host")
    })
}

fn short(v: &str) -> &str {
    &v[..v.len().min(7)]
}

fn words(state: &str) -> &'static str {
    match state {
        "synced" => "已同步",
        "behind" => "版本落后",
        "local" => "本机有同名，没有覆盖",
        "missing" => "没装",
        _ => "失败",
    }
}

/// `board skills <ls | push <目录>… | sync | rm <名称>>`
pub fn run(pos: &[String], json: bool, p: &Paint, rm: &Remote) -> Result<Output> {
    let sub = pos.get(1).map(String::as_str).unwrap_or("ls");
    match sub {
        "ls" => {
            let lib: Vec<SkillInfo> = rm.get("/api/skills", &[])?;
            let targets = local_targets();
            if json {
                let rows: Vec<serde_json::Value> = lib
                    .iter()
                    .map(|s| {
                        let here: Vec<serde_json::Value> = targets
                            .iter()
                            .map(|t| {
                                let (st, v) = local_state(&t.dir, s);
                                json!({ "dir": t.shown, "agents": t.agents, "state": st, "version": v })
                            })
                            .collect();
                        json!({ "skill": s, "local": here })
                    })
                    .collect();
                return Ok(Output::Json(json!({ "skills": rows })));
            }
            if lib.is_empty() {
                return Ok(Output::Text("库里还没有 skill：board skills push <skill 目录>\n".into()));
            }
            let mut s = String::new();
            for k in &lib {
                let when = if k.updated_at > 0 { chrono::DateTime::from_timestamp_millis(k.updated_at).map(|t| t.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string()).unwrap_or_default() } else { String::new() };
                s += &format!("{}  {}  {}  {}{}\n", p.bold(&k.name), p.cyan(short(&k.version)), when, k.source, if k.builtin { "（固定，随服务端版本）" } else { "" });
                for t in &targets {
                    let (st, v) = local_state(&t.dir, k);
                    let ver = v.map(|v| format!(" {}", short(&v))).unwrap_or_default();
                    let mark = if st == "synced" { p.green("✓") } else { p.yellow("!") };
                    s += &format!("  {mark} {}/{}  {}{ver}  {}\n", t.shown, k.name, words(st), p.dim(&t.agents.join("、")));
                }
            }
            if targets.is_empty() {
                s += &format!("{}\n", p.dim("（这台机器上没找到 agent 的 skill 目录）"));
            }
            Ok(Output::Text(s))
        }
        "push" => {
            let dirs = &pos[2..];
            if dirs.is_empty() {
                return Err(BoardError::usage("用法：board skills push <skill 目录>…"));
            }
            let source = board_core::hosts::host_name_from(&sysinfo::System::host_name().unwrap_or_default());
            let mut done = vec![];
            for d in dirs {
                let (name, files) = read_dir(Path::new(d))?;
                let info: SkillInfo = rm.post("/api/skills", json!({ "name": name, "files": files, "source": source }))?;
                done.push(info);
            }
            if json {
                return Ok(Output::Json(json!({ "ok": true, "skills": done })));
            }
            Ok(Output::Text(
                done.iter()
                    .map(|k| format!("{} {} → {}（{} 个文件，{} KB）：在线的机器 10 秒内同步\n", p.green("✓"), p.bold(&k.name), p.cyan(short(&k.version)), k.files, (k.size + 1023) / 1024))
                    .collect(),
            ))
        }
        "rm" => {
            let name = pos.get(2).ok_or_else(|| BoardError::usage("用法：board skills rm <名称>"))?;
            let _: serde_json::Value = rm.delete(&format!("/api/skills/{name}"))?;
            Ok(if json {
                Output::Json(json!({ "ok": true, "removed": name }))
            } else {
                Output::Text(format!("{} 已从库里删掉 {name}：各台机器上同步装的那份随后删掉\n", p.green("✓")))
            })
        }
        "sync" => {
            if host_running() {
                let msg = "本机有 board host 在跑：同步由它来做（库一变它马上跟上，界面「机器 → 共用配置」看结果）";
                return Ok(if json { Output::Json(json!({ "ok": true, "by_host": true, "message": msg })) } else { Output::Text(format!("{} {msg}\n", p.green("✓"))) });
            }
            let lib: Vec<SkillInfo> = rm.get("/api/skills", &[])?;
            let exe = std::env::current_exe().ok().filter(|e| !crate::update::bundled_with_desktop(e));
            let out = sync(&lib, &local_targets(), exe.as_deref(), |name| rm.get(&format!("/api/skills/{name}"), &[]));
            if json {
                return Ok(Output::Json(json!({ "ok": true, "states": out.states, "changes": out.changes })));
            }
            let mut s = String::new();
            for c in &out.changes {
                s += &format!("{} {c}\n", p.green("✓"));
            }
            for st in out.states.iter().filter(|x| x.state != "synced") {
                s += &format!("{} {}/{}（{}）：{}\n", p.yellow("!"), st.dir, st.skill, st.agent, st.why.as_deref().unwrap_or(words(&st.state)));
            }
            if s.is_empty() {
                s = format!("{} 都是最新的\n", p.green("✓"));
            }
            Ok(Output::Text(s))
        }
        other => Err(BoardError::usage(format!("board skills 没有「{other}」：用 ls、push、sync、rm"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(name: &str, files: &[(&str, &[u8])]) -> SkillPack {
        let list: Vec<(String, bool, Vec<u8>)> = files.iter().map(|(p, d)| (p.to_string(), false, d.to_vec())).collect();
        let version = skills::pack_version(&list);
        SkillPack {
            info: SkillInfo { name: name.into(), version, size: 0, files: list.len() as i64, updated_at: 1, by: "t".into(), source: "a".into(), builtin: name == BUILTIN },
            files: list.iter().map(|(p, x, d)| SkillFile { path: p.clone(), exec: *x, data: skills::b64(d) }).collect(),
        }
    }

    fn target(dir: &Path, agents: &[&str]) -> Target {
        Target { agents: agents.iter().map(|s| s.to_string()).collect(), env: "direct".into(), dir: dir.to_path_buf(), shown: dir.display().to_string(), wsl: false }
    }

    #[test]
    fn sync_installs_updates_keeps_local_and_removes() {
        let d = tempfile::tempdir().unwrap();
        let claude = d.path().join("claude");
        let agents = d.path().join("agents");
        let t = vec![target(&claude, &["claude"]), target(&agents, &["dsh", "pi"])];
        // agents 目录里本来有一个同名的、不是同步装的 notes：不覆盖
        std::fs::create_dir_all(agents.join("notes")).unwrap();
        std::fs::write(agents.join("notes/SKILL.md"), "mine").unwrap();
        let v1 = pack("notes", &[("SKILL.md", b"# v1"), ("ref/a.md", b"a")]);
        let mut fetched = 0;
        let out = sync(&[v1.info.clone()], &t, None, |_| {
            fetched += 1;
            Ok(v1.clone())
        });
        assert_eq!(fetched, 1, "同一个版本只取一次");
        assert_eq!(std::fs::read_to_string(claude.join("notes/ref/a.md")).unwrap(), "a");
        assert_eq!(mark_of(&claude.join("notes")).as_deref(), Some(v1.info.version.as_str()));
        assert_eq!(std::fs::read_to_string(agents.join("notes/SKILL.md")).unwrap(), "mine", "本机自己的不覆盖");
        let st = |out: &Synced, agent: &str| out.states.iter().find(|s| s.agent == agent).map(|s| s.state.clone()).unwrap();
        assert_eq!((st(&out, "claude"), st(&out, "dsh"), st(&out, "pi")), ("synced".into(), "local".into(), "local".into()));
        assert!(out.states.iter().find(|s| s.agent == "dsh").unwrap().why.as_deref().unwrap().contains("没有覆盖"));
        // 再同步一次：已经是最新的，不取内容
        let out = sync(&[v1.info.clone()], &t, None, |_| panic!("不该再取"));
        assert!(out.changes.is_empty() && st(&out, "claude") == "synced");
        // 新版本：换上，旧文件不留
        let v2 = pack("notes", &[("SKILL.md", b"# v2")]);
        let out = sync(&[v2.info.clone()], &t, None, |_| Ok(v2.clone()));
        assert_eq!(std::fs::read_to_string(claude.join("notes/SKILL.md")).unwrap(), "# v2");
        assert!(!claude.join("notes/ref").exists());
        assert_eq!(out.changes.len(), 1);
        // 取不到内容：原来那份留着，状态「版本落后」
        let v3 = pack("notes", &[("SKILL.md", b"# v3")]);
        let out = sync(&[v3.info.clone()], &t, None, |_| Err(BoardError::new(ErrorCode::Unreachable, "连不上")));
        assert_eq!(st(&out, "claude"), "behind");
        assert_eq!(std::fs::read_to_string(claude.join("notes/SKILL.md")).unwrap(), "# v2");
        // 库里删掉：同步装的删掉，本机自己的留着
        let out = sync(&[], &t, None, |_| unreachable!());
        assert!(!claude.join("notes").exists());
        assert!(agents.join("notes/SKILL.md").exists());
        assert_eq!(out.changes.len(), 1);
        // 没有留下临时目录
        assert_eq!(std::fs::read_dir(&claude).unwrap().count(), 0);
    }

    /// 「从机器导入」：列出不是同步装的 skill（同名同内容的合成一项，没有 SKILL.md、同步装的、board-skills 不算），
    /// 导入一个交给 push；导入之后本机这份和库里一样，同步时算已同步、不覆盖
    #[test]
    fn local_skills_listed_imported_and_then_counted_as_synced() {
        let d = tempfile::tempdir().unwrap();
        let claude = d.path().join("claude");
        let agents = d.path().join("agents");
        let t = vec![target(&claude, &["claude"]), target(&agents, &["dsh", "pi"])];
        for dir in [&claude, &agents] {
            std::fs::create_dir_all(dir.join("design/ref")).unwrap();
            std::fs::write(dir.join("design/SKILL.md"), "# design").unwrap();
            std::fs::write(dir.join("design/ref/a.md"), "a").unwrap();
        }
        std::fs::create_dir_all(claude.join("notes")).unwrap();
        std::fs::write(claude.join("notes/SKILL.md"), "# notes").unwrap();
        std::fs::create_dir_all(claude.join("empty")).unwrap();
        std::fs::create_dir_all(claude.join(BUILTIN)).unwrap();
        std::fs::write(claude.join(BUILTIN).join("SKILL.md"), "x").unwrap();
        // 同步装的不算
        let synced = pack("synced", &[("SKILL.md", b"# s")]);
        sync(&[synced.info.clone()], &t[..1], None, |_| Ok(synced.clone()));
        let list = local_skills(&t);
        assert_eq!(list.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["design", "notes"]);
        let design = &list[0];
        assert_eq!((design.files, design.agents.clone(), design.dirs.len()), (2, vec!["claude".to_string(), "dsh".into(), "pi".into()], 2));
        // 版本和库里的算法一样
        let same = pack("design", &[("SKILL.md", b"# design"), ("ref/a.md", b"a")]);
        assert_eq!(design.version, same.info.version);
        // 导入：交给 push 的是目录名和文件
        let mut pushed = vec![];
        let n = import_one(&t, "design", &claude.display().to_string(), |name, files| {
            pushed = files.iter().map(|f| f.path.clone()).collect();
            Ok(SkillInfo { name: name.into(), ..same.info.clone() })
        })
        .unwrap();
        assert_eq!((n.as_str(), pushed), ("design", vec!["SKILL.md".to_string(), "ref/a.md".into()]));
        assert!(import_one(&t, "nope", &claude.display().to_string(), |_, _| unreachable!()).unwrap_err().contains("没有"));
        assert!(import_one(&t, "synced", &claude.display().to_string(), |_, _| unreachable!()).unwrap_err().contains("同步装的"));
        // 库里有了 design（就是这一份）：同步时本机自己的算已同步，不覆盖、不取内容
        let out = sync(&[same.info.clone()], &t, None, |_| panic!("内容一样，不该取"));
        assert!(out.states.iter().all(|s| s.state == "synced"), "{:?}", out.states);
        assert!(mark_of(&claude.join("design")).is_none(), "本机自己的目录不动");
    }

    /// 以前一键安装、解压 zip 装的 board-skills（没有标记、不是链接）换成同步装的；别的同名目录照样不动
    #[test]
    fn old_board_skills_copy_is_adopted() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("skills");
        std::fs::create_dir_all(dir.join(BUILTIN)).unwrap();
        std::fs::write(dir.join(BUILTIN).join("SKILL.md"), "old").unwrap();
        let b = pack(BUILTIN, &[("SKILL.md", b"# new")]);
        let out = sync(&[b.info.clone()], &[target(&dir, &["claude"])], None, |_| Ok(b.clone()));
        assert_eq!(out.states[0].state, "synced");
        assert_eq!(std::fs::read_to_string(dir.join(BUILTIN).join("SKILL.md")).unwrap(), "# new");
    }

    #[cfg(unix)]
    #[test]
    fn links_are_left_alone_and_board_skills_gets_own_board() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("skills");
        std::fs::create_dir_all(&dir).unwrap();
        let src = d.path().join("dev-board-skills");
        std::fs::create_dir_all(&src).unwrap();
        std::os::unix::fs::symlink(&src, dir.join(BUILTIN)).unwrap();
        let b = pack(BUILTIN, &[("SKILL.md", b"# board"), ("scripts/board", b"#!/bin/sh")]);
        let exe = d.path().join("myboard");
        std::fs::write(&exe, b"binary").unwrap();
        let out = sync(&[b.info.clone()], &[target(&dir, &["claude"])], Some(&exe), |_| Ok(b.clone()));
        assert_eq!(out.states[0].state, "local");
        assert!(out.states[0].why.as_deref().unwrap().contains("链接"));
        // 去掉链接：装上，bin/<平台>/board 是 board host 自己的
        std::fs::remove_file(dir.join(BUILTIN)).unwrap();
        let out = sync(&[b.info.clone()], &[target(&dir, &["claude"])], Some(&exe), |_| Ok(b.clone()));
        assert_eq!(out.states[0].state, "synced");
        if let Some(plat) = crate::update::platform_of(std::env::consts::OS, std::env::consts::ARCH) {
            assert_eq!(std::fs::read(dir.join(BUILTIN).join("bin").join(plat).join("board")).unwrap(), b"binary");
        }
        // board 换了（board update 是改名替换，大小不同）：版本一样也重写 bin
        std::fs::write(d.path().join("new"), b"binary-v2").unwrap();
        std::fs::rename(d.path().join("new"), &exe).unwrap();
        let out = sync(&[b.info.clone()], &[target(&dir, &["claude"])], Some(&exe), |_| Ok(b.clone()));
        assert_eq!(out.changes.len(), 1);
    }

    #[test]
    fn read_dir_checks_and_skips_hidden() {
        let d = tempfile::tempdir().unwrap();
        let s = d.path().join("My-Notes");
        std::fs::create_dir_all(s.join(".git")).unwrap();
        std::fs::write(s.join(".git/x"), "x").unwrap();
        assert!(read_dir(&s).unwrap_err().message.contains("SKILL.md"));
        std::fs::write(s.join("SKILL.md"), "# n").unwrap();
        std::fs::create_dir_all(s.join("ref")).unwrap();
        std::fs::write(s.join("ref/a.md"), "a").unwrap();
        let (name, files) = read_dir(&s).unwrap();
        assert_eq!(name, "my-notes");
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["SKILL.md", "ref/a.md"]);
        let big = d.path().join("big");
        std::fs::create_dir_all(&big).unwrap();
        std::fs::write(big.join("SKILL.md"), "x").unwrap();
        std::fs::write(big.join("blob"), vec![0u8; MAX_BYTES + 1]).unwrap();
        assert!(read_dir(&big).unwrap_err().message.contains("超过 5 MB"));
    }
}
