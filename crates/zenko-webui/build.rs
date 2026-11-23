use std::fs;
use std::path::PathBuf;

/// Ensure Cargo always recompiles `zenko-webui` and re-embeds fresh `rust-embed`
/// static assets whenever `webui/dist` is rebuilt — even if no `.rs` files changed.
fn main() {
    let dist_dir = PathBuf::from("../../webui/dist");
    println!("cargo:rerun-if-changed={}", dist_dir.display());

    let mut stack = vec![dist_dir];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            println!("cargo:rerun-if-changed={}", path.display());
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
}
