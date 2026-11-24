//! OS-native Linux Landlock filesystem write sandbox for spawned shell & PTY processes.
//! - Zero external crates (direct Linux ABI syscalls via `libc`, already in `Cargo.lock`).
//! - Zero root privileges, zero Docker overhead, inherited by the entire child process tree.
//! - Pre-allocates all C-strings before `fork()` so the `pre_exec` hook is 100% async-signal-safe.
//! - Gracefully degrades on kernels without Landlock (`ENOSYS` / `EOPNOTSUPP`) or non-Linux hosts.

use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct ShellSandboxPolicy {
    pub enabled: bool,
    pub writable_dirs: Vec<PathBuf>,
    pub writable_files: Vec<PathBuf>,
}

impl ShellSandboxPolicy {
    /// Builds a workspace-confined write policy allowing host toolchain reads while confining
    /// filesystem writes strictly to `workspace_dir`, `hub_dir`, `/tmp`/`$TMPDIR`, `$CARGO_HOME`,
    /// and standard device sinks (`/dev/null`, `/dev/tty`, optional PTY slave path).
    pub fn for_workspace(
        enabled: bool,
        workspace_dir: &Path,
        hub_dir: Option<&Path>,
        extra_writable_files: &[PathBuf],
    ) -> Self {
        let mut writable_dirs: Vec<PathBuf> = Vec::with_capacity(6);
        let mut push_dir = |p: PathBuf| {
            let canon = p.canonicalize().unwrap_or(p);
            if canon.is_dir() && !writable_dirs.iter().any(|existing| existing == &canon) {
                writable_dirs.push(canon);
            }
        };

        push_dir(workspace_dir.to_path_buf());
        if let Some(hub) = hub_dir {
            push_dir(hub.to_path_buf());
        }
        push_dir(PathBuf::from("/tmp"));
        if let Ok(tmp) = std::env::var("TMPDIR")
            && !tmp.trim().is_empty()
        {
            push_dir(PathBuf::from(tmp));
        }
        match std::env::var("CARGO_HOME")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            Some(cargo_home) => push_dir(PathBuf::from(cargo_home)),
            None => {
                if let Ok(home) = std::env::var("HOME")
                    && !home.trim().is_empty()
                {
                    let default_cargo = PathBuf::from(home).join(".cargo");
                    if default_cargo.is_dir() {
                        push_dir(default_cargo);
                    }
                }
            }
        }

        let mut writable_files: Vec<PathBuf> = Vec::with_capacity(4 + extra_writable_files.len());
        let mut push_file = |p: PathBuf| {
            if p.exists() && !writable_files.iter().any(|existing| existing == &p) {
                writable_files.push(p);
            }
        };
        push_file(PathBuf::from("/dev/null"));
        push_file(PathBuf::from("/dev/tty"));
        for f in extra_writable_files {
            push_file(f.clone());
        }

        Self {
            enabled,
            writable_dirs,
            writable_files,
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_landlock {
    use super::ShellSandboxPolicy;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use tokio::process::Command;

    const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;
    const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

    const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
    const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
    const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
    const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
    const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
    const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
    const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
    const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
    const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
    const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
    const LANDLOCK_ACCESS_FS_REFER: u64 = 1 << 13;
    const LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;

    const ABI_V1_WRITE_RIGHTS: u64 = LANDLOCK_ACCESS_FS_WRITE_FILE
        | LANDLOCK_ACCESS_FS_REMOVE_DIR
        | LANDLOCK_ACCESS_FS_REMOVE_FILE
        | LANDLOCK_ACCESS_FS_MAKE_CHAR
        | LANDLOCK_ACCESS_FS_MAKE_DIR
        | LANDLOCK_ACCESS_FS_MAKE_REG
        | LANDLOCK_ACCESS_FS_MAKE_SOCK
        | LANDLOCK_ACCESS_FS_MAKE_FIFO
        | LANDLOCK_ACCESS_FS_MAKE_BLOCK
        | LANDLOCK_ACCESS_FS_MAKE_SYM;

    #[repr(C)]
    struct LandlockRulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C, packed)]
    struct LandlockPathBeneathAttr {
        allowed_access: u64,
        parent_fd: libc::c_int,
    }

    pub fn probe_abi_version() -> Option<i32> {
        let ver = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<libc::c_void>(),
                0usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        match ver >= 1 {
            true => Some(ver as i32),
            false => None,
        }
    }

