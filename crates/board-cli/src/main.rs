//! `board` —— agent 用的命令行（PRD §9）。
//!
//! 身份：领卡用 `board start <ID> --agent <标识>`；不领卡的会话先 `board check --agent <标识>`。
//! 名字由 Board 按机器设置生成，记在本机，之后不用再带 `--as`；
//! 也可以自己起名字：`board check --as <名字>`，之后每条命令都带 `--as <名字>`（或环境变量 `BOARD_AS`）；
//! 白板：`--board <项目>/<白板>` 或环境变量 `BOARD`。
//! 所有命令支持 `--json`。成功退出码 0；被拒绝退出码 1，并给出错误码和原因。
//! 数据：通过服务端读写。服务端地址和访问令牌由 `board login` 写进 `~/.board/config.toml`，
//! 环境变量 `BOARD_URL`、`BOARD_TOKEN` 优先；没有配置时直接报错。见 backend.rs、config.rs。
//! 任务下发：`board host` 在这台机器上常驻接派发（host/），`board dispatch` 把卡片派给一台机器。

mod acp;
mod args;
mod backend;
mod base;
mod card;
mod code;
mod config;
mod doctor;
mod envs;
mod events;
mod gitsync;
mod gitcmd;
mod heartbeat;
mod host;
mod land;
mod logfmt;
mod lsdir;
mod mcpsync;
mod output;
mod providersync;
mod redact;
mod res;
mod selftest;
mod service;
mod session;
mod skills;
mod text;
mod touched;
mod update;
mod versions;

use args::{text_arg, Args};
use backend::{env, needs_labels, want_of, Backend, BoardData, Remote, Want};
use board_core::*;
use output::{agent_json, attachment_json, clock, emit, human_size, iso, rule_of, Fmt, Output, Paint};
use serde_json::{json, Value};
use session::{borrowed, check, explicit_name, heartbeat_cmd, start_session};
use std::io::IsTerminal;
use std::time::Duration;
use text::{pad, truncate, width};

/// 全部命令：（命令、参数、说明）。认不认识一个命令、board help 的内容都只看这张表；
/// 新加命令时在这里加一行，再在 run() 里处理（测试 every_command_is_dispatched 会检查）
const COMMANDS: [(&str, &str, &str); 34] = [
        ("login", "[--url <地址>] [--token <令牌>]", "校验令牌后把服务端地址和令牌写进 ~/.board/config.toml；不带 --token 时从标准输入读一行"),
        ("logout", "", "从 ~/.board/config.toml 删掉令牌，保留服务端地址"),
        ("update", "[--latest]", "把单独安装的 board 原地升级到服务端的版本（--latest：最新发版）：下载发版附件、校验 SHA-256 后替换；桌面端自带的 board 跟着桌面端升级，不能用它"),
        ("doctor", "[--as <名字>] [--board <白板>]", "环境自检：board 路径、PATH、服务端地址和令牌（以及来自哪里）、服务端版本、名字是否在线；有问题退出码非 0"),
        (
            "check",
            "[--agent <标识>] [--as <名字>]",
            "会话开始时占用名字。--agent <标识>（claude、codex、dsh …）：名字由 Board 按这台机器（[host] name 或令牌认出的机器）、这个 agent 的设置生成，例如 win/opus-5.5-high，已有在线会话时加 -1、-2，查不到机器时用 <标识>/<标识>（同样加序号）；占到的名字记在本机，之后不写 --as 的命令都用它。--as <名字>（或 BOARD_AS）照旧：自己起名字，被在线 agent 占用时 NAME_TAKEN",
        ),
        ("heartbeat", "start | stop [--force] | final <ID>", "start：check 之后启动后台心跳（每 10 秒一次 board beat，跟随 agent 进程），立即返回；stop：停掉后台心跳再发 final，名字立刻离线；名字是本机另一个还在运行的 agent 进程 check 的时，start、stop 都拒绝（NAME_TAKEN），--force 清理本机残留时用；final <ID>：结束这张卡片这一次的计时"),
        ("beat", "[<ID>] [--final] [--force]", "不带 ID：agent 级心跳，维持名字在线和持有的卡片；--final 名字立刻离线（带着 check 时拿到的会话标识，名字属于别的会话时 NAME_TAKEN）。带 ID：这张卡片的心跳；--final 结束这一次计时。一般用 board heartbeat"),
        ("boards", "", "列出所有白板"),
        ("flow", "", "列出白板的工作流：状态的顺序、标识、名称、规则，以及每个状态能去哪里"),
        ("ls", "[--column <状态>] [--search <文字>] [--label <标签>]", "列出卡片"),
        ("search", "\"<文字>\" [--limit <条数>]", "跨所有白板搜卡片（不用 --board）：文字搜标题、描述、评论，也认 #ID、项目/白板#ID、label:标签；输出白板路径、ID、状态、标题"),
        ("show", "<ID>", "显示卡片的全部字段和动态"),
        ("new", "\"<标题>\" [--body \"<描述>\" | --body-file <文件>] [--image <图片>]… [--file <文件>]… [--top] [--inbox] [--label <标签>]… [--from <ID>]", "在首个可领取状态新建卡片；--inbox 记到「未整理」最下面；--label 可以写多次，带上项目里已有的标签（仅人可打的也能带，只在人要求时带）；多行描述用 --body-file（- 读标准输入）；--image、--file 可以写多次，按顺序附在描述后面（图片直接显示，文件是下载链接）；--from 记下拆自同一块白板上的哪张卡（回收站里的也行，不存在时 NOT_FOUND）"),
        ("edit", "<ID> [--title \"<标题>\"] [--body \"<描述>\" | --body-file <文件>]", "修改标题或描述"),
        ("move", "<ID> <状态> [--top]", "把卡片移到指定状态（标识或名称）；只能沿 board flow 里的连线移动"),
        ("claim", "[<ID>]", "领取；不带 ID 时按工作流顺序领可领取状态最上面的卡片"),
        (
            "start",
            "[<ID>] [--agent <标识>] [--out <目录>]",
            "开始做一张卡，一条命令做完会话开始和领取：这个会话还没有名字时先占用名字（--agent <标识> 由 Board 生成，或 --as <名字>）并启动后台心跳，已经有了就接着用；领取（不带 ID 时领可领取状态最上面的；卡片已经由你持有时接着做）；下载附件；返回卡片的全部字段和动态，以及交审、做不下去时卡片会去哪个状态、项目的仓库和集成分支。board host 启动的会话里不占名字、不启动心跳",
        ),
        (
            "submit",
            "<ID> [<文件>…] --comment \"<交审评论>\" | --comment-file <文件> [--no-code]",
            "交审，一条命令做完：上传附件、发交审评论、结束这一次的计时、把卡片移到交审状态（服务端在一次写入里做完，不会出现 final 之后没移走被释放）。项目设了代码仓库时，评论里要有「提交：」「集成分支：」「检查：」「碰到的别的卡：」四行，缺了拒绝；这张卡没改代码时加 --no-code",
        ),
        (
            "giveup",
            "<ID> [<文件>…] --comment \"<原因>\" | --comment-file <文件>",
            "做不下去、交给人，一条命令做完：上传附件、评论原因、结束这一次的计时、把卡片移到错误节点（没有时移到需要人处理的状态；两个都没有时只评论和结束计时，50 秒后卡片回到领取前的状态）",
        ),
        ("next", "", "只在 board host 启动的会话里用：上一张交审或交给人之后领这个会话排着的下一张；排着的做完了、不再接着做时返回 SESSION_DONE"),
        (
            "base",
            "<ID> [--branch <分支>] [--dir <仓库>]",
            "开工，领到卡片之后、改代码之前在项目的仓库里（主仓库或任意 worktree）用，一条命令做完：卡片要由你持有；git fetch；会话的 worktree（当前目录就是锁着你名字的 worktree 就用它，没有就在 <仓库目录名>.wt/ 里建，规则和 board host 一样）；worktree 里有没推的改动时不切换、退出码 1、列出来；查这张卡有没有提交进过集成分支；卡片评论里有「未完成的改动：wip/…」、远端还有时切过去并 rebase 到最新集成分支（冲突时停下、退出码 1），否则切到最新的集成分支。输出 worktree、基点和下一步（next）",
        ),
        (
            "land",
            "<ID> --message-file <文件> --checked \"<检查和结果>\" [--touched-ok] [--branch <分支>] [--dir <仓库>] | <ID> --wip",
            "推送，在自己的 worktree 里 rebase、跑完检查之后用，一条命令按顺序做完：确认工作区干净、HEAD 在最新的集成分支之上（不在就停，先 rebase、重新跑检查），压成一个提交（--message-file 的内容，最后一行补上「卡片：」），board touched（碰到别的卡就停，确认有意后加 --touched-ok 重来），board hold，推集成分支（被拒不 --force，压好的提交留着），删掉接着做的 wip 分支，输出交审评论最后四行。集成分支：项目设置里的，没设用远端默认分支，--branch 指定。--wip：做不下去时把没做完的改动提交、推到 wip/<白板标识>/<ID>，输出「未完成的改动：」一行",
        ),
        ("hold", "<ID> [--wait <秒>]", "推集成分支前确认领取仍有效：这张卡仍由你持有、在锁定状态，否则 NOT_HOLDER（不要推集成分支，改推 wip 分支）；连不上服务端时每 30 秒重试，最多等 --wait 秒（默认 300），还连不上就 UNREACHABLE"),
        ("comment", "<ID> \"<内容>\" | <ID> --comment-file <文件>", "追加评论；多行内容用 --comment-file（- 读标准输入）"),
        ("label", "<ID> +<标签> -<标签> …", "加标签（+ 或不带前缀）、去标签（-）；仅人可打的标签不能给已有卡片加、去（建卡时可以带上）"),
        ("labels", "", "列出项目的标签、类型（agent 可打 / 仅人可打）、是否优先，以及每个标签被多少张卡片使用"),
        ("attach", "<ID> <文件>… [--comment \"<说明>\" | --comment-file <文件>]", "上传附件（图片、日志等），连同说明作为一条评论"),
        ("files", "<ID> [--out <目录>]", "把卡片的附件下载到本地（默认临时目录），列出路径"),
        ("host", "[run] [--name <机器名>] [--workspace <工作空间>] [--dir <项目>=<路径>]… [--install | --uninstall] | rename <新机器名>", "run（默认）：在这台机器上常驻接任务，每个项目的目录是 <工作空间>/<项目>（不存在时自动建好；白板设了仓库地址时自动克隆），派来的卡片在那里启动本机 agent，Ctrl+C 退出（机器名、工作空间也可写在 ~/.board/config.toml 的 [host]）；--dir 只给单独一块白板指定目录（覆盖工作空间派生的目录）；--install：按这些参数装成开机自启的服务（macOS launchd、Linux systemd 用户服务、Windows 登录时的计划任务），异常退出 10 秒后自动重启，输出写进 ~/.board/host/host.log；--uninstall：删掉服务；rename：机器改名，服务端上的记录原地改名，设置和历史派发记录保留（桌面端「设置 → 本机」保存机器名时调用）"),
        ("skills", "ls | push <目录>… | sync | rm <名称>", "skills 共用库（不用 --as、--board）：ls 列出库里的 skill 和本机各 agent 目录里的状态；push 把本机的 skill 目录（目录名就是名称，里面要有 SKILL.md，合计不超过 5 MB）传进库里，同名的换成新版本，在线的机器自动同步；sync 按库更新本机（有 board host 在跑时由它来做）；rm 从库里删掉，各台机器上同步装的那份随后删掉。board-skills 随服务端版本，不能传、不能删"),
        ("versions", "sync [<tag>…] [--project <项目>] [--dir <仓库>] [--dry-run]", "在代码仓库里从 git tag 和提交历史反推每个版本做了哪些卡片（不用 --as、--board）：v 开头的 tag 是版本，上一个 tag 之后的提交属于这个 tag，提交说明里的「卡片：<项目>/<白板>#<ID>」把提交连到卡片；这些卡片带上和 tag 同名的版本标签（没有就新建），这一版的提交记到服务端，界面「里程碑」里按版本看。--project 不写时取提交里出现最多的项目；只同步某几个 tag 时写在 sync 后面；--dry-run 只列出来不写入；反复运行结果一样"),
        (
            "touched",
            "[<提交>] [--base <提交>] [--upstream <分支>] [--card <项目/白板#ID>] [--dir <仓库>]",
            "推送前检查（只用 git，不用 --as、--board，不连服务端）：要推的提交（默认 HEAD）删掉了别的卡最近加的行、或加回了别的卡最近删的行时，按卡列出提交、文件和例子，退出码 1。「最近」= 开工时的基点（--base；不写时从本 worktree 的 reflog 认开工时所在的提交，认不出就往前 20 个提交）之后集成分支（--upstream，默认 origin/HEAD）上的提交；这次的卡（--card，不写时取提交说明里的「卡片：」）自己的提交不算。最后一行「碰到的别的卡：…」写进交审评论",
        ),
        ("dispatch", "<ID> --host <机器名> | --any [--profile <角色>] [--move] | <ID> --cancel", "按角色把卡片派给一台机器（--profile 写角色的名字；不带时用按规则会交给的角色，规则都没命中时要带上 --profile；--agent <标识> 是旧写法：用机器上这个 agent 自己的配置）；--any 不指定机器，由服务端在能接的机器里挑空位最多的；卡片在「未整理」时是派给 agent 拆解：不领取、不移动它，拆出的卡记回「未整理」；--move 把还在排队的派发改派到 --host 的机器或 --any；--cancel 取消排队或停止正在跑的派发"),
        ("help", "", "显示这段说明"),
];

