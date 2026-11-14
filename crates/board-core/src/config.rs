//! 命令行和桌面端共用的配置文件 `~/.board/config.toml`（Windows：`%USERPROFILE%\.board\config.toml`，PRD §9）。
//!
//! - `url`（服务端地址）和 `token`（访问令牌）：`board login` / `logout`、桌面端「Agent 接入」的一键写入。
//! - `[host]`（机器名、白板与目录）和 `[agents.<标识>]`（agent 命令模板）：`board host` 读，桌面端「设置 → 本机」写 `[host]`。
//! - 环境变量 `BOARD_CONFIG` 指定另一个配置文件代替 `~/.board/config.toml`（board host 启动 agent 时用它传连接）。
//! - 写只有一个入口 [`update`]：加锁 → 读 → 改 → 写临时文件 → 改名。CLI 和桌面端同时写（例如保存本机设置的同时 `board login`）
//!   不会互相覆盖；写到一半不会留下残缺的配置；保留文件里的其他配置项和注释。
//! - 现有文件不是合法 TOML 时：写入前先备份成 `config.toml.bad-<时间戳>`，再用空配置继续写（只读入口仍然报错，不偷偷改文件）。
//! - 权限：文件 0600；`~/.board` 目录 0700。`BOARD_CONFIG` 指到别的目录时不改那个目录的权限。
//!
//! 需要 `config` feature（服务端不带 toml_edit）。

use crate::{BoardError, Result};
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

/// 配置文件里的项
pub const URL: &str = "url";
pub const TOKEN: &str = "token";

/// 用户目录：Windows 上是 USERPROFILE，其他系统是 HOME（测试把它指向临时目录）
pub fn home() -> Option<PathBuf> {
    let keys: [&str; 2] = if cfg!(windows) { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    keys.iter().find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
}

/// `~/.board`：配置文件、board host 的日志都放在这里
pub fn board_dir() -> Result<PathBuf> {
    home().map(|h| h.join(".board")).ok_or_else(|| BoardError::usage("找不到用户目录（HOME / USERPROFILE），没法读写配置文件"))
}

fn env_config() -> Option<String> {
    std::env::var("BOARD_CONFIG").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// 配置文件：`BOARD_CONFIG`，否则 `~/.board/config.toml`
pub fn path() -> Result<PathBuf> {
    match env_config() {
        Some(p) => Ok(PathBuf::from(p)),
        None => Ok(board_dir()?.join("config.toml")),
    }
}

/// 配置文件的位置（给人看的写法）
pub fn shown() -> String {
    if let Some(p) = env_config() {
        return p;
    }
    if cfg!(windows) {
        r"%USERPROFILE%\.board\config.toml".into()
    } else {
        "~/.board/config.toml".into()
    }
}

/// `~` 开头的路径换成用户目录；其他原样
pub fn expand(p: &str) -> PathBuf {
    match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => match home() {
            Some(h) => h.join(rest.trim_start_matches(['/', '\\'])),
            None => PathBuf::from(p),
        },
        _ => PathBuf::from(p),
    }
}

/// 读进来的配置文件；文件不存在时是空的
#[derive(Debug, Clone)]
pub struct Config {
    path: PathBuf,
    pub doc: DocumentMut,
    /// 原文件解析失败、已经备份后按空配置继续（写回时会生成合法的新文件）
    recovered: bool,
}

impl Config {
    /// 读 [`path`]
    pub fn load() -> Result<Config> {
        Config::load_from(&path()?)
    }

    pub fn load_from(path: &Path) -> Result<Config> {
        let text = read_text(path)?;
        Ok(Config { path: path.to_path_buf(), doc: parse_doc(path, &text)?, recovered: false })
    }

    /// 读配置文件给写入用：现有文件不是合法 TOML 时，先把原文备份成
    /// `config.toml.bad-<时间戳>`，再按空配置继续写（CLI login、桌面端一键写入、保存机器名都走这里）。
    /// 只读的命令仍然用 [`Config::load`]，不会偷偷改文件。
    pub fn load_for_update(path: &Path) -> Result<Config> {
        let text = read_text(path)?;
        match text.parse::<DocumentMut>() {
            Ok(doc) => Ok(Config { path: path.to_path_buf(), doc, recovered: false }),
            Err(e) => {
                backup_bad(path).map_err(|be| {
                    BoardError::usage(format!(
                        "{} 不是合法的 TOML：{}；备份原文件失败：{}",
                        path.display(),
                        e.to_string().trim(),
                        be
                    ))
                })?;
                Ok(Config { path: path.to_path_buf(), doc: DocumentMut::new(), recovered: true })
            }
        }
    }

    /// 一项字符串配置；没有或为空时是 None
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        match self.doc.get(key) {
            None => Ok(None),
            Some(item) => match item.as_str() {
                Some(s) => Ok(Some(s.trim().to_string()).filter(|s| !s.is_empty())),
                None => Err(BoardError::usage(format!("{} 里的 {key} 应该是字符串", self.path.display()))),
            },
        }
    }

    pub fn set(&mut self, key: &str, v: &str) {
        self.doc[key] = toml_edit::value(v);
    }

    /// 写一张表（例如派发时带给 agent 的 `[agent_env]`），这个键原来的内容整个换掉
    pub fn set_table(&mut self, key: &str, t: toml_edit::Table) {
        self.doc.insert(key, toml_edit::Item::Table(t));
    }

    /// 删掉一项，返回原来有没有
    pub fn remove(&mut self, key: &str) -> bool {
        self.doc.remove(key).is_some()
    }
}

