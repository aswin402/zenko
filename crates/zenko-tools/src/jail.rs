use std::path::{Component, Path, PathBuf};
use zenko_core::{
    ARTIFACT_PATH_PREFIX, ARTIFACTS_DIR_NAME, HUB_PATH_PREFIX, JAIL_DIR_CREATE_MODE,
    JAIL_MAX_SYMLINK_HOPS, JAIL_NEW_FILE_MODE, JAIL_TMP_FILE_MODE, JAIL_TMP_FILE_PREFIX, Result,
    ZenkoError, expand_tilde,
};

/// Formats a resolved file path cleanly for tool output: workspace-relative when under `ws_root`,
/// `hub:`-prefixed when under `hub_root`, else normalized fallback (zero duplicated `if/else`).
pub fn format_display_rel_path(
    path: &Path,
    ws_root: &Path,
    hub_root: &Path,
    fallback: &str,
) -> String {
    match (path.strip_prefix(ws_root), path.strip_prefix(hub_root)) {
        (Ok(rel), _) => rel.to_string_lossy().replace('\\', "/"),
        (Err(_), Ok(rel)) => {
            format!("{HUB_PATH_PREFIX}{}", rel.to_string_lossy().replace('\\', "/"))
        }
        (Err(_), Err(_)) => fallback.to_string(),
    }
}

/// Resolves a read-only path across three jailed roots:
/// - `artifact:<session>/<file>` strictly inside `<data_dir>/artifacts/` (read-only spilled outputs)
/// - `hub:<subpath>` strictly inside `hub_root` (`~/zenko_hub/`)
/// - Everything else strictly inside `workspace_root`
///
/// Write tools MUST continue calling `resolve_workspace_or_hub_path` so spilled artifacts stay immutable.
pub fn resolve_readable_path(
    workspace_root: &Path,
    hub_root: &Path,
    data_dir: &Path,
    requested: &str,
    must_exist: bool,
) -> Result<PathBuf> {
    let trimmed = requested.trim();
    if let Some(sub) = trimmed.strip_prefix(ARTIFACT_PATH_PREFIX) {
        let rel = sub.trim_start_matches(['/', '\\']);
        let target = match rel.is_empty() {
            true => ".",
            false => rel,
        };
        let artifacts_root = data_dir.join(ARTIFACTS_DIR_NAME);
        return resolve_jailed_path(&artifacts_root, target, must_exist);
    }
    resolve_workspace_or_hub_path(workspace_root, hub_root, requested, must_exist)
}

/// Resolves a path strictly inside either `workspace_root` OR `hub_root` (`~/zenko_hub/`):
/// - `hub:<subpath>` explicitly targets `hub_root`
/// - Absolute or `~/` paths pointing inside `hub_root` resolve within `hub_root`
/// - All other paths resolve strictly inside `workspace_root`
///
/// Both roots enforce identical lexical + canonical symlink-escape jails.
pub fn resolve_workspace_or_hub_path(
    workspace_root: &Path,
    hub_root: &Path,
    requested: &str,
    must_exist: bool,
) -> Result<PathBuf> {
    let trimmed = requested.trim();
    if let Some(sub) = trimmed.strip_prefix(HUB_PATH_PREFIX) {
        let rel = sub.trim_start_matches(['/', '\\']);
        let target = match rel.is_empty() {
            true => ".",
            false => rel,
        };
        return resolve_jailed_path(hub_root, target, must_exist);
    }

    let expanded = expand_tilde(trimmed);
    if expanded.is_absolute() {
        let norm_req = normalize_lexical(&expanded);
        let norm_hub = normalize_lexical(
            &hub_root
                .canonicalize()
                .unwrap_or_else(|_| hub_root.to_path_buf()),
        );
        let raw_hub = normalize_lexical(hub_root);
        let rel_inside_hub = norm_req
            .strip_prefix(&norm_hub)
            .ok()
            .or_else(|| norm_req.strip_prefix(&raw_hub).ok());
        if let Some(rel_path) = rel_inside_hub {
            let rel_str = rel_path.to_string_lossy();
            let target = match rel_str.is_empty() {
                true => ".",
                false => rel_str.as_ref(),
            };
            return resolve_jailed_path(hub_root, target, must_exist);
        }
    }

    let expanded_str = expanded.to_string_lossy();
    let target = match expanded.is_absolute() {
        true => expanded_str.as_ref(),
        false => trimmed,
    };
    resolve_jailed_path(workspace_root, target, must_exist)
}