fn main() {
    // git 的 askpass 会不带参数启动这个程序；立即失败，不能进入正常命令解析或打开任何界面
    if gitcmd::is_askpass() {
        std::process::exit(1);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    // board host 启动 agent 用的 ACP 那一层（acp.rs）：`--` 之后是 agent 的命令，不按 board 的参数解析
    if args.first().map(String::as_str) == Some("acp") {
        std::process::exit(acp::main(&args[1..]));
    }
    // 在 board host 启动的会话里（BOARD_AGENT_ENV，skill 的包装脚本设的）：有的 agent 不把它该有的 BOARD_* 传给命令，
    // 从这次派发的配置文件 [agent_env] 补上，名字、白板、连接都用得到（见 config::from_agent_env）
    if env("BOARD_AGENT_ENV").is_some() {
        config::from_agent_env();
    }
    let json = args.iter().any(|a| a == "--json");
    let p = Paint { on: !json && std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() };
    match run(&args, &p) {
        Ok(mut out) => {
            // 用的名字是本机另一个 agent 进程 check 的：照常执行，但提醒一句（--json 里加 warning 字段）
            if let Some(w) = borrowed(&args) {
                match &mut out {
                    Output::Json(Value::Object(m)) => {
                        m.insert("warning".into(), json!(w));
                    }
                    _ => {
                        let pe = Paint { on: std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none() };
                        eprintln!("{} {w}", pe.yellow("!"));
                    }
                }
            }
            let code = emit(out);
            if code != 0 {
                std::process::exit(code);
            }
        }
        Err(e) => {
            // board start 已经占了名字、启动了后台心跳，之后才失败（领取失败之类）：报错里写上名字
            let session = session::SESSION.get();
            if json {
                let mut err = json!({ "code": e.code, "message": e.message });
                if let Some(d) = e.detail {
                    err["detail"] = d;
                }
                if let Some((name, started)) = session {
                    if err["detail"].is_null() {
                        err["detail"] = json!({});
                    }
                    if let Some(d) = err["detail"].as_object_mut() {
                        d.insert("session".into(), json!({ "name": name, "started": started }));
                    }
                }
                println!("{}", serde_json::to_string_pretty(&json!({ "error": err })).unwrap_or_default());
            } else {
                let pe = Paint { on: std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none() };
                eprintln!("{} {}  {}", pe.red("✗"), pe.red_bold(e.code.as_str()), e.message);
                if let Some((name, true)) = session {
                    eprintln!("{} 名字 {name} 已占用，后台心跳在跑；不再领卡时 board heartbeat stop", pe.yellow("!"));
                }
            }
            std::process::exit(1);
        }
    }
}

/* ------------------------------ board hold ------------------------------ */

/// `board hold` 连不上服务端时最多等多久（秒）
const HOLD_WAIT_SECS: u64 = 300;

/// 两次确认之间等多久：`BOARD_HOLD_INTERVAL`（秒，只给测试用），默认 30 秒
fn hold_every() -> Duration {
    let secs = std::env::var("BOARD_HOLD_INTERVAL").ok().and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| *s > 0.0);
    secs.map(Duration::from_secs_f64).unwrap_or(Duration::from_secs(30))
}

