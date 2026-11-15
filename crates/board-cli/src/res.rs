//! board host 的资源采样（PRD §10.12「资源」）：后台线程每 10 秒采一次 CPU、内存、GPU、磁盘，上报时带上最近一次。
//! 放在后台是因为读磁盘可能卡住（网络盘没响应）：卡住时上报照常，带的是越来越旧的采样，太旧了服务端不收。
//! GPU 用 sysinfo：Linux 读 NVIDIA 驱动（NVML）、amdgpu 和 Vulkan，Windows 读系统的性能计数器，macOS 读 IOKit；
//! 取不到就不报（界面写「不支持」），找过、没有 GPU 报空列表
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use board_core::hostres::{HostRes, ResCpu, ResDisk, ResGpu, ResMem};
use sysinfo::{Disks, Gpus, System};

const EVERY: Duration = Duration::from_secs(10);
const GIB: u64 = 1 << 30;
/// 最多报几块盘（工作空间所在的那块总在里面）
const MAX_DISKS: usize = 6;

/// 不是本机的盘：内存盘、容器层、只读镜像、系统伪文件系统
const SKIP_FS: &[&str] = &[
    "tmpfs",
    "devtmpfs",
    "overlay",
    "overlayfs",
    "squashfs",
    "ramfs",
    "proc",
    "sysfs",
    "cgroup",
    "cgroup2",
    "autofs",
    "nsfs",
    "efivarfs",
    "devfs",
    "nullfs",
    "iso9660",
    "udf",
    "tracefs",
    "debugfs",
    "securityfs",
    "pstore",
    "bpf",
    "configfs",
    "fusectl",
    "mqueue",
    "hugetlbfs",
];
/// 系统自己的卷、启动分区、snap、docker
const SKIP_UNDER: &[&str] = &["/System/Volumes/", "/private/var/vm", "/boot", "/snap/", "/dev", "/proc", "/sys", "/run", "/var/lib/docker"];

/// 后台采样：`start` 起线程，`latest` 取最近一次（带上采样之后过了多久）
#[derive(Clone)]
pub struct Sampler {
    last: Arc<Mutex<Option<(HostRes, Instant)>>>,
    ws: Arc<Mutex<PathBuf>>,
}

impl Sampler {
    /// 起后台线程（board host 主循环开始前调一次）。最多等 1 秒第一次采样，第一次上报就能带上；
    /// 采样卡住（读网络盘没响应）也不多等
    pub fn start(ws: PathBuf) -> Sampler {
        let s = Sampler { last: Arc::new(Mutex::new(None)), ws: Arc::new(Mutex::new(ws)) };
        let me = s.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut probe = Probe::new();
            loop {
                let ws = me.ws.lock().map(|w| w.clone()).unwrap_or_default();
                let r = probe.sample(&ws);
                if let Ok(mut l) = me.last.lock() {
                    *l = Some((r, Instant::now()));
                }
                let _ = tx.send(());
                std::thread::sleep(EVERY);
            }
        });
        let _ = rx.recv_timeout(Duration::from_secs(1));
        s
    }

    /// 工作空间换了：下一次采样按它认工作空间所在的盘
    pub fn set_ws(&self, ws: PathBuf) {
        if let Ok(mut w) = self.ws.lock() {
            if *w != ws {
                *w = ws;
            }
        }
    }

    /// 最近一次采样；还没采到时为空
    pub fn latest(&self) -> Option<HostRes> {
        let l = self.last.lock().ok()?;
        let (r, at) = l.as_ref()?;
        Some(HostRes { age: Some(at.elapsed().as_millis() as i64), ..r.clone() })
    }
}

/// 取不到的（这个系统上 sysinfo 不支持）为空，对应的那一项不报
struct Probe {
    sys: Option<System>,
    disks: Option<Disks>,
    gpus: Option<Gpus>,
    cpu_at: Instant,
}

impl Probe {
    fn new() -> Probe {
        let mut sys = System::new().ok();
        // CPU 使用率要两次刷新之间的差：先刷一次，第一次采样前等够最短间隔
        if let Some(s) = sys.as_mut() {
            s.refresh_cpu_all();
        }
        Probe { sys, disks: Disks::new().ok(), gpus: Gpus::new().ok(), cpu_at: Instant::now() }
    }

