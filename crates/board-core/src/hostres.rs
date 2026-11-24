//! 机器资源（PRD §10.12「资源」）：board host 在后台每 10 秒采样一次 CPU、内存、GPU、磁盘，随上报带给服务端。
//! 服务端存最近一次，另记最近 [`RES_HIST`] 次的 CPU、内存、GPU 使用率（机器详情「资源」磁贴上的趋势线）。
//! 资源每次上报都在变：只写库、不算机器有变化（不推送），界面打开机器详情时每 10 秒取一次机器列表看到新的
use serde::{Deserialize, Serialize};

/// 趋势线记几次（10 秒一次，5 分钟）
pub const RES_HIST: usize = 30;
/// 和上一次采样隔得比这久（离线过、board host 停过）：趋势从头记，不把中间断开的接成一条线
pub const RES_HIST_GAP_MS: i64 = 60_000;
/// 后台采样卡住了（例如网络盘没响应）：这么久以前的采样不报
pub const RES_STALE_MS: i64 = 120_000;

const MAX_GPUS: usize = 8;
const MAX_DISKS: usize = 8;
const MAX_TEXT: usize = 80;

/// 一台机器的资源
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRes {
    /// 采样的时间（毫秒，服务端的时钟：收到时间减去 board host 报的「采样后过了多久」）
    #[serde(default)]
    pub at: i64,
    /// board host 上报：采样之后过了多久（毫秒）；服务端换算成 `at`，不存
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<i64>,
    /// 取不到时为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<ResCpu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem: Option<ResMem>,
    /// 取不到（这个系统不支持）时为空；找过、没有 GPU 时是空列表
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpus: Option<Vec<ResGpu>>,
    /// 本机的盘：工作空间所在的排第一；取不到时为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disks: Option<Vec<ResDisk>>,
    /// 最近几次的使用率（服务端记，旧的在前）
    #[serde(default)]
    pub hist: ResHist,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResCpu {
    /// 整机使用率（0–100）
    pub pct: u8,
    /// 逻辑核数
    #[serde(default)]
    pub cores: u32,
    /// 1、5、15 分钟平均负载；Windows 没有
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<[f64; 3]>,
    /// 型号
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResMem {
    /// 字节
    pub total: u64,
    pub used: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResGpu {
    /// 型号（例如 NVIDIA GeForce RTX 4070、Apple M3 Pro）
    #[serde(default)]
    pub name: String,
    /// 厂商（NVIDIA、AMD、Intel、Apple）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vendor: String,
    /// 使用率（0–100）；读不到时为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pct: Option<u8>,
    /// 显存（字节）；Apple 芯片和内存共用，没有
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_total: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResDisk {
    /// 挂载点（`/`、`/data`、`C:\`）
    pub mount: String,
    /// 卷名（macOS 的「Macintosh HD」）；没有时为空
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// 字节
    pub total: u64,
    pub avail: u64,
    /// 工作空间在这块盘上
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ws: bool,
}

/// 最近几次的使用率（0–100，旧的在前，最多 [`RES_HIST`] 个）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResHist {
    #[serde(default)]
    pub cpu: Vec<u8>,
    #[serde(default)]
    pub mem: Vec<u8>,
    /// 第一块 GPU
    #[serde(default)]
    pub gpu: Vec<u8>,
}

fn clip(s: &str) -> String {
    s.trim().chars().take(MAX_TEXT).collect()
}

fn pct(x: u8) -> u8 {
    x.min(100)
}

impl ResMem {
    pub fn pct(&self) -> Option<u8> {
        (self.total > 0).then(|| ((self.used.min(self.total) as f64 / self.total as f64) * 100.0).round() as u8)
    }
}

impl HostRes {
    /// 服务端收到一次上报：换算采样时间，清理字段，接上存着的趋势（隔太久就从头记）。
    /// 后台采样卡住太久（采样时间早于 [`RES_STALE_MS`]）的不收，返回 None
    pub fn received(self, prev: Option<&HostRes>, now: i64) -> Option<HostRes> {
        let age = self.age.unwrap_or(0).max(0);
        if age > RES_STALE_MS {
            return None;
        }
        let at = now - age;
        let cpu =
            self.cpu.map(|c| ResCpu { pct: pct(c.pct), model: clip(&c.model), load: c.load.filter(|l| l.iter().all(|x| x.is_finite() && *x >= 0.0)), ..c });
        let mem = self.mem.filter(|m| m.total > 0);
        let gpus = self.gpus.map(|g| {
            g.into_iter()
                .take(MAX_GPUS)
                .map(|g| ResGpu {
                    name: clip(&g.name),
                    vendor: clip(&g.vendor),
                    pct: g.pct.map(pct),
                    vram_total: g.vram_total.filter(|t| *t > 0),
                    vram_used: g.vram_used.filter(|_| g.vram_total.is_some_and(|t| t > 0)),
                })
                .collect::<Vec<_>>()
        });
        let disks = self.disks.map(|d| {
            d.into_iter()
                .filter(|d| d.total > 0)
                .take(MAX_DISKS)
                .map(|d| ResDisk { mount: clip(&d.mount), label: clip(&d.label), avail: d.avail.min(d.total), ..d })
                .collect::<Vec<_>>()
        });
        let mut hist = match prev {
            Some(p) if p.at > 0 && at >= p.at && at - p.at <= RES_HIST_GAP_MS => p.hist.clone(),
            _ => ResHist::default(),
        };
        let push = |v: &mut Vec<u8>, x: Option<u8>| {
            if let Some(x) = x {
                v.push(x);
                if v.len() > RES_HIST {
                    v.drain(..v.len() - RES_HIST);
                }
            }
        };
        push(&mut hist.cpu, cpu.as_ref().map(|c| c.pct));
        push(&mut hist.mem, mem.as_ref().and_then(ResMem::pct));
        push(&mut hist.gpu, gpus.as_ref().and_then(|g| g.first()).and_then(|g| g.pct));
        Some(HostRes { at, age: None, cpu, mem, gpus, disks, hist })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(cpu: u8) -> HostRes {
        HostRes {
            cpu: Some(ResCpu { pct: cpu, cores: 8, load: None, model: "  Apple M3  ".into() }),
            mem: Some(ResMem { total: 100, used: 64 }),
            gpus: Some(vec![ResGpu { name: "A10".into(), pct: Some(120), vram_used: Some(5), vram_total: Some(10), ..Default::default() }]),
            disks: Some(vec![
                ResDisk { mount: "/".into(), total: 100, avail: 300, ..Default::default() },
                ResDisk { mount: "/x".into(), total: 0, ..Default::default() },
            ]),
            ..Default::default()
        }
    }

    #[test]
    fn received_cleans_fields_and_starts_history() {
        let r = sample(30).received(None, 1_000_000).unwrap();
        assert_eq!(r.at, 1_000_000);
        assert_eq!(r.cpu.as_ref().unwrap().model, "Apple M3");
        // 超过 100 的使用率按 100；剩余空间不超过总量；总量为 0 的盘不要
        assert_eq!(r.gpus.as_ref().unwrap()[0].pct, Some(100));
        assert_eq!(r.disks.as_ref().unwrap().len(), 1);
        assert_eq!(r.disks.as_ref().unwrap()[0].avail, 100);
        assert_eq!(r.hist, ResHist { cpu: vec![30], mem: vec![64], gpu: vec![100] });
    }

    #[test]
    fn history_continues_keeps_last_30_and_restarts_after_a_gap() {
        let mut r = sample(0).received(None, 1).unwrap();
        for i in 1..40u8 {
            r = sample(i).received(Some(&r), 1 + i as i64 * 10_000).unwrap();
        }
        assert_eq!(r.hist.cpu.len(), RES_HIST);
        assert_eq!(r.hist.cpu.first(), Some(&10));
        assert_eq!(r.hist.cpu.last(), Some(&39));
        // 离线了 5 分钟再回来：从头记
        let back = sample(50).received(Some(&r), r.at + 300_000).unwrap();
        assert_eq!(back.hist.cpu, vec![50]);
    }

    #[test]
    fn age_moves_sample_time_back_and_stale_samples_are_dropped() {
        let r = HostRes { age: Some(3_000), ..sample(10) }.received(None, 100_000).unwrap();
        assert_eq!(r.at, 97_000);
        assert_eq!(r.age, None);
        assert!(HostRes { age: Some(RES_STALE_MS + 1), ..sample(10) }.received(None, 1_000_000).is_none());
    }

    #[test]
    fn missing_parts_stay_missing() {
        let r = HostRes { cpu: None, gpus: None, disks: None, ..sample(1) }.received(None, 5).unwrap();
        assert!(r.cpu.is_none() && r.gpus.is_none() && r.disks.is_none());
        assert!(r.hist.cpu.is_empty() && r.hist.gpu.is_empty());
        // 没有 GPU（空列表）和取不到（None）分开
        let none = HostRes { gpus: Some(vec![]), ..sample(1) }.received(None, 5).unwrap();
        assert_eq!(none.gpus, Some(vec![]));
        let json = serde_json::to_value(&none).unwrap();
        assert!(json.get("age").is_none());
        assert_eq!(json["gpus"], serde_json::json!([]));
    }
}
