//! 子进程启动约定 —— 统一"静默"启动，避免控制台窗口乱闪。
//!
//! 背景（真机反馈，2026-10-29）：用 MSI 装好的应用在启动/自检时会疯狂闪命令行
//! 窗口。原因是 Windows GUI 程序（无控制台）启动控制台子进程（powershell /
//! nvidia-smi / vulkaninfo / moss / python / ffmpeg）时，系统会为**每个**子进程
//! 新建一个控制台窗口；窗口一闪而过，用户既看不清报错，观感也很糟。
//!
//! 解决办法就是给每个子进程加 `CREATE_NO_WINDOW`。注意：
//! - 关掉窗口**不影响**我们读输出——stdout/stderr 仍然走管道（`.output()` /
//!   `Stdio::piped()`），错误信息照旧回传到 UI；
//! - 关掉窗口也**不影响**子进程行为，它只是不再获得可见控制台。
//!
//! 因此项目内所有 `Command::new(...)` 都应改用 [`hidden`]，或至少套一层
//! [`quiet`]（已在 `scripts`/CI 之外全量替换，新增子进程时请沿用）。

use std::ffi::OsStr;
use std::process::Command;

/// `CREATE_NO_WINDOW`：不给子进程分配控制台窗口（Windows 专用标志）
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 让子进程不弹控制台窗口；返回同一个 `&mut Command` 便于链式调用。
///
/// 非 Windows 平台是空操作（Unix 上不存在这个概念）。
pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `Command::new(program)` 的静默版本 —— 项目内启动子进程请一律用它。
pub fn hidden<P: AsRef<OsStr>>(program: P) -> Command {
    let mut cmd = Command::new(program);
    quiet(&mut cmd);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 静默启动的子进程仍要能正常执行并拿到输出（CREATE_NO_WINDOW 不影响管道）
    #[cfg(windows)]
    #[test]
    fn hidden_process_still_reports_output() {
        let out = hidden("cmd")
            .args(["/C", "echo quiet-ok"])
            .output()
            .expect("无法启动 cmd");
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("quiet-ok"));
    }

    /// quiet() 可重复调用且不破坏链式写法
    #[test]
    fn quiet_is_chainable() {
        let mut cmd = Command::new("cmd");
        quiet(&mut cmd).arg("/C").arg("exit 0");
        assert_eq!(cmd.get_args().count(), 2);
    }
}
