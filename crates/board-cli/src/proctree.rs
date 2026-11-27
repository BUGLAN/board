//! 整棵进程树的回收：Windows 用 Job Object，其他系统是空实现（仍按进程组结束）。
//!
//! Windows 上把 `board acp`（以及它启动的 agent）放进 Job Object，并设
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`：停止时可以直接 `TerminateJobObject`，进程树中间有谁先退出都不影响；
//! board host 崩溃、Job 句柄随进程关闭时，系统也会结束里面所有还活着的进程，不会留下孙进程。

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// 一个进程树的 Job Object；最后一个句柄关闭时，系统结束里面所有进程
    pub struct Job(HANDLE);

    impl Job {
        /// 建 Job、设“最后一个句柄关闭就结束整棵进程树”，再把刚启动的子进程放进去
        pub fn attach(child: &Child) -> io::Result<Job> {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
                if SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    size,
                ) == 0
                {
                    let e = io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(e);
                }
                if AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
                    let e = io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(e);
                }
                Ok(Job(job))
            }
        }

        /// 结束 Job 里的所有进程（包括已经脱离父进程、但仍在 Job 里的孙进程）
        pub fn terminate(&self, _hard: bool) {
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::process::Child;

    /// 其他系统继续按进程组结束进程树，这里不额外持有句柄
    pub struct Job(u32);

    impl Job {
        pub fn attach(child: &Child) -> io::Result<Job> {
            Ok(Job(child.id()))
        }

        pub fn terminate(&self, hard: bool) {
            crate::envs::kill_tree(self.0, hard);
        }
    }
}

pub use imp::Job;
