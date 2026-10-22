use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[path = "candidates.rs"]
mod candidates;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessGrants {
    pub grants: Vec<ProcessGrant>,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) before_verify: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessGrant {
    pub entry: PathBuf,
    pub sha256: String,
    pub folder: PathBuf,
    pub files: BTreeMap<String, String>,
    pub helpers: BTreeMap<String, String>,
}

impl ProcessGrant {
    pub fn capture(entry: &Path, folder: &Path, helpers: &[String]) -> Result<Self, String> {
        if !entry.is_absolute() || !folder.is_absolute() {
            return Err("grant paths must be absolute".into());
        }
        let entry = entry.canonicalize().map_err(|e| e.to_string())?;
        let folder = folder.canonicalize().map_err(|e| e.to_string())?;
        let files = snapshot(&folder)?;
        let mut allowed = BTreeMap::new();
        for name in helpers {
            let name = super::super::paths::clean(name)?;
            let hash = files
                .get(&name)
                .ok_or("helper must be a file inside the plugin folder")?;
            #[cfg(windows)]
            if !name.to_ascii_lowercase().ends_with(".exe") {
                return Err("Windows helpers must be .exe files, not shell scripts".into());
            }
            allowed.insert(name, hash.clone());
        }
        Ok(Self {
            sha256: hash(&entry)?,
            entry,
            folder,
            files,
            helpers: allowed,
        })
    }

    fn matches(&self, entry: &Path, folder: Option<&Path>) -> bool {
        let Some(folder) = folder.or_else(|| entry.parent()) else {
            return false;
        };
        entry.canonicalize().ok().as_ref() == Some(&self.entry)
            && folder.canonicalize().ok().as_ref() == Some(&self.folder)
            && hash(entry).is_ok_and(|h| h == self.sha256)
            && snapshot(&self.folder).is_ok_and(|files| files == self.files)
    }
}

impl ProcessGrants {
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            grants: paths
                .into_iter()
                .filter_map(|entry| ProcessGrant::capture(&entry, entry.parent()?, &[]).ok())
                .collect(),
            ..Default::default()
        }
    }

    pub fn from_env() -> Self {
        let result = (|| {
            let path = PathBuf::from(omsi_cfg::flags::OMSI_PLUGIN_PROCESS_ALLOW.live_os()?);
            if !path.is_absolute() || std::fs::metadata(&path).ok()?.len() > 1024 * 1024 {
                return None;
            }
            let bytes = std::fs::read(path).ok()?;
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)).ok()
        })();
        result.unwrap_or_default()
    }

    pub(crate) fn has_candidate(&self, entry: &Path, folder: Option<&Path>) -> bool {
        fn lexical(path: &Path) -> Option<PathBuf> {
            let absolute = std::path::absolute(path).ok()?;
            #[cfg(windows)]
            {
                let text = absolute.to_str()?;
                return Some(PathBuf::from(
                    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
                        format!(r"\\{unc}")
                    } else {
                        text.strip_prefix(r"\\?\").unwrap_or(text).to_owned()
                    },
                ));
            }
            #[cfg(not(windows))]
            Some(absolute)
        }
        let Some(folder) = folder.or_else(|| entry.parent()) else {
            return false;
        };
        let (Some(entry), Some(folder)) = (lexical(entry), lexical(folder)) else {
            return false;
        };
        self.grants.iter().any(|g| {
            lexical(&g.entry).is_some_and(|p| candidates::matches(&p, &entry))
                && lexical(&g.folder).is_some_and(|p| candidates::matches(&p, &folder))
        })
    }

    pub fn allows(&self, entry: &Path, folder: Option<&Path>) -> bool {
        self.grants.iter().any(|g| g.matches(entry, folder))
    }

    pub fn helper(
        &self,
        entry: &Path,
        folder: Option<&Path>,
        file: &str,
    ) -> Result<PathBuf, String> {
        let grant = self.grants.iter().find(|g| g.matches(entry, folder))
            .ok_or("plugin content changed or no hash-bound process grant exists; approve the new content")?;
        let clean = super::super::paths::clean(file)?;
        let expected = grant
            .helpers
            .get(&clean)
            .ok_or("helper is not listed in the player's grant")?;
        let path = super::super::paths::inside(&grant.folder, &clean)?;
        if !path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&grant.folder)
            || hash(&path)? != *expected
        {
            return Err("helper changed or escaped its plugin folder".into());
        }
        #[cfg(windows)]
        if !clean.to_ascii_lowercase().ends_with(".exe") {
            return Err("Windows helpers must be .exe files".into());
        }
        Ok(path)
    }
}