/// 取卡片：连不上服务端（UNREACHABLE）时每 `every` 重试一次，总共最多等 `wait`；等不到就把最后一次的错误换成写明怎么办的 UNREACHABLE。
/// 其他错误直接返回。`note` 在每次重试前调用，参数是还能等多久
fn hold_retry<T>(mut get: impl FnMut() -> Result<T>, wait: Duration, every: Duration, mut note: impl FnMut(Duration)) -> Result<T> {
    let begun = std::time::Instant::now();
    loop {
        match get() {
            Ok(x) => return Ok(x),
            Err(e) if e.code == ErrorCode::Unreachable => {
                let used = begun.elapsed();
                if used + every > wait {
                    return Err(BoardError::new(
                        ErrorCode::Unreachable,
                        format!(
                            "等了 {} 秒还是连不上服务端（{}）：不要推集成分支，把改动推到 wip 分支，等能连上后在卡片上补一条评论",
                            used.as_secs(),
                            e.message
                        ),
                    ));
                }
                note(wait - used);
                std::thread::sleep(every);
            }
            Err(e) => return Err(e),
        }
    }
}

/// 领取仍有效：卡片由这个名字持有，并且在锁定状态。否则 NOT_HOLDER，写明现在在哪个状态（status 是状态名称）、谁持有
fn hold_check(id: i64, status: &str, locked: bool, holder: Option<&str>, name: &str) -> Result<()> {
    if holder == Some(name) && locked {
        return Ok(());
    }
    let who = match holder {
        Some(h) if h != name => format!("由 {h} 持有"),
        Some(_) => "还由你持有，但已经不在锁定状态".to_string(),
        None => "没人持有".to_string(),
    };
    Err(BoardError::new(
        ErrorCode::NotHolder,
        format!(
            "领取已经失效：#{id} 现在在「{status}」，{who}。不要推集成分支：把改动推到 wip 分支，在卡片上评论说明，然后结束这张卡（不 submit、不 giveup、不移动）"
        ),
    )
    .with_detail(json!({ "status": status, "holder": holder })))
}

/* ------------------------------ 执行 ------------------------------ */

