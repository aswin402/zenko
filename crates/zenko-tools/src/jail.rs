use std::path::{Component, Path, PathBuf};
use zenko_core::{
    ARTIFACT_PATH_PREFIX, ARTIFACTS_DIR_NAME, HUB_PATH_PREFIX, Result, ZenkoError, expand_tilde,
};

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

const MAX_SYMLINK_HOPS: usize = 32;

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
                if hops > MAX_SYMLINK_HOPS {
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
}