/// 读文件内容；不存在时当作空配置
fn read_text(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(BoardError::usage(format!("读不了 {}：{e}", path.display()))),
    }
}

fn parse_doc(path: &Path, text: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>()
        .map_err(|e| BoardError::usage(format!("{} 不是合法的 TOML：{}", path.display(), e.to_string().trim())))
}

/// 把解析不了的配置备份成同目录的 `<文件名>.bad-<时间戳>`（同一秒重复时加序号），返回备份路径
fn backup_bad(path: &Path) -> std::io::Result<PathBuf> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "config.toml".into());
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut backup = dir.join(format!("{file}.bad-{stamp}"));
    let mut seq = 2;
    while backup.exists() {
        backup = dir.join(format!("{file}.bad-{stamp}-{seq}"));
        seq += 1;
    }
    std::fs::copy(path, &backup)?;
    Ok(backup)
}

/* ------------------------------ [host] 与 [agents.*] ------------------------------ */

/// `[host]` 与 `[agents.*]`：
///
/// ```toml
/// [host]
/// name = "macbook"
/// workspace = "~/repo"          # 每块白板在这个目录里是 <项目>/<白板>
/// dirs = { "default/main" = "~/repo/board" }   # 旧写法：逐块配；启动时换成对应的 workspace
/// log_days = 30
///
/// [agents.claude]
/// cmd = ["claude", "-p", "{prompt}", "--dangerously-skip-permissions", "--output-format", "stream-json", "--verbose"]
/// model = "opus-5.5"
/// ```
#[derive(Debug, Clone, Default)]
pub struct HostConf {
    pub name: Option<String>,
    /// 工作空间：每台设备一个目录，每块白板的目录是 `<工作空间>/<项目>/<白板>`（`~` 不展开）
    pub workspace: Option<String>,
    /// 旧的白板与目录（`[host] dirs`、`--dir`），按文件里的顺序。启动时换成工作空间，兼容保留
    pub dirs: Vec<(String, String)>,
    /// 本机派发日志保留几天（`~/.board/host/logs` 里结束超过这么多天的删掉）；不写时用默认值
    pub log_days: Option<i64>,
    pub agents: Vec<AgentConf>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentConf {
    pub id: String,
    /// 名称（界面上显示）；不写时用标识
    pub name: Option<String>,
    /// 模型名：派发的 agent 名字（`<agent 标识>-<机器名>/<模型名>`）的第二段；不写时用内置模板的默认值，
    /// 内置模板也没有时用 agent 标识
    pub model: Option<String>,
    pub cmd: Vec<String>,
    /// 机器设置里选了模型时追加的参数（`{model}` 换成模型），例如 `["--model", "{model}"]`；不写时不能选模型
    /// （内置模板有自己的）
    pub model_args: Option<Vec<String>>,
    /// 选了思考强度时追加的参数（`{effort}` 换成档位）；不写时不能选强度（内置模板有自己的）
    pub effort_args: Option<Vec<String>>,
    /// 支持的思考强度档位（写了 effort_args 才有用）；不写时是 low、medium、high
    pub efforts: Option<Vec<String>>,
}

impl Config {
    pub fn host(&self) -> Result<HostConf> {
        let bad = |what: &str| BoardError::usage(format!("{} 里的 {what} 格式不对", self.path.display()));
        let mut c = HostConf::default();
        if let Some(h) = self.doc.get("host") {
            let t = h.as_table_like().ok_or_else(|| bad("[host]"))?;
            if let Some(n) = t.get("name") {
                c.name = Some(n.as_str().ok_or_else(|| bad("host.name"))?.trim().to_string()).filter(|s| !s.is_empty());
            }
            if let Some(w) = t.get("workspace") {
                c.workspace = Some(w.as_str().ok_or_else(|| bad("host.workspace（要写成一个目录）"))?.trim().to_string()).filter(|s| !s.is_empty());
            }
            if let Some(d) = t.get("dirs") {
                let d = d.as_table_like().ok_or_else(|| bad("host.dirs"))?;
                for (k, v) in d.iter() {
                    c.dirs.push((k.to_string(), v.as_str().ok_or_else(|| bad("host.dirs"))?.to_string()));
                }
            }
            if let Some(n) = t.get("log_days") {
                let n = n.as_integer().ok_or_else(|| bad("host.log_days（要写成整数）"))?;
                if n < 1 {
                    return Err(bad("host.log_days（至少 1 天）"));
                }
                c.log_days = Some(n);
            }
        }
        if let Some(a) = self.doc.get("agents") {
            let t = a.as_table_like().ok_or_else(|| bad("[agents]"))?;
            for (id, v) in t.iter() {
                let at = v.as_table_like().ok_or_else(|| bad(&format!("[agents.{id}]")))?;
                let cmd = at
                    .get("cmd")
                    .and_then(|x| x.as_array())
                    .ok_or_else(|| bad(&format!("agents.{id}.cmd（要写成字符串数组）")))?
                    .iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| bad(&format!("agents.{id}.cmd（要写成字符串数组）")))?;
                let name = at.get("name").and_then(|x| x.as_str()).map(str::to_string);
                let model = at.get("model").and_then(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                let list = |key: &str| -> Result<Option<Vec<String>>> {
                    let Some(v) = at.get(key) else { return Ok(None) };
                    let what = format!("agents.{id}.{key}（要写成字符串数组）");
                    let arr = v.as_array().ok_or_else(|| bad(&what))?;
                    arr.iter().map(|x| x.as_str().map(str::to_string)).collect::<Option<Vec<_>>>().ok_or_else(|| bad(&what)).map(Some)
                };
                let (model_args, effort_args, efforts) = (list("model_args")?, list("effort_args")?, list("efforts")?);
                c.agents.push(AgentConf { id: id.to_string(), name, model, cmd, model_args, effort_args, efforts });
            }
        }
        Ok(c)
    }