fn run(tokens: &[String], p: &Paint) -> Result<Output> {
    let a = Args::parse(tokens)?;
    let json = a.flag("--json");
    // 版本号（发版附件里的安装包、skill 自带的 board 都能这样核对）
    if a.flag("--version") || a.flag("-V") {
        let v = env!("CARGO_PKG_VERSION");
        return Ok(if json { Output::Json(json!({ "version": v })) } else { Output::Text(format!("board {v}\n")) });
    }
    let sub = a.pos.first().map(|s| s.as_str());
    let sub = match sub {
        None => return Ok(Output::Text(help(p))),
        Some(_) if a.flag("--help") || a.flag("-h") => return Ok(Output::Text(help(p))),
        Some("help") => return Ok(Output::Text(help(p))),
        Some(s) if !COMMANDS.iter().any(|c| c.0 == s) => {
            return Err(BoardError::usage(format!("未知命令「{s}」，运行 board help 查看用法")));
        }
        Some(s) => s,
    };

    // 自检只检查环境本身：名字和白板没提供时跳过
    if sub == "doctor" {
        return Ok(doctor::run(json, p, a.value("--as"), a.value("--board")));
    }
    // 登录、退出：只读写配置文件，不需要名字和白板
    if sub == "login" {
        return login(&a, json, p);
    }
    if sub == "logout" {
        return logout(json, p);
    }
    // 升级：只读服务端地址（不需要令牌），不需要名字和白板
    if sub == "update" {
        return update::run(a.flag("--latest"), json, p);
    }

    // 推送前检查：只用 git，不需要连接、名字和白板
    if sub == "touched" {
        let o = touched::Opts { commit: a.pos.get(1).map(String::as_str), base: a.value("--base"), upstream: a.value("--upstream"), card: a.value("--card"), dir: a.value("--dir") };
        return touched::run(&o, json, p);
    }

    // 任务下发：board host 在前台一直运行；board host rename 不需要名字（带了也只当参数）；board dispatch 不需要名字（带了就记为发起人）
    if sub == "host" {
        // 只认 board host、board host run、board host rename <新机器名>。以前 board host ls 会直接常驻，
        // 不小心敲错也会启动一个 host，还能和服务里的那个抢名字
        let usage = "用法：board host [run] [--name <机器名>] [--workspace <工作空间>] [--dir <项目>=<路径>]… [--install | --uninstall]，或 board host rename <新机器名>";
        match a.pos.get(1).map(String::as_str) {
            None | Some("run") => {
                if let Some(extra) = a.pos.get(2) {
                    return Err(BoardError::usage(format!("多余的位置参数「{extra}」，{usage}")));
                }
            }
            Some("rename") => {
                if let Some(extra) = a.pos.get(3) {
                    return Err(BoardError::usage(format!("多余的位置参数「{extra}」，{usage}")));
                }
            }
            Some(s) => return Err(BoardError::usage(format!("未知子命令「{s}」，{usage}"))),
        }
        if a.pos.get(1).map(String::as_str) == Some("rename") {
            let to = a
                .pos
                .get(2)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| BoardError::usage("用法：board host rename <新机器名>"))?;
            return host::rename(to, json, p);
        }
        // 装成开机自启的服务、卸载；--service 是服务里运行的守护进程（见 service.rs）
        if a.flag("--install") {
            return service::install(a.value("--name"), a.value("--workspace"), &a.all("--dir"), json, p);
        }
        if a.flag("--uninstall") {
            return service::uninstall(json, p);
        }
        if a.flag("--service") {
            let mut args: Vec<String> = vec![];
            if let Some(n) = a.value("--name") {
                args.extend(["--name".to_string(), n.to_string()]);
            }
            if let Some(w) = a.value("--workspace") {
                args.extend(["--workspace".to_string(), w.to_string()]);
            }
            for d in a.all("--dir") {
                args.extend(["--dir".to_string(), d]);
            }
            service::supervise(args)?;
            return Ok(Output::Text(String::new()));
        }
        host::run(host::Opts { name: a.value("--name").map(str::to_string), workspace: a.value("--workspace").map(str::to_string), dirs: a.all("--dir") }, p)?;
        return Ok(Output::Text(String::new()));
    }
    if sub == "dispatch" {
        return dispatch(&a, json, p);
    }

    // 连接（PRD §9）：环境变量优先，其次 ~/.board/config.toml；都没有时 NOT_LOGGED_IN
    let mut be = Backend::open()?;

    // skills 共用库：只要连接，不需要名字和白板
    if sub == "skills" {
        return skills::run(&a.pos, json, p, be.remote());
    }
    // 里程碑：从 git 反推版本，只要连接，不需要名字和白板
    if sub == "versions" {
        return versions::run(&a.pos, a.value("--project"), a.value("--dir"), a.flag("--dry-run"), json, p, be.remote());
    }

    // 名字（PRD §3）：check 占用名字；不带 ID 的 beat 是 agent 级心跳。都不需要白板
    if sub == "check" {
        return check(&a, &mut be, json, p);
    }

    // 身份（PRD §3）：--as / BOARD_AS，没有时用本机 check 时记下的名字（board check --agent <标识> 生成的）
    // board start：这个会话还没有名字时在这里占用名字、启动后台心跳
    let started = if sub == "start" {
        let s = start_session(&a, &mut be)?;
        let _ = session::SESSION.set(s.clone());
        Some(s)
    } else {
        None
    };
    let name = match &started {
        Some((name, _)) => Some(name.clone()),
        None => explicit_name(&a).or_else(heartbeat::current_name),
    };
    let Some(name) = name else {
        return Err(BoardError::code(ErrorCode::NoIdentity));
    };
    let actor = Actor::agent(name.clone());
    if sub == "heartbeat" {
        return heartbeat_cmd(&a, &name, json, p);
    }
    if sub == "beat" && a.pos.len() == 1 {
        let done = a.flag("--final");
        let session = heartbeat::session_for(&name, a.flag("--force"))?;
        let info = be.agent_beat(&name, done, session.as_deref())?;
        if json {
            return Ok(Output::Json(agent_json(&info)));
        }
        let cards: Vec<String> = info.cards.iter().map(|h| format!("{}#{}", h.board, h.card.id)).collect();
        return Ok(Output::Text(if done {
            format!(
                "{} {} 已离线{}\n",
                p.green("✓"),
                p.bold(&name),
                if cards.is_empty() {
                    String::new()
                } else {
                    p.dim(&format!("  持有的 {} 收不到心跳，{} 秒内释放", cards.join("、"), NAME_GAP_MS / 1000))
                }
            )
        } else {
            let mut s = format!("{} {} 心跳\n", p.green("✓"), p.bold(&name));
            for h in &info.cards {
                let run = match h.card.run_started_at {
                    Some(t) => format!("本次 {}", clock(info.beat_at - t)),
                    None => "已 final：50 秒内沿连线移走".to_string(),
                };
                s += &format!("  {} {}  {}\n", p.cyan(&format!("{}#{}", h.board, h.card.id)), h.card.title, p.dim(&run));
            }
            s
        }));
    }

    if sub == "boards" {
        let ws = be.workspace()?;
        let boards: Vec<(&ProjectInfo, &BoardInfo)> =
            ws.projects.iter().flat_map(|pr| pr.boards.iter().map(move |b| (pr, b))).collect();
        if json {
            return Ok(Output::Json(Value::Array(
                boards
                    .iter()
                    .map(|(pr, b)| json!({ "board": b.path, "project": pr.name, "name": b.name, "counts": b.counts, "flow": b.statuses.iter().map(|s| &s.key).collect::<Vec<_>>() }))
                    .collect(),
            )));
        }
        let w = boards.iter().map(|(_, b)| width(&b.path)).max().unwrap_or(0) + 2;
        let nw = boards.iter().map(|(pr, b)| width(&format!("{} / {}", pr.name, b.name))).max().unwrap_or(0) + 2;
        let mut s = String::new();
        for (pr, b) in boards {
            let n = |k: &str| b.counts.get(k).copied().unwrap_or(0);
            let mut parts = vec![format!("{INBOX_NAME} {}", n(INBOX))];
            parts.extend(b.statuses.iter().map(|st| format!("{} {}", st.name, n(&st.key))));
            s += &format!("{}{}{}\n", p.cyan(&pad(&b.path, w)), pad(&format!("{} / {}", pr.name, b.name), nw), p.dim(&parts.join(" · ")));
        }
        return Ok(Output::Text(s));
    }

    // 跨白板搜索（PRD §9）：不需要白板，一次在所有白板（含「未整理」）里找
    if sub == "search" {
        let q = a.pos[1..].join(" ");
        if q.trim().is_empty() {
            return Err(BoardError::usage("用法：board search \"<文字>\"（也认 #ID、项目/白板#ID、label:标签）"));
        }
        let limit = match a.value("--limit") {
            Some(v) => v
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .ok_or_else(|| BoardError::usage("board search 的 --limit 需要是正整数"))?,
            None => SEARCH_LIMIT,
        };
        let hits = be.search(&q, limit)?;
        if json {
            return Ok(Output::Json(Value::Array(
                hits.iter()
                    .map(|h| {
                        json!({
                            "board": h.board,
                            "id": h.id,
                            "column": h.column,
                            "status": h.status,
                            "title": h.title,
                            "labels": h.labels,
                            "updated_at": iso(Some(h.updated_at)),
                            "matched": h.matched,
                        })
                    })
                    .collect(),
            )));
        }
        if hits.is_empty() {
            return Ok(Output::Text(format!("{}\n", p.dim(&format!("（没有找到和「{q}」相关的卡片）")))));
        }
        let bw = hits.iter().map(|h| width(&h.board)).max().unwrap_or(0) + 2;
        let sw = hits.iter().map(|h| width(&h.status)).max().unwrap_or(0) + 2;
        let tw = hits.iter().map(|h| width(&h.title)).max().unwrap_or(0).min(56) + 2;
        let lw = hits.iter().map(|h| width(&h.labels.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" "))).max().unwrap_or(0) + 2;
        let mut s = String::new();
        for h in &hits {
            let tags = h.labels.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
            s += &format!(
                "{}{}{}{}\n",
                p.cyan(&pad(&format!("{}#{}", h.board, h.id), bw + 6)),
                pad(&h.status, sw),
                pad(&truncate(&h.title, 56), tw),
                p.magenta(&pad(&tags, lw)),
            );
        }
        s += &p.dim(&format!("{} 条", hits.len()));
        s.push('\n');
        return Ok(Output::Text(s));
    }

    // 推集成分支前确认领取仍有效（PRD §9.1「执行」、skill「推送」）：连不上服务端时重试，所以放在读整块白板之前
    if sub == "hold" {
        let id = a.id(sub)?;
        let raw = a.value("--board").map(|s| s.to_string()).or_else(|| env("BOARD")).filter(|s| !s.trim().is_empty());
        let Some(raw) = raw else {
            return Err(BoardError::code(ErrorCode::NoBoard));
        };
        let wait = match a.value("--wait") {
            Some(v) => v.trim().parse::<u64>().map_err(|_| BoardError::usage("board hold 的 --wait 需要是秒数（整数）"))?,
            None => HOLD_WAIT_SECS,
        };
        let every = hold_every();
        let (path, flow, card) = hold_retry(
            || {
                // 只取这一张卡：工作流在返回里（旧服务端再取一次快照）
                let d = be.card(&raw, id)?;
                let v = be.view(&raw, &d, false)?;
                Ok((v.path, v.flow, d.detail.card))
            },
            Duration::from_secs(wait),
            every,
            |left| eprintln!("{} 连不上服务端，{} 秒后再确认 #{id} 的领取（还能等 {} 秒）", p.yellow("!"), every.as_secs_f64().round(), left.as_secs()),
        )?;
        hold_check(id, flow.name(&card.column), flow.is_lock(&card.column), card.holder.as_deref(), &name)?;
        return Ok(if json {
            Output::Json(json!({ "ok": true, "board": path, "id": id, "column": card.column, "holder": name }))
        } else {
            Output::Text(format!("{} #{id} 仍由你持有，在「{}」：可以推集成分支\n", p.green("✓"), flow.name(&card.column)))
        });
    }

    // 推送（skill「代码 → 推送」）：只取工作流和这一张卡，所以也放在读整块白板之前
    if sub == "land" {
        return land::run(&a, &name, &mut be, json, p);
    }
    // 开工（skill「代码 → 开工」）：只取工作流和这一张卡
    if sub == "base" {
        return base::run(&a, &name, &mut be, json, p);
    }

    // 白板（PRD §3）
    let raw = a.value("--board").map(|s| s.to_string()).or_else(|| env("BOARD"));
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Err(BoardError::code(ErrorCode::NoBoard));
    };
    // 卡片的网址：服务端地址 + /<项目>/<白板>/<ID>（和服务端的单页应用回退、界面的路由一致，PRD §10.1）
    let origin = be.remote().url().trim_end_matches('/').to_string();
    // 读写一张卡的命令：不先取白板，工作流这些在返回里，一个请求做完（card.rs）
    match sub {
        "show" | "edit" | "move" | "claim" | "next" | "comment" | "beat" | "files" | "start" | "giveup" | "submit" => {
            let c = card::Ctx { a: &a, sub, json, p, raw: &raw, name: &name, actor: &actor, origin: &origin };
            return card::run(&c, &mut be, &started);
        }
        _ => {}
    }
    // 每条命令都先取一次白板：只取这条命令用得到的（backend.rs 的 want_of、needs_labels），
    // 终点状态攒了几百张卡的白板上，全量快照加整个工作区要好几秒
    let BoardData { path, flow, repo, branch, cards: mut all, counts, omitted, labels, now } =
        be.board(&raw, want_of(sub, a.value("--column").is_some()), needs_labels(sub, json))?;
    let label_name = |id: i64| labels.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default();
    let find_label = |name: &str| -> Result<i64> {
        let q = name.trim().to_lowercase();
        labels.iter().find(|l| l.name.to_lowercase() == q).map(|l| l.id).ok_or_else(|| {
            BoardError::usage(format!(
                "项目里没有标签「{name}」，可选：{}",
                if labels.is_empty() {
                    "（这个项目还没有标签，在界面上新建）".to_string()
                } else {
                    labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join("、")
                }
            ))
        })
    };
    let mut exec = |cmd: Cmd| -> Result<CardDetail> { be.exec(&path, &actor, cmd) };
    let label_priority = |id: i64| labels.iter().any(|l| l.id == id && l.priority);
    let url_of = |id: i64| format!("{origin}/{path}/{id}");
    let fmt = Fmt { p, label_name: &label_name, label_priority: &label_priority, path: &path, url_of: &url_of, now, flow: &flow };

    let out = match sub {
        "flow" => {
            let count = |k: &str| counts.get(k).copied().unwrap_or(0);
            if json {
                Output::Json(json!({
                    "repo": repo,
                    "branch": branch,
                    "statuses": flow.0
                        .iter()
                        .map(|st| json!({ "key": st.key, "name": st.name, "claim": st.claim, "lock": st.lock, "final": st.fin, "error": st.err, "review": st.review, "to": st.to, "cards": count(&st.key) }))
                        .collect::<Vec<_>>(),
                }))
            } else {
                let kw = flow.0.iter().map(|st| width(&st.key)).max().unwrap_or(0).max(5) + 2;
                let nw = flow.0.iter().map(|st| width(&st.name)).max().unwrap_or(0) + 2;
                let mut s = String::new();
                let rw = flow.0.iter().map(|st| width(&rule_of(st))).max().unwrap_or(0) + 2;
                for (i, st) in flow.0.iter().enumerate() {
                    let rules = rule_of(st);
                    let to: Vec<&str> = st.to.iter().map(|k| flow.name(k)).collect();
                    s += &format!(
                        "{} {}{}{}{}{}\n",
                        p.dim(&format!("{}.", i + 1)),
                        fmt.col(&st.key, &pad(&st.key, kw)),
                        pad(&st.name, nw),
                        p.dim(&pad(&format!("{} 张", count(&st.key)), 8)),
                        p.yellow(&pad(&rules, rw)),
                        if to.is_empty() { p.dim("→ （没有连线）") } else { format!("→ {}", to.join("、")) }
                    );
                }
                s += &format!("{} {}{}{}\n", p.dim("·"), p.magenta(&pad(INBOX, kw)), pad(INBOX_NAME, nw), p.dim(&format!("{} 张  不属于工作流", count(INBOX))));
                let blank = p.dim("（没设）");
                let hint = p.dim("  # 项目设置里改；集成分支留空时按仓库的 AGENTS.md，没写用远端默认分支");
                s += &format!(
                    "{} {}  {} {}{}\n",
                    p.dim("仓库："),
                    if repo.is_empty() { blank.clone() } else { repo.clone() },
                    p.dim("集成分支："),
                    if branch.is_empty() { blank } else { branch.clone() },
                    hint
                );
                Output::Text(s)
            }
        }
        "ls" => {
            let column = match a.value("--column") {
                Some(c) => Some(
                    flow.resolve(c)
                        .ok_or_else(|| BoardError::usage(format!("未知的状态「{}」，可选：{}", c.trim(), flow.choices())))?,
                ),
                None => None,
            };
            let label = a.value("--label").map(find_label).transpose()?;
            let q = a.value("--search").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
            // 写了 --column 时先取的是「终点状态只带最近几张」的快照：要列的状态有被省略的，再取一次全量
            if column.as_ref().is_some_and(|col| omitted.get(col).copied().unwrap_or(0) > 0) {
                all = be.board(&raw, Want::Full, false)?.cards;
            }
            let cards: Vec<Card> = all
                .into_iter()
                .filter(|c| column.as_ref().is_none_or(|col| &c.column == col))
                .filter(|c| label.is_none_or(|l| c.labels.contains(&l)))
                .filter(|c| q.as_ref().is_none_or(|q| format!("{}\n{}", c.title, c.body).to_lowercase().contains(q)))
                .collect();
            if json {
                Output::Json(Value::Array(cards.iter().map(|c| fmt.card(c)).collect()))
            } else {
                Output::Text(fmt.list(&cards))
            }
        }
        "new" => {
            let title = a.pos[1..].join(" ");
            let inbox = a.flag("--inbox");
            let top = a.flag("--top") && !inbox;
            // --label 可以写多次，按名称匹配（不区分大小写），同一个只算一次；不存在的在新建前拒绝。
            // 仅人可打的标签建卡时可以带上（PRD §6），什么时候带由 board-skills 约束：只在人要求时带
            let mut with: Vec<i64> = vec![];
            for n in a.all("--label") {
                let id = find_label(&n)?;
                if !with.contains(&id) {
                    with.push(id);
                }
            }
            let mut body = text_arg(&a, "--body", "--body-file")?;
            // --image / --file：先在本地全部检查一遍（不存在、类型不支持、超过 10 MB 都在上传前拒绝，不建卡），
            // 再逐个上传成还没归属的附件，按参数顺序把它们的 Markdown 接在描述后面；建卡时服务端按引用把附件归到这张卡片
            let files = new_files(&a, title.trim().is_empty())?;
            if !files.is_empty() {
                let mut refs = vec![];
                for (name, bytes) in &files {
                    refs.push(be.upload_unbound(&path, &actor, name, bytes)?.markdown);
                }
                let text = body.take().unwrap_or_default();
                let text = text.trim_end();
                body = Some(format!("{text}{}{}", if text.is_empty() { "" } else { "\n\n" }, refs.join("\n\n")));
            }
            let from = match a.value("--from") {
                Some(raw) => Some(
                    raw.trim()
                        .trim_start_matches('#')
                        .parse::<i64>()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or_else(|| BoardError::usage(format!("--from：无效的 ID「{raw}」")))?,
                ),
                None => None,
            };
            let d = be.exec(&path, &actor, Cmd::New {
                title,
                body,
                column: inbox.then(|| INBOX.to_string()),
                top,
                labels: with,
                from,
            })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                let ls: Vec<String> = d.card.labels.iter().map(|l| p.magenta(&format!("#{}", label_name(*l)))).collect();
                Output::Text(format!(
                    "{} 已新建 {}  {}{}{}{}\n{}{}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{}", d.card.id)),
                    d.card.title,
                    if ls.is_empty() { String::new() } else { format!("  {}", ls.join(" ")) },
                    p.dim(&format!("  → {}最{}", if inbox { INBOX_NAME } else { flow.home().name.as_str() }, if top { "上面" } else { "下面" })),
                    d.card.split_from.map(|f| p.dim(&format!("  拆自 #{f}"))).unwrap_or_default(),
                    p.dim("网址  "),
                    url_of(d.card.id),
                ))
            }
        }
        "label" => {
            let id = a.id(sub)?;
            let ops = &a.pos[2..];
            if ops.is_empty() {
                return Err(BoardError::usage("用法：board label <ID> +<标签> -<标签> …"));
            }
            let (mut add, mut remove) = (vec![], vec![]);
            for o in ops {
                if let Some(n) = o.strip_prefix('-') {
                    remove.push(find_label(n)?);
                } else {
                    add.push(find_label(o.strip_prefix('+').unwrap_or(o))?);
                }
            }
            // 仅人可打的标签只有人能给已有卡片加、去；服务端同样会拒绝，这里先拦下，省一次请求
            if let Some(l) = labels.iter().find(|l| l.human_only && (add.contains(&l.id) || remove.contains(&l.id))) {
                return Err(BoardError::usage(format!("标签「{}」只有人能打（仅人可打）：agent 不能给已有卡片加、去，只能在建卡时按人的要求带上", l.name)));
            }
            let d = exec(Cmd::Label { id, add, remove })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                let ls: Vec<String> = d.card.labels.iter().map(|l| p.magenta(&format!("#{}", label_name(*l)))).collect();
                Output::Text(format!(
                    "{} {}  标签：{}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{}", d.card.id)),
                    if ls.is_empty() { p.dim("（无）") } else { ls.join(" ") }
                ))
            }
        }
        "attach" => {
            let id = a.id(sub)?;
            let paths = &a.pos[2..];
            if paths.is_empty() {
                return Err(BoardError::usage("用法：board attach <ID> <文件>… [--comment \"<说明>\"]"));
            }
            // 先全部读出来、按服务端的规则检查（类型、大小、内容和扩展名对得上）：有一个不行就什么也不上传，
            // 免得前面的已经挂到卡片上、后面的才被拒绝，留下没有评论引用的附件（default/main#562）
            let mut blobs = vec![];
            for f in paths {
                let bytes = std::fs::read(f).map_err(|e| BoardError::usage(format!("读不到文件 {f}：{e}")))?;
                let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
                let (name, _, _) = board_core::files::check(&name, &bytes).map_err(|e| BoardError::usage(format!("{f}：{}", e.message)))?;
                blobs.push((name, bytes));
            }
            let mut done: Vec<Attachment> = vec![];
            for (name, bytes) in &blobs {
                done.push(be.attach(&path, id, &actor, name, bytes)?);
            }
            // 附件放进一条评论里，界面的动态里就能看到（图片直接显示）
            let mut text = text_arg(&a, "--comment", "--comment-file")?.map(|s| s.trim().to_string()).unwrap_or_default();
            for f in &done {
                text += &format!("{}{}", if text.is_empty() { "" } else { "\n\n" }, f.markdown);
            }
            let d = be.exec(&path, &actor, Cmd::Comment { id, text })?;
            if json {
                Output::Json(json!({ "attachments": done.iter().map(attachment_json).collect::<Vec<_>>(), "card": fmt.full(&d) }))
            } else {
                let mut s = format!("{} 已上传 {} 个附件到 {}{}\n", p.green("✓"), done.len(), p.cyan(&format!("#{id}")), fmt.renewed(&d, &name));
                for f in &done {
                    s += &format!("  {}  {}  {}\n", p.dim(&format!("{}", f.id)), f.name, p.dim(&human_size(f.size)));
                }
                Output::Text(s)
            }
        }
        "labels" => {
            if json {
                Output::Json(Value::Array(
                    labels
                        .iter()
                        .map(|l| json!({ "name": l.name, "color": l.color, "cards": l.uses, "human_only": l.human_only, "priority": l.priority }))
                        .collect(),
                ))
            } else if labels.is_empty() {
                Output::Text(format!("{}\n", p.dim("（这个项目还没有标签，在界面上新建）")))
            } else {
                let w = labels.iter().map(|l| width(&l.name)).max().unwrap_or(0) + 4;
                Output::Text(
                    labels
                        .iter()
                        .map(|l| {
                            let kind = if l.human_only { "仅人可打" } else { "agent 可打" };
                            let prio = if l.priority { p.yellow("优先") } else { String::new() };
                            format!(
                                "{}{}{}{}\n",
                                p.magenta(&pad(&format!("#{}", l.name), w)),
                                pad(kind, 12),
                                p.dim(&pad(&format!("{} 张卡片", l.uses), 12)),
                                prio
                            )
                        })
                        .collect(),
                )
            }
        }
        // 命令表里有、上面却没处理：返回错误而不是 panic（测试 every_command_is_dispatched 防止发生）
        other => return Err(BoardError::usage(format!("命令「{other}」还没有实现"))),
    };
    Ok(out)
}

