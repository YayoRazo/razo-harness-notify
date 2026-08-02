use crate::events::CanonicalEvent;
use std::path::Path;

pub mod antigravity;
pub mod claude_code;
pub mod kimi;
pub mod opencode;
pub mod unsupported;

pub trait HookAdapter {
    fn id(&self) -> &'static str;
    fn install(&self, base_dir: &Path, binary_path: &Path) -> Result<(), String>;
    fn uninstall(&self, base_dir: &Path) -> Result<(), String>;
}

pub const HOOK_MAP: [(&str, CanonicalEvent); 3] = [
    ("Stop", CanonicalEvent::Done),
    ("Notification", CanonicalEvent::Attention),
    ("SubagentStop", CanonicalEvent::SubagentDone),
];

pub fn marker_for(harness: &str) -> String {
    format!("notify --harness {harness} --event")
}

pub fn check_marker() -> &'static str {
    "check --hook session-start"
}

pub fn backup_before_write(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    // Preserve the original extension in the backup name (foo.json -> foo.json.bak).
    // An extensionless target must not double the suffix (foo -> foo.bak, not foo.bak.bak).
    let backup = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => path.with_extension(format!("{ext}.bak")),
        None => path.with_extension("bak"),
    };
    std::fs::copy(path, backup).map(|_| ())
}

/// Write content to a temp sibling file, then atomically rename it over the
/// target. This closes the TOCTOU window between reading and writing: another
/// process (e.g. the harness itself) writing the same config between our read
/// and our write cannot cause silent data loss - the worst case is one writer
/// wins, but the file is never left in a partially-written or empty state.
pub fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("tmp");
    let tmp = path.with_file_name(format!(".{stem}.{ext}.tmp"));
    std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn all_adapters() -> Vec<Box<dyn HookAdapter>> {
    let mut v: Vec<Box<dyn HookAdapter>> = vec![
        Box::new(claude_code::ClaudeCodeAdapter),
        Box::new(opencode::OpencodeAdapter),
        Box::new(antigravity::AntigravityAdapter),
        Box::new(kimi::KimiAdapter),
    ];
    for id in ["kilo", "kiro", "cursor", "windsurf", "cline", "copilot"] {
        v.push(Box::new(unsupported::UnsupportedAdapter { harness_id: id }));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn backup_before_write_copies_an_existing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{\"a\":1}").unwrap();
        backup_before_write(&path).unwrap();
        let backup = path.with_extension("json.bak");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "{\"a\":1}");
    }

    #[test]
    fn backup_before_write_does_not_double_the_suffix_for_an_extensionless_target() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("hooks");
        std::fs::write(&path, "data").unwrap();
        backup_before_write(&path).unwrap();
        assert!(dir.path().join("hooks.bak").exists());
        assert!(!dir.path().join("hooks.bak.bak").exists());
    }

    #[test]
    fn backup_before_write_is_a_no_op_when_file_does_not_exist() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(backup_before_write(&path).is_ok());
        assert!(!path.with_extension("json.bak").exists());
    }

    #[test]
    fn registry_has_every_adapter() {
        let ids: Vec<&str> = all_adapters().iter().map(|a| a.id()).collect();
        assert_eq!(
            ids,
            vec!["claude-code", "opencode", "antigravity", "kimi", "kilo", "kiro", "cursor", "windsurf", "cline", "copilot"]
        );
    }
}