/// Ensures a requested path stays strictly inside the target root,
/// blocking `..` traversal AND both live and dangling symlink escapes.
pub fn resolve_jailed_path(workspace_root: &Path, requested: &str, must_exist: bool) -> Result<PathBuf> {
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| normalize_lexical(workspace_root));

    let req_path = Path::new(requested);
    let candidate = if req_path.is_absolute() {
        req_path.to_path_buf()
    } else {
        root.join(req_path)
    };

    // Lexical normalization check first
    let normalized = normalize_lexical(&candidate);
    if !normalized.starts_with(&root) {
        return Err(ZenkoError::PolicyDenied(format!(
            "path '{}' escapes workspace root '{}'",
            requested,
            root.display()
        )));
    }

    // Component-by-component symlink-aware walk (catches dangling symlinks even when
    // `Path::exists()` returns `false`).
    let mut pending: std::collections::VecDeque<std::ffi::OsString> = normalized
        .strip_prefix(&root)
        .map_err(|_| {
            ZenkoError::PolicyDenied(format!(
                "path '{}' escapes workspace root '{}'",
                requested,
                root.display()
            ))
        })?
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();

    let mut cur = root.clone();
    let mut hops = 0usize;

    while let Some(comp) = pending.pop_front() {
        let next = cur.join(&comp);
        match std::fs::symlink_metadata(&next) {
            Ok(meta) if meta.file_type().is_symlink() => {
                hops += 1;
                if hops > JAIL_MAX_SYMLINK_HOPS {
                    return Err(ZenkoError::PolicyDenied(format!(
                        "symlink loop while resolving '{}'",
                        requested
                    )));
                }
                let link_target = std::fs::read_link(&next).map_err(|e| {
                    ZenkoError::Tool(format!("cannot read symlink '{}': {e}", requested))
                })?;
                let resolved_link = match link_target.is_absolute() {
                    true => normalize_lexical(&link_target),
                    false => normalize_lexical(&cur.join(&link_target)),
                };
                if !resolved_link.starts_with(&root) {
                    return Err(ZenkoError::PolicyDenied(format!(
                        "symlink '{}' escapes workspace root",
                        requested
                    )));
                }
                // Re-walk resolved link target components from `root` before remaining components
                let target_comps: Vec<std::ffi::OsString> = resolved_link
                    .strip_prefix(&root)
                    .map_err(|_| {
                        ZenkoError::PolicyDenied(format!(
                            "symlink '{}' escapes workspace root",
                            requested
                        ))
                    })?
                    .components()
                    .filter_map(|c| match c {
                        Component::Normal(s) => Some(s.to_os_string()),
                        _ => None,
                    })
                    .collect();
                for tc in target_comps.into_iter().rev() {
                    pending.push_front(tc);
                }
                cur = root.clone();
            }
            Ok(_) => {
                cur = next;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if must_exist {
                    return Err(ZenkoError::Tool(format!("cannot access '{}': {e}", requested)));
                }
                cur = next;
                while let Some(rem) = pending.pop_front() {
                    cur.push(rem);
                }
                break;
            }
            Err(e) => {
                return Err(ZenkoError::Tool(format!("cannot access '{}': {e}", requested)));
            }
        }
    }

    if must_exist && !cur.exists() {
        return Err(ZenkoError::Tool(format!(
            "cannot access '{}': path does not exist",
            requested
        )));
    }
    if !cur.starts_with(&root) {
        return Err(ZenkoError::PolicyDenied(format!(
            "path '{}' escapes workspace root '{}'",
            requested,
            root.display()
        )));
    }

    Ok(cur)
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