    /// 写 `[host]` 的机器名和工作空间，保留 `[host]` 里的其他项；机器名为 None 时去掉 name（用本机主机名）。
    /// 旧的白板与目录（`[host] dirs`）原样留着（`--dir` 还在用它），不再改。不校验，调用方先校验
    pub fn set_host(&mut self, name: Option<&str>, workspace: Option<&str>) -> Result<()> {
        if self.doc.get("host").and_then(|h| h.as_table_like()).is_none() {
            self.doc["host"] = toml_edit::table();
        }
        let host = self.doc["host"]
            .as_table_like_mut()
            .ok_or_else(|| BoardError::usage(format!("{} 里的 [host] 格式不对", self.path.display())))?;
        match name {
            Some(n) => {
                host.insert("name", toml_edit::value(n));
            }
            None => {
                host.remove("name");
            }
        }
        match workspace.map(str::trim).filter(|w| !w.is_empty()) {
            Some(w) => {
                host.insert("workspace", toml_edit::value(w));
            }
            None => {
                host.remove("workspace");
            }
        }
        Ok(())
    }
}

/* ------------------------------ 写 ------------------------------ */

/// 改配置文件（[`path`]）：加锁后读最新的内容交给 `f` 修改，写回；`f` 返回 Err 时不写。
/// `f` 返回 `Ok((r, false))` 表示没有改动，不写文件
pub fn update<T>(f: impl FnOnce(&mut Config) -> Result<(T, bool)>) -> Result<T> {
    let path = path()?;
    // 只有 ~/.board 是我们自己的目录，设 0700；BOARD_CONFIG 指到别处时不碰那个目录的权限
    let own = board_dir().is_ok_and(|d| path.parent() == Some(d.as_path()));
    update_at(&path, own, f)
}

/// [`update`] 的实现：own_dir 为真时把所在目录设成 0700
pub fn update_at<T>(path: &Path, own_dir: bool, f: impl FnOnce(&mut Config) -> Result<(T, bool)>) -> Result<T> {
    let fail = |e: std::io::Error| BoardError::usage(format!("写入 {} 失败：{e}", path.display()));
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir).map_err(fail)?;
    #[cfg(unix)]
    {
        if own_dir {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(fail)?;
        }
    }
    #[cfg(not(unix))]
    let _ = own_dir;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "config.toml".into());
    // 锁：同一个配置文件旁边的 .<文件名>.lock（进程、线程之间都互斥；进程退出时系统自动释放）
    let lock = open_private(&dir.join(format!(".{file_name}.lock")), false).map_err(fail)?;
    lock.lock().map_err(fail)?;
    let mut conf = Config::load_for_update(path)?;
    let (r, changed) = f(&mut conf)?;
    // 原文件坏掉、已备份时，即使这次逻辑上没有改动也写一份合法的新文件
    if changed || conf.recovered {
        write_atomic(path, &dir, &file_name, conf.doc.to_string().as_bytes()).map_err(fail)?;
    }
    drop(lock);
    Ok(r)
}