/* ------------------------------ 派发 ------------------------------ */

/// `board dispatch <ID> --host <机器名> [--agent <标识>]`、`board dispatch <ID> --cancel`（任务下发需求 §五）
fn dispatch(a: &Args, json: bool, p: &Paint) -> Result<Output> {
    let id = a.id("dispatch")?;
    let raw = a.value("--board").map(|s| s.to_string()).or_else(|| env("BOARD"));
    let Some(path) = raw.filter(|s| !s.trim().is_empty()) else {
        return Err(BoardError::code(ErrorCode::NoBoard));
    };
    let who = a.value("--as").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).or_else(|| env("BOARD_AS"));
    let mut be = Backend::open()?;
    if a.flag("--cancel") {
        let d = be.cancel_dispatch(&path, id, who.as_deref())?;
        if json {
            return Ok(Output::Json(serde_json::to_value(&d).unwrap_or_default()));
        }
        let what = if d.state == "done" {
            format!("派发已取消（{} · {}）", d.host, d.agent_label)
        } else {
            format!("已通知 {} 停止这次派发：结束 agent 进程后卡片移到需要人处理的状态", d.host)
        };
        return Ok(Output::Text(format!("{} {}  {what}\n", p.green("✓"), p.cyan(&format!("#{id}")))));
    }
    // --any：不指定机器，由服务端挑（发给服务端的 host 是空的）
    let any = a.flag("--any");
    let host = match (a.value("--host").map(str::trim).filter(|s| !s.is_empty()), any) {
        (Some(_), true) => return Err(BoardError::usage("--host 和 --any 只能写一个")),
        (Some(h), false) => h,
        (None, true) => "",
        (None, false) => {
            return Err(BoardError::usage(
                "用法：board dispatch <ID> --host <机器名> | --any [--profile <角色>] [--move]，或 board dispatch <ID> --cancel",
            ))
        }
    };
    let moved = a.flag("--move");
    let profile = a.value("--profile").map(str::trim).filter(|s| !s.is_empty());
    let d = if moved {
        be.move_dispatch(&path, id, host, a.value("--agent"), profile, who.as_deref())?
    } else {
        be.dispatch(&path, id, host, a.value("--agent"), profile, who.as_deref())?
    };
    // 谁来做：按角色派的写角色名
    let who_words = if d.profile_name.is_empty() { d.agent_label.clone() } else { format!("{}（{}）", d.profile_name, d.agent_label) };
    if json {
        return Ok(Output::Json(serde_json::to_value(&d).unwrap_or_default()));
    }
    // 任意机器、还没挑到机器
    let host_words = if d.host.is_empty() { "任意机器".to_string() } else { d.host.clone() };
    if moved {
        let tail = if d.host.is_empty() { "等服务端挑一台能接的机器" } else { "排队，机器有空位时开始" };
        return Ok(Output::Text(format!("{} {} 已改派到 {} · {who_words}  {}\n", p.green("✓"), p.cyan(&format!("#{}", d.card)), p.bold(&host_words), p.dim(tail))));
    }
    // 「未整理」里的条目是派给 agent 拆解：不领取、不移动它
    let split = d.kind == board_core::hosts::KIND_SPLIT;
    let (what, after, tail) = if split {
        ("已派给", " 拆解", "排队，机器有空位时开始；这一条留在「未整理」，拆出的卡也记到「未整理」")
    } else {
        ("已派发到", "", "排队，机器有空位时开始")
    };
    Ok(Output::Text(format!(
        "{} {} {what} {} · {who_words}{after}  {}\n",
        p.green("✓"),
        p.cyan(&format!("#{}", d.card)),
        p.bold(&host_words),
        p.dim(tail)
    )))
}