/// Resolves which confined root (`workspace_root`, `hub_root`, or optional `<data_dir>/artifacts`)
/// contains a previously resolved `abs_path`.
pub fn select_jail_root(
    abs_path: &Path,
    workspace_root: &Path,
    hub_root: &Path,
    data_dir: Option<&Path>,
) -> Option<PathBuf> {
    let candidates = [
        Some(workspace_root.to_path_buf()),
        Some(hub_root.to_path_buf()),
        data_dir.map(|d| d.join(ARTIFACTS_DIR_NAME)),
    ];
    for root in candidates.into_iter().flatten() {
        let canon_root = root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(&root));
        if abs_path.starts_with(&canon_root) {
            return Some(canon_root);
        }
    }
    None
}

fn split_confined_relative<'a>(
    canon_root: &Path,
    abs_path: &'a Path,
) -> Result<(&'a Path, &'a std::ffi::OsStr)> {
    let rel = abs_path.strip_prefix(canon_root).map_err(|_| {
        ZenkoError::PolicyDenied(format!(
            "path '{}' escapes confined root '{}'",
            abs_path.display(),
            canon_root.display()
        ))
    })?;
    let file_name = rel.file_name().ok_or_else(|| {
        ZenkoError::PolicyDenied(format!(
            "path '{}' does not point to a file inside '{}'",
            abs_path.display(),
            canon_root.display()
        ))
    })?;
    let parent_rel = rel.parent().unwrap_or_else(|| Path::new(""));
    Ok((parent_rel, file_name))
}

