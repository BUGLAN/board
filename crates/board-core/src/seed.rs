//! 示例工作区（`board-server --demo`）：三个项目、五块白板，演示开源项目自动维护场景（PRD §14）。
//! 只在数据库里还没有任何卡片、也没有建过别的项目时写入。

use crate::engine::Cmd;
use crate::error::Result;
use crate::model::*;
use crate::store::{NewBoard, Store, W};

const M: i64 = 60 * 1000;
const H: i64 = 60 * M;
const D: i64 = 24 * H;

fn new(title: &str, body: &str) -> Cmd {
    Cmd::New { title: title.into(), body: Some(body.into()), column: None, top: false, labels: vec![] }
}
fn inbox(title: &str, body: &str) -> Cmd {
    Cmd::New { title: title.into(), body: Some(body.into()), column: Some("inbox".into()), top: false, labels: vec![] }
}
fn claim(id: i64) -> Cmd {
    Cmd::Claim { id: Some(id) }
}
fn mv(id: i64, col: &str) -> Cmd {
    Cmd::Move { id, column: col.into(), top: false, index: None }
}
fn comment(id: i64, text: &str) -> Cmd {
    Cmd::Comment { id, text: text.into() }
}

/// 是否适合写入示例数据：没有卡片，且只有未改动过的默认项目（或什么都没有）
pub fn is_fresh(store: &Store) -> Result<bool> {
    let conn = store.conn();
    let cards: i64 = conn.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0))?;
    let others: i64 = conn.query_row("SELECT COUNT(*) FROM projects WHERE key != 'default'", [], |r| r.get(0))?;
    let boards: i64 = conn.query_row("SELECT COUNT(*) FROM boards WHERE project = 'default'", [], |r| r.get(0))?;
    Ok(cards == 0 && others == 0 && boards <= 1)
}