    /// Pure raw-syscall Landlock installation safe to execute inside `pre_exec` after `fork()`.
    /// Performs zero heap allocations and degrades cleanly if the host kernel lacks Landlock.
    pub fn apply_signal_safe(writable_dirs: &[CString], writable_files: &[CString]) {
        let Some(abi) = probe_abi_version() else {
            return;
        };

        let mut handled_dir_rights = ABI_V1_WRITE_RIGHTS;
        let mut handled_file_rights = LANDLOCK_ACCESS_FS_WRITE_FILE;
        if abi >= 2 {
            handled_dir_rights |= LANDLOCK_ACCESS_FS_REFER;
        }
        if abi >= 3 {
            handled_dir_rights |= LANDLOCK_ACCESS_FS_TRUNCATE;
            handled_file_rights |= LANDLOCK_ACCESS_FS_TRUNCATE;
        }

        let attr = LandlockRulesetAttr {
            handled_access_fs: handled_dir_rights,
        };
        let ruleset_fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const LandlockRulesetAttr,
                std::mem::size_of::<LandlockRulesetAttr>(),
                0u32,
            )
        } as libc::c_int;
        if ruleset_fd < 0 {
            return;
        }

        for dir in writable_dirs {
            let fd = unsafe {
                libc::open(
                    dir.as_ptr(),
                    libc::O_PATH | libc::O_CLOEXEC | libc::O_DIRECTORY,
                )
            };
            if fd >= 0 {
                let rule = LandlockPathBeneathAttr {
                    allowed_access: handled_dir_rights,
                    parent_fd: fd,
                };
                unsafe {
                    let _ = libc::syscall(
                        libc::SYS_landlock_add_rule,
                        ruleset_fd,
                        LANDLOCK_RULE_PATH_BENEATH,
                        &rule as *const LandlockPathBeneathAttr,
                        0u32,
                    );
                    libc::close(fd);
                }
            }
        }

        for file in writable_files {
            let fd = unsafe { libc::open(file.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
            if fd >= 0 {
                let rule = LandlockPathBeneathAttr {
                    allowed_access: handled_file_rights,
                    parent_fd: fd,
                };
                unsafe {
                    let _ = libc::syscall(
                        libc::SYS_landlock_add_rule,
                        ruleset_fd,
                        LANDLOCK_RULE_PATH_BENEATH,
                        &rule as *const LandlockPathBeneathAttr,
                        0u32,
                    );
                    libc::close(fd);
                }
            }
        }

        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) == 0 {
                let _ = libc::syscall(libc::SYS_landlock_restrict_self, ruleset_fd, 0u32);
            }
            libc::close(ruleset_fd);
        }
    }

    pub fn prepare_cstrings(paths: &[std::path::PathBuf]) -> Vec<CString> {
        paths
            .iter()
            .filter_map(|p| CString::new(p.as_os_str().as_bytes()).ok())
            .collect()
    }

    pub fn install_on_tokio_command(cmd: &mut Command, policy: &ShellSandboxPolicy) {
        if !policy.enabled || probe_abi_version().is_none() {
            return;
        }
        let dirs = prepare_cstrings(&policy.writable_dirs);
        let files = prepare_cstrings(&policy.writable_files);
        unsafe {
            cmd.pre_exec(move || {
                apply_signal_safe(&dirs, &files);
                Ok(())
            });
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod fallback_sandbox {
    use super::ShellSandboxPolicy;
    use tokio::process::Command;

    pub fn probe_abi_version() -> Option<i32> {
        None
    }

    pub fn install_on_tokio_command(_cmd: &mut Command, _policy: &ShellSandboxPolicy) {}
}

/// Probes the host Linux Landlock ABI version (returns `Some(version >= 1)` when supported).
pub fn probe_landlock_abi() -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        linux_landlock::probe_abi_version()
    }
    #[cfg(not(target_os = "linux"))]
    {
        fallback_sandbox::probe_abi_version()
    }
}

/// Installs OS-native Landlock filesystem write isolation into `cmd.pre_exec` (async-signal-safe).
pub fn install_sandbox_on_command(cmd: &mut Command, policy: &ShellSandboxPolicy) {
    #[cfg(target_os = "linux")]
    {
        linux_landlock::install_on_tokio_command(cmd, policy);
    }
    #[cfg(not(target_os = "linux"))]
    {
        fallback_sandbox::install_on_tokio_command(cmd, policy);
    }
}

/// Prepares signal-safe CStrings and exposes direct post-fork Landlock installation for PTY spawns.
#[cfg(target_os = "linux")]
pub(crate) fn prepare_landlock_cstrings(
    policy: &ShellSandboxPolicy,
) -> Option<(Vec<std::ffi::CString>, Vec<std::ffi::CString>)> {
    if !policy.enabled || linux_landlock::probe_abi_version().is_none() {
        return None;
    }
    Some((
        linux_landlock::prepare_cstrings(&policy.writable_dirs),
        linux_landlock::prepare_cstrings(&policy.writable_files),
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn apply_prepared_landlock_signal_safe(
    dirs: &[std::ffi::CString],
    files: &[std::ffi::CString],
) {
    linux_landlock::apply_signal_safe(dirs, files);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn landlock_confines_writes_to_workspace_when_kernel_supports_it() {
        let Some(abi) = probe_landlock_abi() else {
            return;
        };
        assert!(abi >= 1);

        let root = std::env::temp_dir().join(format!("zenko_ll_{}", zenko_core::new_id()));
        let ws = root.join("workspace");
        let outside = root.join("outside");
        std::fs::create_dir_all(&ws).expect("ws");
        std::fs::create_dir_all(&outside).expect("outside");

        // Use a targeted policy that ONLY grants write access to `ws` (not `/tmp`) so we can
        // prove the Linux kernel itself blocks writing to sibling `outside/` with EACCES!
        let policy = ShellSandboxPolicy {
            enabled: true,
            writable_dirs: vec![ws.canonicalize().expect("canon")],
            writable_files: vec![PathBuf::from("/dev/null")],
        };

        let mut ok_cmd = Command::new("/bin/sh");
        ok_cmd
            .arg("-c")
            .arg("echo inside > inside.txt")
            .current_dir(&ws);
        install_sandbox_on_command(&mut ok_cmd, &policy);
        let ok_status = ok_cmd.status().await.expect("ok status");
        assert!(ok_status.success(), "write inside workspace must succeed");
        assert_eq!(
            std::fs::read_to_string(ws.join("inside.txt")).expect("read"),
            "inside\n"
        );

        let blocked_target = outside.join("escaped.txt");
        let mut deny_cmd = Command::new("/bin/sh");
        deny_cmd
            .arg("-c")
            .arg(format!("echo nope > '{}'", blocked_target.display()))
            .current_dir(&ws);
        install_sandbox_on_command(&mut deny_cmd, &policy);
        let deny_status = deny_cmd.status().await.expect("deny status");
        assert!(
            !deny_status.success(),
            "Linux Landlock kernel sandbox MUST block write outside allowed roots"
        );
        assert!(
            !blocked_target.exists(),
            "escaped file must never be created"
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