#[cfg(unix)]
mod unix_confined {
    use super::*;
    use std::ffi::CString;
    use std::io::Write as _;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};

    struct OwnedRawFd(libc::c_int);

    impl OwnedRawFd {
        fn into_raw(self) -> libc::c_int {
            let md = std::mem::ManuallyDrop::new(self);
            md.0
        }
    }

    impl Drop for OwnedRawFd {
        fn drop(&mut self) {
            // SAFETY: OwnedRawFd uniquely owns a valid open Unix file descriptor.
            unsafe {
                libc::close(self.0);
            }
        }
    }

    fn os_to_cstring(s: &std::ffi::OsStr) -> Result<CString> {
        CString::new(s.as_bytes())
            .map_err(|_| ZenkoError::PolicyDenied("path component contains interior NUL byte".into()))
    }

    fn map_open_err(err: std::io::Error, label: &str) -> ZenkoError {
        match err.raw_os_error() {
            Some(code) if code == libc::ELOOP => ZenkoError::PolicyDenied(format!(
                "kernel O_NOFOLLOW blocked symlink traversal on '{label}'"
            )),
            Some(code) if code == libc::ENOTDIR => ZenkoError::PolicyDenied(format!(
                "non-directory path segment encountered while traversing '{label}'"
            )),
            _ => ZenkoError::Tool(format!("cannot access '{label}': {err}")),
        }
    }

    fn open_dir_confined(
        canon_root: &Path,
        rel_dir: &Path,
        create_missing: bool,
    ) -> Result<OwnedRawFd> {
        let root_c = os_to_cstring(canon_root.as_os_str())?;
        // SAFETY: root_c is a valid NUL-terminated path; flags open a directory fd with CLOEXEC.
        let root_fd = unsafe {
            libc::open(
                root_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if root_fd < 0 {
            return Err(map_open_err(
                std::io::Error::last_os_error(),
                &canon_root.display().to_string(),
            ));
        }
        let mut cur = OwnedRawFd(root_fd);

        for comp in rel_dir.components() {
            let seg = match comp {
                Component::CurDir => continue,
                Component::Normal(s) => s,
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(ZenkoError::PolicyDenied(format!(
                        "illegal relative segment in confined path '{}'",
                        rel_dir.display()
                    )));
                }
            };
            let seg_c = os_to_cstring(seg)?;
            let seg_label = seg.to_string_lossy();
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            // SAFETY: cur.0 is a valid directory fd and seg_c is a valid single path component.
            let raw_fd = unsafe { libc::openat(cur.0, seg_c.as_ptr(), flags) };
            let next_fd = match (raw_fd >= 0, create_missing) {
                (true, _) => raw_fd,
                (false, false) => {
                    return Err(map_open_err(std::io::Error::last_os_error(), &seg_label));
                }
                (false, true) => {
                    let err = std::io::Error::last_os_error();
                    match err.raw_os_error() {
                        Some(code) if code == libc::ENOENT => {
                            // SAFETY: cur.0 is a confined dir fd; mkdirat creates child dir only.
                            let mk = unsafe {
                                libc::mkdirat(cur.0, seg_c.as_ptr(), JAIL_DIR_CREATE_MODE as libc::mode_t)
                            };
                            if mk < 0 {
                                let mk_err = std::io::Error::last_os_error();
                                if mk_err.raw_os_error() != Some(libc::EEXIST) {
                                    return Err(map_open_err(mk_err, &seg_label));
                                }
                            }
                            // SAFETY: Re-open newly created segment strictly with O_NOFOLLOW | O_DIRECTORY.
                            let reopened = unsafe { libc::openat(cur.0, seg_c.as_ptr(), flags) };
                            match reopened >= 0 {
                                true => reopened,
                                false => {
                                    return Err(map_open_err(
                                        std::io::Error::last_os_error(),
                                        &seg_label,
                                    ));
                                }
                            }
                        }
                        _ => return Err(map_open_err(err, &seg_label)),
                    }
                }
            };
            cur = OwnedRawFd(next_fd);
        }
        Ok(cur)
    }

    pub fn open_jailed_read_file(root: &Path, abs_path: &Path) -> Result<std::fs::File> {
        let canon_root = root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(root));
        let (parent_rel, file_name) = split_confined_relative(&canon_root, abs_path)?;
        let parent_fd = open_dir_confined(&canon_root, parent_rel, false)?;
        let name_c = os_to_cstring(file_name)?;
        let name_label = file_name.to_string_lossy();
        // SAFETY: parent_fd.0 is a confined directory fd; O_NOFOLLOW blocks leaf symlink races.
        let fd = unsafe {
            libc::openat(
                parent_fd.0,
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(map_open_err(std::io::Error::last_os_error(), &name_label));
        }
        let owned = OwnedRawFd(fd);
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: owned.0 is a valid open file descriptor and st is a writable stat struct.
        if unsafe { libc::fstat(owned.0, &mut st) } < 0 {
            return Err(map_open_err(std::io::Error::last_os_error(), &name_label));
        }
        if (st.st_mode & libc::S_IFMT) != libc::S_IFREG {
            return Err(ZenkoError::PolicyDenied(format!(
                "refused to read non-regular file '{name_label}'"
            )));
        }
        // SAFETY: ownership of fd transfers cleanly to std::fs::File via into_raw().
        Ok(unsafe { std::fs::File::from_raw_fd(owned.into_raw()) })
    }

    pub fn atomic_write_jailed(root: &Path, abs_path: &Path, content: &[u8]) -> Result<()> {
        let canon_root = root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(root));
        let (parent_rel, file_name) = split_confined_relative(&canon_root, abs_path)?;
        let parent_fd = open_dir_confined(&canon_root, parent_rel, true)?;
        let name_c = os_to_cstring(file_name)?;
        let name_label = file_name.to_string_lossy();

        // Reject overwriting a symlink at the destination inside the pinned parent directory,
        // and capture any existing regular file's permission bits so edits preserve `+x` / `0644`.
        let mut existing_st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: parent_fd.0 is a confined directory fd; AT_SYMLINK_NOFOLLOW inspects leaf directly.
        let existing_rc = unsafe {
            libc::fstatat(
                parent_fd.0,
                name_c.as_ptr(),
                &mut existing_st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if existing_rc == 0 && (existing_st.st_mode & libc::S_IFMT) == libc::S_IFLNK {
            return Err(ZenkoError::PolicyDenied(format!(
                "kernel AT_SYMLINK_NOFOLLOW blocked atomic write over symlink '{name_label}'"
            )));
        }
        let target_mode: libc::mode_t = match existing_rc == 0 {
            true => (existing_st.st_mode & 0o777) as libc::mode_t,
            false => JAIL_NEW_FILE_MODE as libc::mode_t,
        };

        let tmp_name = format!("{JAIL_TMP_FILE_PREFIX}{}", zenko_core::new_id());
        let tmp_c = CString::new(tmp_name.as_bytes())
            .map_err(|_| ZenkoError::Tool("invalid temp filename".into()))?;
        // SAFETY: parent_fd.0 is a confined directory fd; O_CREAT|O_EXCL|O_NOFOLLOW creates a fresh 0600 file.
        let tmp_fd = unsafe {
            libc::openat(
                parent_fd.0,
                tmp_c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                JAIL_TMP_FILE_MODE as libc::c_uint,
            )
        };
        if tmp_fd < 0 {
            return Err(map_open_err(std::io::Error::last_os_error(), &tmp_name));
        }

        // SAFETY: tmp_fd is uniquely owned and transfers into std::fs::File.
        let mut file = unsafe { std::fs::File::from_raw_fd(tmp_fd) };
        let write_res = file.write_all(content).and_then(|_| file.flush());
        if write_res.is_ok() {
            // SAFETY: fchmod operates directly on the owned open fd before renameat (TOCTOU-immune).
            let _ = unsafe { libc::fchmod(file.as_raw_fd(), target_mode) };
        }
        drop(file);
        if let Err(err) = write_res {
            // SAFETY: clean up temporary file inside the same confined directory fd.
            unsafe {
                libc::unlinkat(parent_fd.0, tmp_c.as_ptr(), 0);
            }
            return Err(err.into());
        }

        // SAFETY: atomic rename within the pinned parent directory fd (immune to parent symlink swaps).
        if unsafe { libc::renameat(parent_fd.0, tmp_c.as_ptr(), parent_fd.0, name_c.as_ptr()) } < 0
        {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::unlinkat(parent_fd.0, tmp_c.as_ptr(), 0);
            }
            return Err(map_open_err(err, &name_label));
        }
        Ok(())
    }

    pub fn remove_jailed_file(root: &Path, abs_path: &Path) -> Result<()> {
        let canon_root = root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(root));
        let (parent_rel, file_name) = split_confined_relative(&canon_root, abs_path)?;
        let parent_fd = open_dir_confined(&canon_root, parent_rel, false)?;
        let name_c = os_to_cstring(file_name)?;
        let name_label = file_name.to_string_lossy();

        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: parent_fd.0 is a confined directory fd; AT_SYMLINK_NOFOLLOW checks leaf without deref.
        if unsafe {
            libc::fstatat(
                parent_fd.0,
                name_c.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } < 0
        {
            return Err(map_open_err(std::io::Error::last_os_error(), &name_label));
        }
        if (st.st_mode & libc::S_IFMT) != libc::S_IFREG {
            return Err(ZenkoError::PolicyDenied(format!(
                "refused to delete non-regular file '{name_label}'"
            )));
        }
        // SAFETY: unlink strictly inside the pinned confined directory fd.
        if unsafe { libc::unlinkat(parent_fd.0, name_c.as_ptr(), 0) } < 0 {
            return Err(map_open_err(std::io::Error::last_os_error(), &name_label));
        }
        Ok(())
    }
}