/// 新写一个配置文件（覆盖原来的，不保留旧内容）：只有自己能读（0600），原子写入，不加锁、不改目录权限。
/// 给只用一次的配置文件（board host 每次派发给 agent 的连接）
pub fn write_new(path: &Path, f: impl FnOnce(&mut Config)) -> Result<()> {
    let fail = |e: std::io::Error| BoardError::usage(format!("无法写 {}：{e}", path.display()));
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir).map_err(fail)?;
    let mut c = Config { path: path.to_path_buf(), doc: DocumentMut::new(), recovered: false };
    f(&mut c);
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "config.toml".into());
    write_atomic(path, &dir, &file_name, c.doc.to_string().as_bytes()).map_err(fail)
}

/// 打开（或新建）一个只有自己能读写的文件（0600）
fn open_private(p: &Path, truncate: bool) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(truncate);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(p)
}

/// 先写临时文件（名字带进程号和序号，同时写的不会撞上；创建即 0600）、sync，再改名。失败时删掉临时文件（里面可能有令牌）
fn write_atomic(path: &Path, dir: &Path, file_name: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = dir.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let r = (|| {
        let mut f = open_private(&tmp, true)?;
        #[cfg(unix)]
        {
            // umask 之外再保险一次：创建时的 mode 受 umask 影响只会更严，这里固定成 0600
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn creates_file_and_keeps_comments_and_other_items() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(".board").join("config.toml");
        update_at(&f, true, |c| {
            c.set(URL, "https://a");
            Ok(((), true))
        })
        .unwrap();
        assert_eq!(read(&f), "url = \"https://a\"\n");

        std::fs::write(&f, "# 我的配置\neditor = \"vim\"\nurl = \"http://old\"\n\n[agents.x]\ncmd = [\"x\"] # 注释\n").unwrap();
        update_at(&f, true, |c| {
            c.set(URL, "https://b");
            c.set(TOKEN, "bd_1");
            Ok(((), true))
        })
        .unwrap();
        assert_eq!(read(&f), "# 我的配置\neditor = \"vim\"\nurl = \"https://b\"\ntoken = \"bd_1\"\n\n[agents.x]\ncmd = [\"x\"] # 注释\n");

        // 没有改动：不写（文件内容不变），f 的错误原样返回、不写
        update_at(&f, true, |_| Ok(((), false))).unwrap();
        let e = update_at::<()>(&f, true, |c| {
            c.set(URL, "https://c");
            Err(BoardError::usage("不对"))
        })
        .unwrap_err();
        assert_eq!(e.message, "不对");
        assert!(read(&f).contains("https://b"));
        // 没有留下临时文件
        let left: Vec<_> = std::fs::read_dir(f.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn host_section_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.toml");
        std::fs::write(&f, "url = \"https://a\"\n\n[host]\nmax = 2\n").unwrap();
        update_at(&f, false, |c| c.set_host(Some("macbook"), Some("~/repo")).map(|_| ((), true))).unwrap();
        assert_eq!(read(&f), "url = \"https://a\"\n\n[host]\nmax = 2\nname = \"macbook\"\nworkspace = \"~/repo\"\n");
        let h = Config::load_from(&f).unwrap().host().unwrap();
        assert_eq!(h.name.as_deref(), Some("macbook"));
        assert_eq!(h.workspace.as_deref(), Some("~/repo"));
        update_at(&f, false, |c| c.set_host(None, Some("~/repo")).map(|_| ((), true))).unwrap();
        assert!(!read(&f).contains("name ="));
        // 不配工作空间：去掉这一项
        update_at(&f, false, |c| c.set_host(None, None).map(|_| ((), true))).unwrap();
        assert!(!read(&f).contains("workspace"), "{}", read(&f));
        assert_eq!(Config::load_from(&f).unwrap().host().unwrap().workspace, None);
        // 旧的 [host] dirs 照样读得出来，改机器名、工作空间时不碰它
        let o = d.path().join("old.toml");
        std::fs::write(&o, "[host]\ndirs = { \"default/main\" = \"~/repo/board\" }\n").unwrap();
        let h = Config::load_from(&o).unwrap().host().unwrap();
        assert_eq!(h.dirs, vec![("default/main".to_string(), "~/repo/board".to_string())]);
        assert_eq!(h.workspace, None);
        update_at(&o, false, |c| c.set_host(Some("win"), Some("D:/repo")).map(|_| ((), true))).unwrap();
        assert!(read(&o).contains("dirs = { \"default/main\" = \"~/repo/board\" }"), "{}", read(&o));
        // 日志保留天数：不写时用默认值（None），写了要是不小于 1 的整数；改机器名、工作空间时不碰它
        let g = d.path().join("log.toml");
        std::fs::write(&g, "[host]\nlog_days = 7\n").unwrap();
        let h = Config::load_from(&g).unwrap().host().unwrap();
        assert_eq!(h.log_days, Some(7));
        update_at(&g, false, |c| c.set_host(Some("mbp"), Some("~/repo")).map(|_| ((), true))).unwrap();
        assert!(read(&g).contains("log_days = 7"), "{}", read(&g));
        assert_eq!(Config::load_from(&g).unwrap().host().unwrap().log_days, Some(7));
        assert_eq!(Config::load_from(&f).unwrap().host().unwrap().log_days, None);
        for bad in ["[host]\nlog_days = 0\n", "[host]\nlog_days = -3\n"] {
            std::fs::write(&g, bad).unwrap();
            assert!(Config::load_from(&g).unwrap().host().is_err(), "{bad}");
        }
        std::fs::write(&g, "[host]\nlog_days = \"30\"\n").unwrap();
        assert!(Config::load_from(&g).unwrap().host().is_err(), "要写成整数");
    }

    #[test]
    fn concurrent_writers_do_not_overwrite_each_other() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.toml");
        std::fs::write(&f, "# 注释\nurl = \"https://a\"\n").unwrap();
        for round in 0..20 {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let (fa, fb) = (f.clone(), f.clone());
            let (ba, bb) = (barrier.clone(), barrier.clone());
            let a = std::thread::spawn(move || {
                ba.wait();
                update_at(&fa, false, |c| {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    c.set(TOKEN, &format!("bd_{round}"));
                    Ok(((), true))
                })
                .unwrap()
            });
            let b = std::thread::spawn(move || {
                bb.wait();
                update_at(&fb, false, |c| {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    c.set_host(Some(&format!("m{round}")), None).map(|_| ((), true))
                })
                .unwrap()
            });
            a.join().unwrap();
            b.join().unwrap();
            let c = Config::load_from(&f).unwrap();
            assert_eq!(c.get(TOKEN).unwrap().as_deref(), Some(format!("bd_{round}").as_str()), "第 {round} 轮");
            assert_eq!(c.host().unwrap().name.as_deref(), Some(format!("m{round}").as_str()), "第 {round} 轮");
            assert!(read(&f).starts_with("# 注释\nurl = \"https://a\"\n"), "{}", read(&f));
        }
    }

    #[cfg(unix)]
    #[test]
    fn permissions() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let d = tempfile::tempdir().unwrap();
        // ~/.board：目录 0700，文件 0600（原来是 0644 的也改过来）
        let own = d.path().join(".board");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o755)).unwrap();
        let f = own.join("config.toml");
        std::fs::write(&f, "").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        update_at(&f, true, |c| {
            c.set(TOKEN, "bd_1");
            Ok(((), true))
        })
        .unwrap();
        assert_eq!(mode(&f), 0o600);
        assert_eq!(mode(&own), 0o700);
        // BOARD_CONFIG 指到别的目录：文件 0600，那个目录的权限不动
        let other = d.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o755)).unwrap();
        let g = other.join("agent.toml");
        update_at(&g, false, |c| {
            c.set(TOKEN, "bd_2");
            Ok(((), true))
        })
        .unwrap();
        assert_eq!(mode(&g), 0o600);
        assert_eq!(mode(&other), 0o755);
    }

    #[test]
    fn checks_and_expand() {
        assert!(crate::valid_board("default/main"));
        assert!(!crate::valid_board("default"));
        assert!(!crate::valid_board("default/main/x"));
        assert!(!crate::valid_board("Default/main"));
        assert_eq!(expand("/abs/x"), PathBuf::from("/abs/x"));
        assert_eq!(expand("~x"), PathBuf::from("~x"));
        if let Some(h) = home() {
            assert_eq!(expand("~/repo"), h.join("repo"));
            assert_eq!(expand("~"), h);
        }
    }

    // 现场配置里的 `\;` 不是 TOML 合法转义：写入入口要先把原文件备份，再用空配置继续写
    #[test]
    fn invalid_config_is_backed_up_before_write() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".board");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.toml");
        let bad = "# 旧配置\n[agents.claude]\ncmd = [\"bash\", \"-c\", \"echo agent \\; exec sleep 300\", \"-\"]\n";
        std::fs::write(&f, bad).unwrap();
        assert!(Config::load_from(&f).is_err(), "只读入口仍然把非法配置当错误");

        update_at(&f, true, |c| {
            c.set(URL, "https://a");
            Ok(((), true))
        })
        .unwrap();
        assert!(read(&f).contains("url = \"https://a\""), "{}", read(&f));
        assert_eq!(Config::load_from(&f).unwrap().get(URL).unwrap().as_deref(), Some("https://a"));
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("config.toml.bad-"))
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        assert_eq!(std::fs::read_to_string(dir.join(&backups[0])).unwrap(), bad);
    }

    // Windows 路径带反斜杠：写进 [host].workspace 再读回来仍是原值
    #[test]
    fn host_workspace_with_windows_paths_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.toml");
        let ws = r"C:\Users\buglan\repo";
        update_at(&f, false, |c| c.set_host(Some("win"), Some(ws)).map(|_| ((), true))).unwrap();
        let text = read(&f);
        assert!(Config::load_from(&f).is_ok(), "写出的配置必须是合法 TOML：{text}");
        let h = Config::load_from(&f).unwrap().host().unwrap();
        assert_eq!(h.name.as_deref(), Some("win"));
        assert_eq!(h.workspace.as_deref(), Some(ws));
    }

    // [host] workspace 要写成字符串；不是字符串时报错，不偷偷换成别的
    #[test]
    fn host_workspace_must_be_a_string() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.toml");
        std::fs::write(&f, "[host]\nworkspace = 3\n").unwrap();
        let e = Config::load_from(&f).unwrap().host().unwrap_err();
        assert!(e.message.contains("host.workspace"), "{}", e.message);
    }

    // 反斜杠、引号、换行交给 toml_edit 序列化：写进去再读回来必须一模一样
    #[test]
    fn strings_with_backslashes_quotes_and_newlines_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.toml");
        let value = r#"echo agent \; exec sleep 300
"C:\Users\buglan\""#;
        update_at(&f, false, |c| {
            c.set("cmdline", value);
            Ok(((), true))
        })
        .unwrap();
        let text = read(&f);
        assert!(Config::load_from(&f).is_ok(), "写出的配置必须是合法 TOML：{text}");
        assert_eq!(Config::load_from(&f).unwrap().get("cmdline").unwrap().as_deref(), Some(value));
    }
}