pub fn seed_demo(store: &mut Store, now: i64) -> Result<bool> {
    if !is_fresh(store)? {
        return Ok(false);
    }
    store.write(now, Origin::system(), |w| {
        w.tx.execute("DELETE FROM projects", [])?;
        let h = Actor::human();
        let a = |n: &str| Actor::agent(n);

        // ---------------- 项目与白板 ----------------
        let nb = |name: &str, key: &str, desc: &str, x: f64, y: f64| NewBoard {
            name: name.into(),
            key: key.into(),
            desc: desc.into(),
            pos: Some(Pos { x, y }),
        };
        w.at(now - 30 * D).create_project(
            "Board 开源仓库",
            "oss",
            "自动维护 board 的 GitHub 仓库",
            Some("#FF6A33"),
            Some(Pos { x: 0.0, y: 40.0 }),
            nb("维护主线", "main", "issue、社区反馈与定期清理", 420.0, -60.0),
        )?;
        w.at(now - 6 * D).create_board("oss", "v1.0 发布", "release", "发布前必须完成的事项", Some(Pos { x: 420.0, y: 160.0 }))?;
        w.at(now - 12 * D).create_project(
            "文档站",
            "docs",
            "board 的使用文档与示例",
            Some("#4DB5FF"),
            Some(Pos { x: 0.0, y: 470.0 }),
            nb("内容", "content", "命令参考、快速开始、错误码", 420.0, 380.0),
        )?;
        w.at(now - 4 * D).create_board("docs", "英文翻译", "i18n", "", Some(Pos { x: 420.0, y: 590.0 }))?;
        w.at(now - 3 * D).create_project(
            "官网改版",
            "site",
            "新版首页与定价页，下周上线",
            Some("#3DDC97"),
            Some(Pos { x: 0.0, y: 820.0 }),
            nb("上线清单", "launch", "", 420.0, 800.0),
        )?;

        // ---------------- 标签 ----------------
        let lb = |w: &mut W, p: &str, list: &[(&str, &str)]| -> Result<Vec<(String, i64)>> {
            let mut out = vec![];
            for (name, color) in list {
                let l = w.create_label(p, name, Some(color))?;
                out.push((name.to_string(), l.id));
            }
            Ok(out)
        };
        let oss_l = lb(w, "oss", &[
            ("bug", "#F43F5E"),
            ("文档", "#3B82F6"),
            ("重构", "#8B5CF6"),
            ("社区", "#22C55E"),
            ("发布", "#EAB308"),
            ("Windows", "#14B8A6"),
        ])?;
        let docs_l = lb(w, "docs", &[("写作", "#3B82F6"), ("截图", "#F97316"), ("翻译", "#14B8A6")])?;
        let site_l = lb(w, "site", &[("设计", "#EC4899"), ("文案", "#EAB308")])?;
        let ids = |set: &[(String, i64)], names: &[&str]| -> Vec<i64> {
            names.iter().filter_map(|n| set.iter().find(|(k, _)| k == n).map(|(_, id)| *id)).collect()
        };

        let run = |w: &mut W, b: &BoardRef, t: i64, who: &Actor, cmd: Cmd| -> Result<()> {
            w.at(t).exec(b, who, cmd)?;
            Ok(())
        };

        // ---------------- oss/main：开源项目自动维护场景 ----------------
        let b = w.resolve("oss/main")?;
        let issue = a("issue-watcher");
        let cleaner = a("cleaner");
        let t0 = now - 6 * 60 * M;
        run(w, &b, t0, &issue, new("`board ls --search` 支持匹配描述里的 gh 编号", "gh#118\n\n**验收标准**\n- `board ls --search gh#118` 能找到描述里写了 gh#118 的卡片\n- 大小写不敏感"))?;
        run(w, &b, t0 + 5 * M, &issue, new("Windows 下路径分隔符导致快照测试失败", "gh#140 · 来自 issue 监听\n\n在 Windows runner 上 `tests/snapshot.rs` 有 3 个用例失败，路径里出现了 `\\\\`。\n\n**验收标准**\n- [ ] 快照里统一使用 `/`\n- [ ] CI 增加 windows-latest"))?;
        run(w, &b, t0 + 9 * M, &issue, new("无网络时 `sync` 命令直接 panic", "gh#152\n\n```\nthread 'main' panicked at src/net.rs:88\n```\n\n应当给出可读的错误信息并以退出码 1 结束。"))?;
        run(w, &b, t0 + 20 * M, &cleaner, new("清理已废弃的 v1 配置解析代码", "`src/config/v1.rs` 自 0.9 起已无调用方。删除前确认 `cargo test` 全绿。"))?;
        run(w, &b, t0 + 30 * M, &h, new("文档：为每个命令补充 `--json` 输出示例", "参考 README 的「CLI」一节，每个命令给一段输出样例。"))?;
        run(w, &b, t0 + 44 * M, &a("community-watcher"), new("论坛反馈：`claim` 超时时间希望可配置", "来源：https://forum.example.org/t/claim-timeout/311\n\n先收集更多反馈，暂不处理。"))?;
        run(w, &b, t0 + 60 * M, &cleaner, new("升级 tokio 到 1.40，移除 `rt-multi-thread` 之外的 feature", ""))?;
        run(w, &b, t0 + 70 * M, &issue, new("README 里的安装命令在 zsh 下报错", "gh#161\n\n`curl ... | sh -s -- --to ~/.local/bin` 在 zsh 中 `~` 未展开。"))?;
        // #1：已完成
        run(w, &b, t0 + 80 * M, &a("alpha"), claim(1))?;
        run(w, &b, t0 + 96 * M, &a("alpha"), comment(1, "已实现：搜索范围扩展到描述，补了 4 个测试。"))?;
        run(w, &b, t0 + 98 * M, &a("alpha"), mv(1, "review"))?;
        run(w, &b, t0 + 150 * M, &h, comment(1, "看过了，没问题 👍"))?;
        run(w, &b, t0 + 151 * M, &h, mv(1, "done"))?;
        // #4、#8：待审核
        run(w, &b, t0 + 160 * M, &a("beta"), claim(4))?;
        run(w, &b, t0 + 175 * M, &a("beta"), comment(4, "确认 `v1.rs` 无引用，已删除 412 行；cargo test 全绿。"))?;
        run(w, &b, t0 + 176 * M, &a("beta"), mv(4, "review"))?;
        run(w, &b, t0 + 200 * M, &a("gamma"), claim(8))?;
        run(w, &b, t0 + 214 * M, &a("gamma"), comment(8, "改为使用 `$HOME`，并在 CI 里加了 zsh / bash 两个 shell 的冒烟测试。"))?;
        run(w, &b, t0 + 215 * M, &a("gamma"), mv(8, "review"))?;
        // 人调整优先级：#3 提到最上面
        run(w, &b, t0 + 230 * M, &h, Cmd::Move { id: 3, column: "todo".into(), top: false, index: Some(0) })?;
        // 未整理：人随手记下的计划和想法
        run(w, &b, now - 50 * M, &h, inbox("调研：一个仓库是否需要多块白板", "- 按子项目拆？\n- 还是用标签区分？\n\n先看看社区里其他项目怎么做。"))?;
        run(w, &b, now - 34 * M, &h, inbox("「待审核」里加一个打回的快捷操作", "现在打回要拖回待办，再写一条评论说明原因，有点绕。"))?;
        run(w, &b, now - 20 * M, &h, inbox("CI 缓存总是失效，查一下 key 的计算方式", ""))?;
        run(w, &b, now - 6 * M, &h, inbox("整理 11 月社区反馈，挑出值得做的", "论坛 + Discord 两个来源。"))?;
        // #2：beta 正在做（剩约 4 分钟）；#3：alpha 正在做（剩约 19 分钟）
        run(w, &b, now - 26 * M, &a("beta"), claim(2))?;
        run(w, &b, now - 26 * M + 30_000, &a("beta"), comment(2, "已定位到 `snapshot::normalize`，准备统一替换分隔符。"))?;
        run(w, &b, now - 15 * M, &a("alpha"), claim(3))?;
        run(w, &b, now - 11 * M, &a("alpha"), comment(3, "复现了：`reqwest` 的错误被 `unwrap()` 了。正在改成返回 `Result`。"))?;
        for (id, names) in [
            (1, &["社区"][..]),
            (2, &["bug", "Windows"]),
            (3, &["bug"]),
            (4, &["重构"]),
            (5, &["文档"]),
            (6, &["社区"]),
            (7, &["重构"]),
            (8, &["bug", "文档"]),
        ] {
            w.set_labels_raw(b.id, id, &ids(&oss_l, names))?;
        }

        // ---------------- oss/release ----------------
        let b = w.resolve("oss/release")?;
        run(w, &b, now - 900 * M, &h, new("冻结 CLI 参数，写进 CHANGELOG", ""))?;
        run(w, &b, now - 800 * M, &h, new("各平台安装包签名（macOS / Windows）", ""))?;
        run(w, &b, now - 700 * M, &h, new("发布公告草稿", ""))?;
        run(w, &b, now - 600 * M, &h, new("跑一遍完整的验收标准（PRD §13）", "**验收标准**\n- [ ] 26 条全部通过"))?;
        run(w, &b, now - 500 * M, &a("delta"), claim(1))?;
        run(w, &b, now - 480 * M, &a("delta"), mv(1, "review"))?;
        run(w, &b, now - 9 * M, &a("epsilon"), claim(2))?;
        run(w, &b, now - 3 * M, &a("epsilon"), comment(2, "macOS 公证已提交，等待 Apple 返回。"))?;
        for (id, names) in [(1, &["发布"][..]), (2, &["发布", "Windows"]), (3, &["发布", "文档"]), (4, &["发布"])] {
            w.set_labels_raw(b.id, id, &ids(&oss_l, names))?;
        }

        // ---------------- docs/content ----------------
        let b = w.resolve("docs/content")?;
        run(w, &b, now - 300 * M, &h, new("命令参考：补全 `board move` 的 --top 说明", ""))?;
        run(w, &b, now - 280 * M, &h, new("快速开始：五分钟接入一个 agent", "**验收标准**\n- [ ] 从安装到第一次 `board claim` 不超过 5 步\n- [ ] 附一个 shell 循环示例"))?;
        run(w, &b, now - 260 * M, &a("alpha"), claim(1))?;
        run(w, &b, now - 236 * M, &a("alpha"), mv(1, "review"))?; // 领取 30 分钟内交审
        run(w, &b, now - 200 * M, &a("issue-watcher"), new("错误码页面缺少 `EMPTY_TODO` 的说明", "gh#77"))?;
        run(w, &b, now - 200 * M, &h, mv(1, "done"))?;
        run(w, &b, now - 150 * M, &h, new("示例仓库截图过期，需要重截", ""))?;
        run(w, &b, now - 120 * M, &h, new("中文文档术语统一（领取 / 认领）", ""))?;
        run(w, &b, now - 90 * M, &a("delta"), claim(3))?;
        run(w, &b, now - 70 * M, &a("delta"), mv(3, "review"))?;
        run(w, &b, now - 40 * M, &h, inbox("要不要做一个交互式的命令演示？", ""))?;
        run(w, &b, now - 12 * M, &a("gamma"), claim(2))?;
        run(w, &b, now - 6 * M, &a("gamma"), comment(2, "初稿写好了，正在补 shell 循环示例。"))?;
        for (id, names) in [(1, &["写作"][..]), (2, &["写作"]), (4, &["截图"]), (5, &["写作"])] {
            w.set_labels_raw(b.id, id, &ids(&docs_l, names))?;
        }

        // ---------------- docs/i18n ----------------
        let b = w.resolve("docs/i18n")?;
        run(w, &b, now - 300 * M, &h, new("翻译「快速开始」", ""))?;
        run(w, &b, now - 280 * M, &h, new("翻译错误码页面", ""))?;
        run(w, &b, now - 60 * M, &h, inbox("考虑接入机器翻译 + 人工校对", ""))?;
        for id in [1, 2] {
            w.set_labels_raw(b.id, id, &ids(&docs_l, &["翻译"]))?;
        }

        // ---------------- site/launch ----------------
        let b = w.resolve("site/launch")?;
        run(w, &b, now - 600 * M, &h, new("首页首屏：一句话讲清「人和 agent 共用一块白板」", ""))?;
        run(w, &b, now - 500 * M, &h, new("定价页：开源版 / 团队版对比表", ""))?;
        run(w, &b, now - 400 * M, &h, new("OG 图与 favicon", ""))?;
        run(w, &b, now - 380 * M, &a("beta"), claim(3))?;
        run(w, &b, now - 360 * M, &a("beta"), mv(3, "review"))?;
        run(w, &b, now - 300 * M, &h, mv(3, "done"))?;
        run(w, &b, now - 100 * M, &h, inbox("客户案例页先放三个", ""))?;
        run(w, &b, now - 50 * M, &h, inbox("博客要不要迁到同一个域名", ""))?;
        for (id, names) in [(1, &["设计", "文案"][..]), (2, &["文案"]), (3, &["设计"])] {
            w.set_labels_raw(b.id, id, &ids(&site_l, names))?;
        }

        w.at(now);
        Ok(())
    })?;
    Ok(true)
}