#[cfg(not(unix))]
mod unix_confined {
    use super::*;

    pub fn open_jailed_read_file(root: &Path, abs_path: &Path) -> Result<std::fs::File> {
        let canon_root = root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(root));
        let _ = split_confined_relative(&canon_root, abs_path)?;
        let meta = std::fs::symlink_metadata(abs_path)?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(ZenkoError::PolicyDenied(format!(
                "refused to read non-regular or symlink target '{}'",
                abs_path.display()
            )));
        }
        Ok(std::fs::File::open(abs_path)?)
    }

    pub fn atomic_write_jailed(root: &Path, abs_path: &Path, content: &[u8]) -> Result<()> {
        let canon_root = root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(root));
        let _ = split_confined_relative(&canon_root, abs_path)?;
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Ok(meta) = std::fs::symlink_metadata(abs_path)
            && meta.file_type().is_symlink()
        {
            return Err(ZenkoError::PolicyDenied(format!(
                "refused to overwrite symlink '{}'",
                abs_path.display()
            )));
        }
        let tmp = abs_path.with_extension(format!("zenko-tmp-{}", zenko_core::new_id()));
        if let Err(err) = std::fs::write(&tmp, content) {
            let _ = std::fs::remove_file(&tmp);
            return Err(err.into());
        }
        if let Err(err) = std::fs::rename(&tmp, abs_path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(err.into());
        }
        Ok(())
    }

    pub fn remove_jailed_file(root: &Path, abs_path: &Path) -> Result<()> {
        let canon_root = root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(root));
        let _ = split_confined_relative(&canon_root, abs_path)?;
        let meta = std::fs::symlink_metadata(abs_path)?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(ZenkoError::PolicyDenied(format!(
                "refused to delete non-regular or symlink target '{}'",
                abs_path.display()
            )));
        }
        std::fs::remove_file(abs_path)?;
        Ok(())
    }
}