fn hash(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("not a regular file".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn snapshot(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    let mut count = 0;
    let mut total = 0;
    while let Some(dir) = pending.pop() {
        for item in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let meta = std::fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
            count += 1;
            total += meta.len();
            if count > 4096 || total > 256 * 1024 * 1024 {
                return Err("plugin grant exceeds 4096 entries or 256 MiB".into());
            }
            if meta.file_type().is_symlink() || (!meta.is_file() && !meta.is_dir()) {
                return Err("plugin grants cannot include symlinks or special files".into());
            }
            if !item
                .path()
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(root)
            {
                return Err("plugin folder contains a path escaping its root".into());
            }
            if meta.is_dir() {
                pending.push(item.path());
            } else {
                let path = item.path();
                let name = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_str()
                    .ok_or("plugin paths must be UTF-8")?
                    .replace('\\', "/");
                files.insert(name, hash(&path)?);
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, PathBuf, ProcessGrants) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("omsi-grants-{}-{}-{unique}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(root.join("modules")).unwrap();
        let entry = root.join("plugin.lua");
        std::fs::write(&entry, "return 1").unwrap();
        std::fs::write(root.join("modules/code.lua"), "return 2").unwrap();
        std::fs::write(root.join("helper.exe"), "approved helper").unwrap();
        let grant = ProcessGrant::capture(&entry, &root, &["helper.exe".into()]).unwrap();
        (
            root,
            entry,
            ProcessGrants {
                grants: vec![grant],
                ..Default::default()
            },
        )
    }
    #[test]
    fn digest_is_sha256() {
        let (root, entry, _) = fixture();
        std::fs::write(&entry, "abc").unwrap();
        assert_eq!(
            hash(&entry).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn manifest_roundtrip_preserves_exact_helper_consent() {
        let (root, entry, grants) = fixture();
        let json = serde_json::to_vec(&grants).unwrap();
        let grants: ProcessGrants = serde_json::from_slice(&json).unwrap();
        assert!(grants.allows(&entry, Some(&root)));
        assert!(grants.helper(&entry, Some(&root), "helper.exe").is_ok());
        for path in ["../helper.exe", "/bin/sh", "modules/code.lua"] {
            assert!(grants.helper(&entry, Some(&root), path).is_err());
        }
        assert!(
            grants
                .helper(
                    &entry,
                    Some(&root),
                    root.join("helper.exe").to_str().unwrap()
                )
                .is_err()
        );
        assert!(serde_json::from_str::<ProcessGrants>(r#"{"paths":["plugin.lua"]}"#).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn code_modules_helpers_and_added_files_revoke_saved_grants() {
        for name in ["plugin.lua", "modules/code.lua", "helper.exe", "new.lua"] {
            let (root, entry, grants) = fixture();
            std::fs::write(root.join(name), "replaced code").unwrap();
            assert!(!grants.allows(&entry, Some(&root)), "{name}");
            assert!(grants.helper(&entry, Some(&root), "helper.exe").is_err());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn removed_module_and_different_folder_revoke_consent() {
        let (root, entry, grants) = fixture();
        assert!(!grants.allows(&entry, Some(&root.join("modules"))));
        std::fs::remove_file(root.join("modules/code.lua")).unwrap();
        assert!(!grants.allows(&entry, Some(&root)));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn short_paths_reach_worker_verification_but_do_not_grant_access() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetShortPathNameW(long: *const u16, short: *mut u16, size: u32) -> u32;
        }
        let (root, entry, grants) = fixture();
        let long: Vec<u16> = entry.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut short = vec![0; 32768];
        // SAFETY: both buffers are valid for the supplied lengths and long is NUL terminated.
        let n = unsafe { GetShortPathNameW(long.as_ptr(), short.as_mut_ptr(), short.len() as u32) };
        assert!(n > 0 && (n as usize) < short.len());
        let short = PathBuf::from(std::ffi::OsString::from_wide(&short[..n as usize]));
        assert!(grants.has_candidate(&short, short.parent()));
        assert!(grants.helper(&short, short.parent(), "helper.exe").is_ok());
        // The lexical DOS hint deliberately admits ambiguous aliases. Even identical
        // bytes at a different canonical path must still be rejected by the worker.
        let other = root.with_extension("alias~1");
        std::fs::create_dir(&other).unwrap();
        let other_entry = other.join("plugin.lua");
        std::fs::copy(&entry, &other_entry).unwrap();
        assert!(grants.has_candidate(&other_entry, Some(&other)));
        assert!(grants.helper(&other_entry, Some(&other), "helper.exe").is_err());
        std::fs::write(&entry, "modified plugin").unwrap();
        assert!(grants.has_candidate(&short, short.parent()));
        assert!(grants.helper(&short, short.parent(), "helper.exe").is_err());
        std::fs::remove_dir_all(other).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_code_cannot_receive_consent() {
        let (root, entry, _) = fixture();
        std::os::unix::fs::symlink(&entry, root.join("alias.lua")).unwrap();
        assert!(ProcessGrant::capture(&entry, &root, &[]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