/* ------------------------------ 登录 ------------------------------ */

/// 设置了环境变量时提醒：它们会覆盖配置文件
fn env_overrides(keys: &[&'static str]) -> Vec<&'static str> {
    keys.iter().copied().filter(|k| env(k).is_some()).collect()
}

fn override_hint(p: &Paint, vars: &[&str]) -> String {
    if vars.is_empty() {
        return String::new();
    }
    let exports: Vec<String> = vars.iter().map(|v| format!("export {v}")).collect();
    format!(
        "{} 环境变量 {} 会覆盖配置文件：从 shell 配置文件（例如 ~/.zshrc、~/.bashrc）里删掉对应的 {}\n",
        p.yellow("!"),
        vars.join("、"),
        exports.join("、")
    )
}

/// `board login [--url <地址>] [--token <令牌>]`：用 `GET /api/workspace` 校验令牌，通过后写进配置文件
fn login(a: &Args, json: bool, p: &Paint) -> Result<Output> {
    // --url 给了就不用先读配置文件：现有配置坏了也要能登录（写入时 config::update 会先备份坏文件）
    let url_arg = a.value("--url").map(str::trim).filter(|s| !s.is_empty());
    let file = if url_arg.is_none() { Some(config::File::load()?) } else { None };
    let url = match url_arg {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => file.as_ref().unwrap().get(config::URL)?.ok_or_else(|| {
            BoardError::usage(format!("{} 里还没有服务端地址：board login --url <服务端地址> --token <令牌>", config::shown()))
        })?,
    };
    if !backend::is_http(&url) {
        return Err(BoardError::usage(format!("服务端地址需要以 https:// 或 http:// 开头：{url}")));
    }
    let token = match a.value("--token") {
        Some(t) => t.trim().to_string(),
        None => {
            if std::io::stdin().is_terminal() {
                eprint!("粘贴访问令牌（在界面「Agent → 访问令牌」新建）后回车：");
            }
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).map_err(|e| BoardError::usage(format!("读不到令牌：{e}")))?;
            line.trim().to_string()
        }
    };
    if token.is_empty() {
        return Err(BoardError::usage("没有令牌：board login --token <令牌>，或从标准输入给出一行令牌"));
    }
    // 校验：令牌错误、连不上时按服务端的错误码报错，不写文件
    Remote::new(&url, &token)?.get::<Workspace>("/api/workspace", &[])?;
    // 写的时候重新读一遍（校验期间别处可能改了配置文件，例如桌面端保存了本机设置）
    config::update(|f| {
        f.set(config::URL, &url);
        f.set(config::TOKEN, &token);
        Ok(((), true))
    })?;
    let vars = env_overrides(&["BOARD_URL", "BOARD_TOKEN"]);
    if json {
        return Ok(Output::Json(json!({ "ok": true, "url": url, "config": config::shown(), "env_overrides": vars })));
    }
    Ok(Output::Text(format!(
        "{} 已登录 {}：服务端地址和令牌写进了 {}（不显示令牌原文）\n{}",
        p.green("✓"),
        p.cyan(&url),
        config::shown(),
        override_hint(p, &vars)
    )))
}

/// `board logout`：从配置文件删掉令牌，保留服务端地址
fn logout(json: bool, p: &Paint) -> Result<Output> {
    let removed = config::update(|f| {
        let removed = f.remove(config::TOKEN);
        Ok((removed, removed))
    })?;
    let vars = env_overrides(&["BOARD_TOKEN"]);
    if json {
        return Ok(Output::Json(json!({ "ok": true, "removed": removed, "config": config::shown(), "env_overrides": vars })));
    }
    let head = if removed {
        format!("{} 已退出：从 {} 删掉了令牌，保留服务端地址\n", p.green("✓"), config::shown())
    } else {
        format!("{} {} 里没有令牌\n", p.dim("·"), config::shown())
    };
    Ok(Output::Text(head + &override_hint(p, &vars)))
}

/* ------------------------------ board new 带附件 ------------------------------ */

/// `board new` 的 `--image` / `--file`（按命令行里的顺序）：读出来并按服务端的规则检查，返回 (文件名, 内容)。
/// 有一个不行就报错，什么也不上传；`--image` 只收图片。标题为空时不读文件（服务端会拒绝建卡，不留没用的附件）
fn new_files(a: &Args, no_title: bool) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = vec![];
    for (flag, f) in a.many.iter().filter(|(k, _)| k == "--image" || k == "--file") {
        if no_title {
            return Err(BoardError::new(ErrorCode::EmptyTitle, "标题不能为空"));
        }
        let bytes = std::fs::read(f).map_err(|e| BoardError::usage(format!("{flag}：读不到文件 {f}：{e}")))?;
        let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
        let (name, _, image) = board_core::files::check(&name, &bytes).map_err(|e| BoardError::usage(format!("{flag} {f}：{}", e.message)))?;
        if flag == "--image" && !image {
            return Err(BoardError::usage(format!("--image {f}：不是图片（PNG、JPEG、GIF、WebP）；其他文件用 --file")));
        }
        out.push((name, bytes));
    }
    Ok(out)
}