/// Opens a regular file inside `root` using kernel-enforced directory-FD traversal (`openat` +
/// `O_NOFOLLOW | O_CLOEXEC` on Unix), blocking concurrent TOCTOU symlink-swap escapes.
pub fn open_jailed_read_file(root: &Path, abs_path: &Path) -> Result<std::fs::File> {
    unix_confined::open_jailed_read_file(root, abs_path)
}

/// Reads a bounded UTF-8 file strictly through a kernel-confined `openat(O_NOFOLLOW)` handle.
pub fn read_jailed_string_bounded(
    root: &Path,
    abs_path: &Path,
    max_bytes: u64,
    label: &str,
) -> Result<String> {
    use std::io::Read as _;
    let file = open_jailed_read_file(root, abs_path)?;
    let len = file.metadata()?.len();
    if len > max_bytes {
        return Err(ZenkoError::Tool(format!(
            "File '{label}' ({len} bytes) exceeds safety limit ({max_bytes} bytes)."
        )));
    }
    let mut buf = Vec::with_capacity(len.min(max_bytes) as usize);
    file.take(max_bytes.saturating_add(1)).read_to_end(&mut buf)?;
    if buf.len() as u64 > max_bytes {
        return Err(ZenkoError::Tool(format!(
            "File '{label}' grew past safety limit ({max_bytes} bytes) while reading."
        )));
    }
    String::from_utf8(buf).map_err(|_| {
        ZenkoError::Tool(format!("File '{label}' is not valid UTF-8 text."))
    })
}

/// Atomically writes `content` to `abs_path` strictly inside `root` via pinned parent `dir_fd`,
/// `O_CREAT | O_EXCL | O_NOFOLLOW` temp file, and `renameat`.
pub fn atomic_write_jailed(root: &Path, abs_path: &Path, content: &[u8]) -> Result<()> {
    unix_confined::atomic_write_jailed(root, abs_path, content)
}

