use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct ProcessGrants {
    paths: HashSet<PathBuf>,
}

impl ProcessGrants {
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths
                .into_iter()
                .filter(|p| p.is_absolute() && p.is_file())
                .filter_map(|p| p.canonicalize().ok())
                .collect(),
        }
    }

    pub fn from_env() -> Self {
        let paths = omsi_cfg::flags::OMSI_PLUGIN_PROCESS_ALLOW
            .live_os()
            .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
            .unwrap_or_default();
        Self::from_paths(paths)
    }

    pub fn allows(&self, path: &Path) -> bool {
        path.is_file()
            && path
                .canonicalize()
                .ok()
                .is_some_and(|p| self.paths.contains(&p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_existing_absolute_entry_files_are_granted() {
        let root = std::env::temp_dir().join(format!("omsi-process-grants-{}", std::process::id()));
        std::fs::create_dir_all(root.join("other")).unwrap();
        let entry = root.join("plugin.lua");
        let other = root.join("other/plugin.lua");
        std::fs::write(&entry, "").unwrap();
        std::fs::write(&other, "").unwrap();
        let grants = ProcessGrants::from_paths([
            entry.clone(),
            root.clone(),
            PathBuf::from("relative.lua"),
            root.join("missing.lua"),
        ]);
        assert!(grants.allows(&entry));
        assert!(!grants.allows(&other));
        assert!(!grants.allows(&root));
        assert!(!ProcessGrants::default().allows(&entry));
        std::fs::remove_file(&entry).unwrap();
        assert!(!grants.allows(&entry));
        std::fs::remove_file(other).unwrap();
        std::fs::remove_dir(root.join("other")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
