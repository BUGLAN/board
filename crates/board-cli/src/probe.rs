//! 派发的运行状态（PRD §9.1「执行」）：board host 随每 10 秒的机器心跳上报每个在跑的派发「日志最后一次增长是多久之前」
//! 和「agent 进程树最近一段时间的 CPU 占用」，界面据此判断 agent 在干活、等待输出还是可能在空转。
//! 机器心跳本身是 board host 替 agent 名字发的，agent 卡住、空转时照发，不能说明 agent 在干什么。
//!
//! - 进程树：agent 主进程和它的子孙（按父进程号找）；Unix 上再加上同一会话里的进程（board host 启动 agent 时 setsid，
//!   会话号就是主进程号；中间的进程退出、子进程被收养之后父进程号对不上，会话号还在）。
//! - CPU 占用：两次采样之间进程树里各进程累计 CPU 时间之差，除以经过的时间，取整数百分比，单核满载算 100%，各进程累加。
//!   第一次采样从 agent 启动算起（进程树里的进程都是那之后才启动的）。两次采样之间就退出了的进程，它最后那一段不计。
//! - 进程信息用 sysinfo 取（和 heartbeat.rs 一样，不用 ps、WMI）。WSL 里的 agent 不在 Windows 的进程表里，量不到，不在这里采样。

use std::collections::{HashMap, HashSet};
use std::time::Instant;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// 一个 agent 进程树上一次的采样
#[derive(Debug, Default)]
pub(crate) struct CpuSample {
    at: Option<Instant>,
    /// 进程号 → 累计 CPU 时间（毫秒）
    acc: HashMap<u32, u64>,
}

/// 本机所有进程的 CPU 时间：每次上报前刷新一次，再对每个在跑的 agent 采样
#[derive(Default)]
pub(crate) struct CpuMeter(Option<System>);

impl CpuMeter {
    pub(crate) fn refresh(&mut self) {
        if self.0.is_none() {
            self.0 = System::new().ok();
        }
        if let Some(s) = self.0.as_mut() {
            s.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cpu().without_tasks());
        }
    }

    /// root 的进程树里每个进程的累计 CPU 时间（毫秒）；进程表里没有 root 时为空
    fn tree(&self, root: u32) -> Option<HashMap<u32, u64>> {
        let s = self.0.as_ref()?;
        let top = Pid::from_u32(root);
        s.process(top)?;
        let mut kids: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (pid, p) in s.processes() {
            if p.thread_kind().is_some() {
                continue;
            }
            if let Some(pp) = p.parent() {
                kids.entry(pp).or_default().push(*pid);
            }
        }
        let mut seen: HashSet<Pid> = HashSet::from([top]);
        let mut stack = vec![top];
        while let Some(p) = stack.pop() {
            for k in kids.get(&p).into_iter().flatten() {
                if seen.insert(*k) {
                    stack.push(*k);
                }
            }
        }
        #[cfg(unix)]
        for (pid, p) in s.processes() {
            if p.thread_kind().is_none() && !seen.contains(pid) && p.session_id() == Some(top) {
                seen.insert(*pid);
            }
        }
        Some(seen.into_iter().filter_map(|pid| s.process(pid).map(|p| (pid.as_u32(), p.accumulated_cpu_time()))).collect())
    }

    /// root 的进程树从上一次采样（第一次从 since，即 agent 启动时）到现在的 CPU 占用；进程表里没有 root 时为空
    pub(crate) fn sample(&self, root: u32, since: Instant, prev: &mut CpuSample) -> Option<i64> {
        let now = Instant::now();
        let acc = self.tree(root)?;
        let ms = now.duration_since(prev.at.unwrap_or(since)).as_millis() as u64;
        let used = cpu_used(&prev.acc, &acc);
        prev.at = Some(now);
        prev.acc = acc;
        Some(cpu_percent(used, ms))
    }
}

/// 两次采样之间进程树用掉的 CPU 时间（毫秒）：上一次就在的进程算差值，新出现的进程算它的全部
fn cpu_used(before: &HashMap<u32, u64>, now: &HashMap<u32, u64>) -> u64 {
    now.iter().map(|(pid, t)| t.saturating_sub(before.get(pid).copied().unwrap_or(0))).sum()
}

/// 用掉的 CPU 时间占经过时间的百分比（四舍五入），单核满载 100%
fn cpu_percent(used_ms: u64, wall_ms: u64) -> i64 {
    let wall = wall_ms.max(1);
    ((used_ms * 100 + wall / 2) / wall) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_counts_new_and_running_processes() {
        let before = HashMap::from([(1, 1000), (2, 500), (9, 300)]);
        // 1 用了 200，2 没动，3 是新的（300），9 已经退出（不计）
        let now = HashMap::from([(1, 1200), (2, 500), (3, 300)]);
        assert_eq!(cpu_used(&before, &now), 500);
        assert_eq!(cpu_percent(500, 10_000), 5);
        assert_eq!(cpu_percent(0, 10_000), 0);
        assert_eq!(cpu_percent(25_000, 10_000), 250, "多核累加可以超过 100%");
        assert_eq!(cpu_percent(10, 0), 1000, "经过时间为 0 时不除以 0");
    }

    /// 真的启动一个空转 CPU 的子进程和一个只 sleep 的子进程：前者占用高，后者接近 0
    #[cfg(unix)]
    #[test]
    fn measures_busy_and_idle_trees() {
        use std::os::unix::process::CommandExt;
        let spawn = |script: &str| {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", script]);
            unsafe {
                c.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
            c.spawn().unwrap()
        };
        // 忙的那个在孙进程里空转：要靠进程树（会话）才算得到
        let mut busy = spawn("sh -c 'while :; do :; done' & wait");
        let mut idle = spawn("sleep 30");
        let t0 = Instant::now();
        let mut m = CpuMeter::default();
        let (mut a, mut b) = (CpuSample::default(), CpuSample::default());
        m.refresh();
        m.sample(busy.id(), t0, &mut a);
        m.sample(idle.id(), t0, &mut b);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        m.refresh();
        let hot = m.sample(busy.id(), t0, &mut a).unwrap();
        let cold = m.sample(idle.id(), t0, &mut b).unwrap();
        crate::envs::kill_tree(busy.id(), true);
        crate::envs::kill_tree(idle.id(), true);
        let _ = (busy.wait(), idle.wait());
        assert!(hot >= 30, "空转的进程树 CPU 应该很高：{hot}%");
        assert!(cold <= 2, "只 sleep 的进程树 CPU 应该接近 0：{cold}%");
        assert!(m.sample(999_999_999, t0, &mut CpuSample::default()).is_none(), "进程表里没有就是量不到");
    }
}