/// Deletes a regular file strictly inside `root` via pinned parent `dir_fd` + `unlinkat`.
pub fn remove_jailed_file(root: &Path, abs_path: &Path) -> Result<()> {
    unix_confined::remove_jailed_file(root, abs_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_parent_traversal() {
        let dir = std::env::temp_dir().join(format!("zenko_jail_{}", zenko_core::new_id()));
        let hub = std::env::temp_dir().join(format!("zenko_hub_jail_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::create_dir_all(&hub).expect("mkdir hub");
        let res = resolve_jailed_path(&dir, "../../etc/passwd", false);
        assert!(res.is_err());
        let ok = resolve_jailed_path(&dir, "sub/file.txt", false);
        assert!(ok.is_ok());

        let hub_ok = resolve_workspace_or_hub_path(&dir, &hub, "hub:scripts/check.sh", false);
        assert!(hub_ok.is_ok());
        assert!(hub_ok.expect("hub path").ends_with("scripts/check.sh"));

        let hub_escape =
            resolve_workspace_or_hub_path(&dir, &hub, "hub:../../etc/passwd", false);
        assert!(hub_escape.is_err());

        let data = std::env::temp_dir().join(format!("zenko_data_jail_{}", zenko_core::new_id()));
        let art_dir = data.join(ARTIFACTS_DIR_NAME).join("s1");
        std::fs::create_dir_all(&art_dir).expect("mkdir art");
        std::fs::write(art_dir.join("c1.txt"), "hello").expect("write art");
        let art_ok = resolve_readable_path(&dir, &hub, &data, "artifact:s1/c1.txt", true);
        assert!(art_ok.is_ok());
        assert!(resolve_readable_path(&dir, &hub, &data, "artifact:../../etc/passwd", false).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside_missing = std::env::temp_dir().join(format!(
                "zenko_outside_missing_{}/pwn.txt",
                zenko_core::new_id()
            ));
            let dangling = dir.join("dangling_escape.txt");
            symlink(&outside_missing, &dangling).expect("dangling symlink");
            assert!(
                resolve_jailed_path(&dir, "dangling_escape.txt", false).is_err(),
                "dangling symlink pointing outside root must be rejected even for non-existent writes"
            );
            let dangling_dir = dir.join("dangling_dir");
            symlink(outside_missing.parent().expect("parent"), &dangling_dir).expect("dangling dir");
            assert!(
                resolve_jailed_path(&dir, "dangling_dir/sub/file.txt", false).is_err(),
                "dangling directory symlink pointing outside root must be rejected"
            );
        }

        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(hub);
        let _ = std::fs::remove_dir_all(data);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_jailed_uses_0644_for_new_files_and_preserves_existing_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("zenko_jail_perm_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");

        // 1. New project file gets readable 0644 (never locked down to 0600).
        let html = dir.join("index.html");
        atomic_write_jailed(&dir, &html, b"<h1>Zenko</h1>").expect("write new file");
        let new_mode = std::fs::metadata(&html).expect("meta").permissions().mode() & 0o777;
        assert_eq!(new_mode, zenko_core::JAIL_NEW_FILE_MODE);

        // 2. Existing executable 0755 script keeps its executable bit on overwrite.
        let script = dir.join("run.sh");
        std::fs::write(&script, "#!/bin/sh\necho old\n").expect("seed script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod 755");
        atomic_write_jailed(&dir, &script, b"#!/bin/sh\necho new\n").expect("overwrite script");
        let preserved = std::fs::metadata(&script).expect("meta").permissions().mode() & 0o777;
        assert_eq!(preserved, 0o755);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn kernel_confined_openat_writes_reads_deletes_and_blocks_symlinks() {
        let dir = std::env::temp_dir().join(format!("zenko_openat_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");

        let target = dir.join("nested/sub/hello.txt");
        atomic_write_jailed(&dir, &target, b"safe-content\n").expect("atomic write");
        let read_back = read_jailed_string_bounded(&dir, &target, 4096, "hello.txt").expect("read");
        assert_eq!(read_back, "safe-content\n");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = std::env::temp_dir().join(format!("zenko_ext_{}.txt", zenko_core::new_id()));
            std::fs::write(&outside, "secret").expect("write outside");
            let sym_leaf = dir.join("link.txt");
            symlink(&outside, &sym_leaf).expect("symlink leaf");

            // Kernel O_NOFOLLOW must block reading, overwriting, and deleting a symlink leaf!
            assert!(
                open_jailed_read_file(&dir, &sym_leaf).is_err(),
                "O_NOFOLLOW must block reading symlink leaf"
            );
            assert!(
                atomic_write_jailed(&dir, &sym_leaf, b"pwn").is_err(),
                "AT_SYMLINK_NOFOLLOW must block overwriting symlink leaf"
            );
            assert!(
                remove_jailed_file(&dir, &sym_leaf).is_err(),
                "AT_SYMLINK_NOFOLLOW must refuse deleting non-regular symlink"
            );
            assert_eq!(std::fs::read_to_string(&outside).expect("outside intact"), "secret");
            let _ = std::fs::remove_file(&outside);
        }

        remove_jailed_file(&dir, &target).expect("remove regular");
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