fn help(p: &Paint) -> String {
    let aw = COMMANDS.iter().map(|r| width(r.1)).max().unwrap_or(0) + 2;
    let mut out = vec![
        format!("{} {}", p.bold("board"), p.dim("— 看板式白板的命令行。人在界面上操作，agent 通过它读写同一块白板。")),
        String::new(),
    ];
    for (cmd, args, desc) in COMMANDS {
        out.push(format!("  board {} {}{}", p.cyan(&pad(cmd, 8)), p.dim(&pad(args, aw)), desc));
    }
    out.extend([
        String::new(),
        format!("  {}默认是 todo  doing  review  done  rework  check；每块白板可以自定义，用 board flow 查看  {}", p.dim("状态："), p.dim("（未整理：inbox）")),
        format!(
            "  {}领卡用 board start <ID> --agent <标识>，一条命令占用名字、启动心跳、领取；不领卡的会话先 board check --agent <标识>（例如 board check --agent dsh），名字由 Board 按这台机器的设置生成，之后不用再带 --as；也可以自己起名字：board check --as <名字>，之后每条命令都带 --as <名字>，或前缀 BOARD_AS=<名字>",
            p.dim("身份：")
        ),
        format!("  {}--board <项目>/<白板>，或前缀 BOARD=<项目>/<白板>", p.dim("白板：")),
        format!(
            "  {}board login --url <服务端地址> --token <令牌>，写进 {}（令牌在界面「Agent → 访问令牌」新建）；环境变量 BOARD_URL/BOARD_TOKEN 优先",
            p.dim("连接："),
            config::shown()
        ),
        format!("  {}所有命令都支持 --json；被拒绝时退出码为 1，并给出错误码", p.dim("输出：")),
        format!("  {}board --version", p.dim("版本：")),
        format!("  {}{}", p.dim("数据："), Backend::describe()),
        String::new(),
    ]);
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 命令表里的每个命令在 run() 里都有处理（help 在前面单独处理）
    #[test]
    fn every_command_is_dispatched() {
        let src = include_str!("main.rs");
        let body = &src[src.find("fn run(").unwrap()..src.find("fn help(").unwrap()];
        for (cmd, _, _) in COMMANDS {
            // `"giveup" | "submit" =>` 这样几条命令共用一个分支的也算
            let handled = [format!("Some(\"{cmd}\")"), format!("sub == \"{cmd}\""), format!("\"{cmd}\" =>"), format!("\"{cmd}\" |")].iter().any(|k| body.contains(k.as_str()));
            assert!(handled, "命令 {cmd} 在命令表里，但 run() 里没有处理");
        }
        let mut names: Vec<_> = COMMANDS.iter().map(|c| c.0).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), COMMANDS.len(), "命令表里有重复");
    }

    /// board hold：连不上时按间隔重试，等到了就接着确认；等不到就 UNREACHABLE（写明改推 wip 分支）；其他错误不重试
    #[test]
    fn hold_retries_only_while_unreachable() {
        let down = || BoardError::new(ErrorCode::Unreachable, "连不上");
        let mut n = 0;
        let mut notes = 0;
        let r = hold_retry(
            || {
                n += 1;
                if n < 3 { Err(down()) } else { Ok(n) }
            },
            Duration::from_millis(500),
            Duration::from_millis(10),
            |_| notes += 1,
        );
        assert_eq!((r.unwrap(), notes), (3, 2), "恢复后接着确认");
        let begun = std::time::Instant::now();
        let e = hold_retry(|| Err::<(), _>(down()), Duration::from_millis(120), Duration::from_millis(30), |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::Unreachable);
        assert!(e.message.contains("wip 分支"), "{}", e.message);
        assert!(begun.elapsed() < Duration::from_millis(400));
        let mut tries = 0;
        let e = hold_retry(
            || {
                tries += 1;
                Err::<(), _>(BoardError::code(ErrorCode::NotFound))
            },
            Duration::from_secs(5),
            Duration::from_millis(10),
            |_| {},
        )
        .unwrap_err();
        assert_eq!((e.code, tries), (ErrorCode::NotFound, 1));
    }

    #[test]
    fn hold_needs_holder_and_locked_status() {
        assert!(hold_check(7, "进行中", true, Some("me/x"), "me/x").is_ok());
        let e = hold_check(7, "待办", false, None, "me/x").unwrap_err();
        assert_eq!(e.code, ErrorCode::NotHolder);
        assert!(e.message.contains("#7 现在在「待办」，没人持有"), "{}", e.message);
        assert!(e.message.contains("不 submit、不 giveup、不移动"), "{}", e.message);
        let e = hold_check(7, "进行中", true, Some("other/y"), "me/x").unwrap_err();
        assert!(e.message.contains("由 other/y 持有"), "{}", e.message);
        assert!(hold_check(7, "待审核", false, Some("me/x"), "me/x").is_err(), "不在锁定状态也不算");
    }

    /* ---------- skill 里写的命令、参数、步骤引用（default/main#550） ---------- */

    use crate::args::{BOOL_FLAGS, VALUE_FLAGS};

    const SKILL: [(&str, &str); 6] = [
        ("SKILL.md", include_str!("../../../.agents/skills/board-skills/SKILL.md")),
        ("work.md", include_str!("../../../.agents/skills/board-skills/work.md")),
        ("plan.md", include_str!("../../../.agents/skills/board-skills/plan.md")),
        ("host.md", include_str!("../../../.agents/skills/board-skills/host.md")),
        ("reference.md", include_str!("../../../.agents/skills/board-skills/reference.md")),
        ("release.md", include_str!("../../../.agents/skills/board-skills/release.md")),
    ];

    /// 文字里写到的 board 命令：（行号，命令）。行内代码和代码块里以 `board ` 开头的（前面可以有 `BOARD=… ` 这类环境变量），
    /// `&&`、`|`、`;` 连着的几条分开算；代码块里 ` #` 之后是注释
    fn skill_commands(text: &str) -> Vec<(usize, String)> {
        let mut out = vec![];
        let mut block = false;
        let pick = |n: usize, code: &str, out: &mut Vec<(usize, String)>| {
            for seg in code.split("&&").flat_map(|s| s.split('|')).flat_map(|s| s.split(';')) {
                let mut words = seg.split_whitespace().skip_while(|w| w.split_once('=').is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c == '_')));
                if words.next() == Some("board") {
                    let rest: Vec<&str> = words.collect();
                    out.push((n, format!("board {}", rest.join(" "))));
                }
            }
        };
        for (i, line) in text.lines().enumerate() {
            let n = i + 1;
            if line.trim_start().starts_with("```") {
                block = !block;
                continue;
            }
            if block {
                let code = line.split(" #").next().unwrap_or(line);
                pick(n, code, &mut out);
            } else {
                for (k, part) in line.split('`').enumerate() {
                    if k % 2 == 1 {
                        pick(n, part, &mut out);
                    }
                }
            }
        }
        out
    }

    /// 命令表里有这条命令，写到的 `--参数` 都认识
    fn check_command(cmd: &str) -> std::result::Result<(), String> {
        let Some(sub) = cmd.split_whitespace().nth(1) else { return Ok(()) };
        if sub.starts_with('<') || sub.starts_with('[') {
            return Ok(());
        }
        if !COMMANDS.iter().any(|c| c.0 == sub) {
            return Err(format!("没有 board {sub} 这条命令"));
        }
        let b = cmd.as_bytes();
        let mut i = 0;
        while let Some(k) = cmd[i..].find("--") {
            let at = i + k;
            let mut end = at + 2;
            while end < b.len() && (b[end].is_ascii_lowercase() || b[end].is_ascii_digit() || b[end] == b'-') {
                end += 1;
            }
            let flag = &cmd[at..end];
            let starts_word = at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'-');
            if starts_word && flag.len() > 2 && !VALUE_FLAGS.contains(&flag) && !BOOL_FLAGS.contains(&flag) {
                return Err(format!("没有 {flag} 这个参数"));
            }
            i = end.max(at + 2);
        }
        Ok(())
    }

    /// 「『小节』第 N 步」「`x.md`『小节』第 N–M 步」这类引用：（行号，原文，文件，小节，步骤号）
    fn step_refs(file: &'static str, text: &str) -> Vec<(usize, String, String, String, Vec<usize>)> {
        let mut out = vec![];
        for (i, line) in text.lines().enumerate() {
            let mut from = 0;
            while let Some(k) = line[from..].find("」第") {
                let close = from + k;
                from = close + "」第".len();
                let Some(open) = line[..close].rfind('「') else { continue };
                let section = line[open + '「'.len_utf8()..close].rsplit('→').next().unwrap_or("").trim().to_string();
                let tail = line[from..].trim_start();
                let num: String = tail.chars().take_while(|c| c.is_ascii_digit() || *c == '–' || *c == '-').collect();
                if num.is_empty() || !tail[num.len()..].trim_start().starts_with('步') {
                    continue;
                }
                let steps: Vec<usize> = num.split(['–', '-']).filter_map(|x| x.parse().ok()).collect();
                let before = line[..open].trim_end().trim_end_matches('`');
                let target = before
                    .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
                    .next()
                    .filter(|w| w.ends_with(".md"))
                    .map(str::to_string)
                    .unwrap_or_else(|| file.to_string());
                let rest = &line[open..];
                let raw = &rest[..rest.find('步').map_or(rest.len(), |p| p + '步'.len_utf8())];
                out.push((i + 1, raw.to_string(), target, section, steps));
            }
        }
        out
    }

    /// `file` 的小节 `section`（标题）里有没有编号为 `n` 的步骤（顶格的 `n. `）
    fn has_step(text: &str, section: &str, n: usize) -> std::result::Result<(), String> {
        let lines: Vec<&str> = text.lines().collect();
        let level = |l: &str| l.chars().take_while(|c| *c == '#').count();
        let Some(at) = lines.iter().position(|l| level(l) > 0 && l.trim_start_matches('#').trim() == section) else {
            return Err(format!("没有「{section}」这一节"));
        };
        let lv = level(lines[at]);
        let body = lines[at + 1..].iter().take_while(|l| !(level(l) > 0 && level(l) <= lv));
        let mut seen = body.filter(|l| l.starts_with(&format!("{n}. ")));
        if seen.next().is_none() {
            return Err(format!("「{section}」里没有第 {n} 步"));
        }
        Ok(())
    }

    fn skill_problems(files: &[(&'static str, String)]) -> Vec<String> {
        let mut bad = vec![];
        for (file, text) in files {
            for (n, cmd) in skill_commands(text) {
                if let Err(why) = check_command(&cmd) {
                    bad.push(format!("{file}:{n}：{why}：{cmd}"));
                }
            }
            for (n, raw, target, section, steps) in step_refs(file, text) {
                let Some((_, t)) = files.iter().find(|(f, _)| *f == target) else {
                    bad.push(format!("{file}:{n}：没有 {target} 这个文件：{raw}"));
                    continue;
                };
                for s in steps {
                    if let Err(why) = has_step(t, &section, s) {
                        bad.push(format!("{file}:{n}：{target} {why}：{raw}"));
                    }
                }
            }
        }
        bad
    }

    /// skill 里写到的每条 board 命令、参数都在命令表里，「第 N 步」的引用都指得到
    #[test]
    fn skill_mentions_only_real_commands_and_steps() {
        let files: Vec<(&'static str, String)> = SKILL.iter().map(|(f, t)| (*f, t.to_string())).collect();
        let bad = skill_problems(&files);
        assert!(bad.is_empty(), "skill 里有和命令行对不上的地方：\n{}", bad.join("\n"));
        // 真的抽到了东西：不是什么都没查
        let cmds: usize = files.iter().map(|(_, t)| skill_commands(t).len()).sum();
        let refs: usize = files.iter().map(|(f, t)| step_refs(f, t).len()).sum();
        assert!(cmds > 50 && refs > 10, "抽到的命令 {cmds} 条、步骤引用 {refs} 处");
    }

    /// 写错命令、参数、步骤时报出文件、行号和那一条
    #[test]
    fn skill_check_reports_where() {
        let mut files: Vec<(&'static str, String)> = SKILL.iter().map(|(f, t)| (*f, t.to_string())).collect();
        files[0].1.push_str("\n用 `board foo 1` 和 `board start --nope`，见 work.md「会话开始」第 9 步。\n\n```sh\nBOARD=a/b board lnd 2\n```\n");
        let at = |needle: &str| files[0].1.lines().position(|l| l.contains(needle)).unwrap() + 1;
        let (n, m) = (at("board foo 1"), at("board lnd 2"));
        let bad = skill_problems(&files);
        assert_eq!(
            bad,
            vec![
                format!("SKILL.md:{n}：没有 board foo 这条命令：board foo 1"),
                format!("SKILL.md:{n}：没有 --nope 这个参数：board start --nope"),
                format!("SKILL.md:{m}：没有 board lnd 这条命令：board lnd 2"),
                format!("SKILL.md:{n}：work.md 「会话开始」里没有第 9 步：「会话开始」第 9 步"),
            ]
        );
    }

    #[test]
    fn help_lists_every_command() {
        let h = help(&Paint { on: false });
        for (cmd, _, _) in COMMANDS {
            assert!(h.contains(&format!("board {cmd}")), "help 里没有 {cmd}");
        }
        let identity = h.lines().find(|l| l.contains("身份：")).unwrap();
        assert!(identity.contains("领卡用 board start <ID> --agent <标识>"), "{identity}");
        assert!(identity.contains("不领卡的会话先 board check --agent <标识>"), "{identity}");
    }

    #[test]
    fn version_flag_prints_cargo_version() {
        let p = Paint { on: false };
        let v = env!("CARGO_PKG_VERSION");
        for flag in ["--version", "-V"] {
            match run(&[flag.to_string()], &p) {
                Ok(Output::Text(t)) => assert_eq!(t, format!("board {v}\n")),
                _ => panic!("{flag} 应该打印版本号"),
            }
            match run(&[flag.to_string(), "--json".to_string()], &p) {
                Ok(Output::Json(j)) => assert_eq!(j, json!({ "version": v })),
                _ => panic!("{flag} --json 应该输出 JSON"),
            }
        }
    }

    /// `board host rename` 缺新名字时报用法错（不会真的去常驻或改名）
    #[test]
    fn host_rename_requires_a_name() {
        let p = Paint { on: false };
        match run(&["host".into(), "rename".into()], &p) {
            Err(e) => assert!(e.message.contains("用法"), "{}", e.message),
            Ok(_) => panic!("缺新名字时应当报用法错"),
        }
    }

    /// `board host` 后面只认 run 和 rename：敲错子命令时不能启动 host
    #[test]
    fn host_rejects_unknown_subcommands() {
        let p = Paint { on: false };
        for arg in ["ls", "foo"] {
            match run(&["host".into(), arg.into()], &p) {
                Err(e) => {
                    assert_eq!(e.code, ErrorCode::Usage, "{}", e.message);
                    assert!(e.message.contains(&format!("未知子命令「{arg}」")), "{}", e.message);
                    assert!(e.message.contains("用法"), "{}", e.message);
                }
                Ok(_) => panic!("board host {arg} 不能启动 host"),
            }
        }
        // run 后面多写的位置参数也不猜意思
        match run(&["host".into(), "run".into(), "extra".into()], &p) {
            Err(e) => {
                assert_eq!(e.code, ErrorCode::Usage, "{}", e.message);
                assert!(e.message.contains("多余的位置参数「extra」"), "{}", e.message);
            }
            Ok(_) => panic!("board host run extra 不能启动 host"),
        }
    }
}