    fn sample(&mut self, ws: &Path) -> HostRes {
        let (mut cpu, mut mem) = (None, None);
        if let Some(sys) = self.sys.as_mut() {
            let wait = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.saturating_sub(self.cpu_at.elapsed());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            sys.refresh_cpu_usage();
            self.cpu_at = Instant::now();
            sys.refresh_memory();
            let cpus = sys.cpus();
            cpu = (!cpus.is_empty()).then(|| ResCpu {
                pct: sys.global_cpu_usage().round().clamp(0.0, 100.0) as u8,
                cores: cpus.len() as u32,
                // Windows 没有平均负载
                load: if cfg!(windows) { None } else { System::load_average().ok().map(|l| [l.one, l.five, l.fifteen]) },
                model: cpus[0].brand().trim().to_string(),
            });
            mem = (sys.total_memory() > 0).then(|| ResMem { total: sys.total_memory(), used: sys.used_memory() });
        }
        let gpus = self.gpus.as_mut().map(|g| {
            g.refresh(true);
            g.iter()
                .map(|x| ResGpu {
                    name: x.model().unwrap_or_default().trim().to_string(),
                    vendor: x.vendor().unwrap_or_default().trim().to_string(),
                    pct: x.usage().filter(|u| u.is_finite()).map(|u| u.round().clamp(0.0, 100.0) as u8),
                    vram_used: x.used_memory(),
                    vram_total: x.total_memory().filter(|t| *t > 0),
                })
                .collect::<Vec<_>>()
        });
        let disks = self.disks.as_mut().map(|disks| {
            disks.refresh(true);
            let raw: Vec<RawDisk> = disks
                .iter()
                .map(|d| RawDisk {
                    mount: d.mount_point().to_string_lossy().into_owned(),
                    label: d.name().to_string_lossy().into_owned(),
                    fs: d.file_system().to_string_lossy().into_owned(),
                    total: d.total_space(),
                    avail: d.available_space(),
                    removable: d.is_removable(),
                })
                .collect();
            pick_disks(raw, &ws.to_string_lossy(), cfg!(target_os = "macos"), cfg!(windows))
        });
        HostRes { cpu, mem, gpus, disks, ..Default::default() }
    }
}

/// sysinfo 列出来的一块盘
#[derive(Debug, Clone)]
pub struct RawDisk {
    pub mount: String,
    pub label: String,
    pub fs: String,
    pub total: u64,
    pub avail: u64,
    pub removable: bool,
}

/// 统一成小写、正斜杠、去掉末尾的斜杠（Windows 的盘符不分大小写）
fn norm(p: &str, windows: bool) -> String {
    let p = if windows { p.replace('\\', "/").to_lowercase() } else { p.to_string() };
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        "/".into()
    } else {
        t.to_string()
    }
}

fn under(path: &str, mount: &str) -> bool {
    mount == "/" || path == mount || path.starts_with(&format!("{mount}/"))
}

/// 挑出要报的盘：工作空间所在的（挂载点是工作空间路径最长的前缀）排第一，其余按挂载点排；不要伪文件系统、
/// 系统卷、可移动盘、小于 1 GB 的；同一块盘挂了几处（btrfs 子卷、绑定挂载）只留一处。卷名只在 macOS 上留着（「Macintosh HD」），
/// Linux 上是设备名、Windows 上常常是空的，界面上只写挂载点
pub fn pick_disks(raw: Vec<RawDisk>, ws: &str, keep_label: bool, windows: bool) -> Vec<ResDisk> {
    let ws = if ws.trim().is_empty() { String::new() } else { norm(ws, windows) };
    let home =
        raw.iter().filter(|d| !ws.is_empty() && under(&ws, &norm(&d.mount, windows))).max_by_key(|d| norm(&d.mount, windows).len()).map(|d| d.mount.clone());
    let mut out: Vec<(RawDisk, bool)> = vec![];
    for d in raw {
        let is_ws = home.as_deref() == Some(d.mount.as_str());
        let m = norm(&d.mount, windows);
        let fs = d.fs.to_lowercase();
        let skip = d.total < GIB
            || SKIP_FS.contains(&fs.as_str())
            || (!windows
                && SKIP_UNDER
                    .iter()
                    .any(|x| m == x.trim_end_matches('/') || m.starts_with(x.trim_end_matches('/')) && m[x.trim_end_matches('/').len()..].starts_with('/')))
            || d.removable
            || (fs.starts_with("fuse.") && !fs.starts_with("fuse.sshfs"));
        if skip && !is_ws {
            continue;
        }
        // 同一块盘挂了几处：总量和剩余都一样、卷名一样就算一块，留工作空间那处，否则留挂载点短的
        if let Some(i) = out.iter().position(|(o, _)| o.total == d.total && o.avail == d.avail && o.label == d.label) {
            if is_ws || (!out[i].1 && norm(&out[i].0.mount, windows).len() > m.len()) {
                out[i] = (d, is_ws);
            }
            continue;
        }
        out.push((d, is_ws));
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| norm(&a.0.mount, windows).cmp(&norm(&b.0.mount, windows))));
    out.truncate(MAX_DISKS);
    out.into_iter()
        .map(|(d, ws)| ResDisk {
            label: if keep_label { d.label.trim().to_string() } else { String::new() },
            mount: d.mount,
            total: d.total,
            avail: d.avail.min(d.total),
            ws,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(mount: &str, fs: &str, total_gb: u64, avail_gb: u64) -> RawDisk {
        RawDisk { mount: mount.into(), label: String::new(), fs: fs.into(), total: total_gb * GIB, avail: avail_gb * GIB, removable: false }
    }

    #[test]
    fn linux_keeps_real_disks_with_workspace_first() {
        let raw = vec![
            d("/", "ext4", 196, 77),
            d("/boot/efi", "vfat", 1, 1),
            d("/boot", "ext4", 2, 1),
            d("/run", "tmpfs", 13, 13),
            d("/snap/core22/1380", "squashfs", 1, 0),
            d("/data", "xfs", 1863, 412),
            d("/var/lib/docker/overlay2/x/merged", "overlay", 196, 77),
        ];
        let out = pick_disks(raw, "/data/repo", false, false);
        let mounts: Vec<&str> = out.iter().map(|x| x.mount.as_str()).collect();
        assert_eq!(mounts, vec!["/data", "/"]);
        assert!(out[0].ws && !out[1].ws);
    }

    #[test]
    fn macos_keeps_root_with_label_and_drops_system_volumes() {
        let mut root = d("/", "apfs", 994, 382);
        root.label = "Macintosh HD".into();
        let mut data = d("/System/Volumes/Data", "apfs", 994, 382);
        data.label = "Macintosh HD - Data".into();
        let mut usb = d("/Volumes/USB", "exfat", 64, 10);
        usb.removable = true;
        let out = pick_disks(vec![root, data, d("/System/Volumes/VM", "apfs", 994, 382), usb], "~/repo".replace('~', "/Users/me").as_str(), true, false);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].mount.as_str(), out[0].label.as_str(), out[0].ws), ("/", "Macintosh HD", true));
    }

    #[test]
    fn windows_matches_drive_letter_case_insensitively() {
        let raw = vec![d("C:\\", "NTFS", 476, 33), d("D:\\", "NTFS", 931, 519), d("E:\\", "FAT32", 0, 0)];
        let out = pick_disks(raw, "d:\\Repo", false, true);
        let mounts: Vec<&str> = out.iter().map(|x| x.mount.as_str()).collect();
        assert_eq!(mounts, vec!["D:\\", "C:\\"]);
        assert!(out[0].ws);
    }

    #[test]
    fn same_disk_mounted_twice_counts_once_and_workspace_wins() {
        // btrfs 子卷：/ 和 /home 是同一块盘
        let raw = vec![d("/", "btrfs", 500, 200), d("/home", "btrfs", 500, 200)];
        let out = pick_disks(raw.clone(), "/home/me/repo", false, false);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].mount.as_str(), out[0].ws), ("/home", true));
        // 工作空间不在上面：留挂载点短的
        let out = pick_disks(raw, "", false, false);
        assert_eq!((out[0].mount.as_str(), out[0].ws), ("/", false));
    }

    #[test]
    fn workspace_on_a_disk_that_would_be_skipped_is_still_reported() {
        let mut usb = d("/Volumes/Work", "apfs", 2000, 900);
        usb.removable = true;
        let out = pick_disks(vec![d("/", "apfs", 994, 382), usb], "/Volumes/Work/repo", false, false);
        assert_eq!(out[0].mount, "/Volumes/Work");
        assert!(out[0].ws);
    }

    /// 在这台机器上真采一次，看看数值（手动跑：cargo test -p board res::tests::real_sample -- --ignored --nocapture）
    #[test]
    #[ignore]
    fn real_sample() {
        let mut p = Probe::new();
        let home = std::env::var("HOME").unwrap_or_default();
        let r = p.sample(Path::new(&home));
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
    }
}
