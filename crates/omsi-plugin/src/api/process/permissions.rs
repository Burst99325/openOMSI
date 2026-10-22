//! Host-owned process consent. Plugins can request execution but cannot answer prompts.
use super::{ProcessGrant, ProcessGrants, worker::StartRequest};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessDecision {
    AllowOnce = 1,
    AlwaysAllow = 2,
    Deny = 3,
}

#[derive(Clone, Debug)]
pub struct ProcessPrompt {
    pub id: u64,
    pub plugin: String,
    pub entry: PathBuf,
    pub file: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}
struct Pending {
    prompt: ProcessPrompt,
    decision: Weak<AtomicU8>,
    grant: ProcessGrant,
}
#[derive(Default)]
struct Saved {
    loaded: bool,
    grants: Vec<ProcessGrant>,
}

/// One game's permission broker. `path` belongs to the host's user settings, never a
/// plugin directory. A host that does not install this broker remains default-deny.
pub struct ProcessPermissions {
    path: Option<PathBuf>,
    saved: Mutex<Saved>,
    denied: Mutex<Vec<ProcessGrant>>,
    pending: Mutex<VecDeque<Pending>>,
    next: AtomicU64,
}
pub(super) enum Authorization {
    Granted(ProcessGrant),
    Pending(ProcessGrant, Arc<AtomicU8>),
}
impl ProcessPermissions {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            saved: Mutex::new(Saved::default()),
            denied: Mutex::new(Vec::new()),
            pending: Mutex::new(VecDeque::new()),
            next: AtomicU64::new(1),
        }
    }
    /// Nonblocking, host-only view of the oldest live request.
    pub fn prompt(&self) -> Option<ProcessPrompt> {
        let mut pending = self.pending.try_lock().ok()?;
        pending.retain(|p| {
            p.decision
                .upgrade()
                .is_some_and(|d| d.load(Ordering::Acquire) == 0)
        });
        pending.front().map(|p| p.prompt.clone())
    }
    /// Cheap input-routing check; it never clones paths or command arguments.
    pub fn has_pending(&self) -> bool {
        self.pending.try_lock().is_ok_and(|pending| {
            pending.iter().any(|p| {
                p.decision
                    .upgrade()
                    .is_some_and(|d| d.load(Ordering::Acquire) == 0)
            })
        })
    }
    /// A stale click cannot approve the next request. No files are accessed on this path.
    pub fn respond(&self, id: u64, choice: ProcessDecision) -> bool {
        let Ok(pending) = self.pending.try_lock() else {
            return false;
        };
        pending
            .iter()
            .find(|p| p.prompt.id == id)
            .and_then(|p| p.decision.upgrade())
            .is_some_and(|d| {
                d.compare_exchange(0, choice as u8, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            })
    }
    pub(super) fn authorize(&self, request: &StartRequest) -> Result<Authorization, String> {
        let folder = request
            .folder
            .as_deref()
            .or_else(|| request.entry.parent())
            .ok_or("no plugin folder")?;
        let clean = super::super::paths::clean(&request.file)?;
        let grant = ProcessGrant::capture(&request.entry, folder, &[clean.clone()])?;
        // Capture validates the folder and helper type. Resolve the helper again to reject
        // entry/helper changes between snapshot and publishing the request.
        let file = ProcessGrants {
            grants: vec![grant.clone()],
            ..Default::default()
        }
        .helper(&request.entry, request.folder.as_deref(), &clean)?;
        if self.denied.lock().unwrap().contains(&grant) {
            return Err("native process request denied by the player for this session".into());
        }
        let mut saved = self.saved.lock().unwrap();
        if !saved.loaded {
            saved.loaded = true;
            saved.grants = self
                .path
                .as_ref()
                .and_then(|p| {
                    (std::fs::metadata(p).ok()?.len() <= 1024 * 1024).then_some(())?;
                    serde_json::from_slice::<ProcessGrants>(&std::fs::read(p).ok()?).ok()
                })
                .unwrap_or_default()
                .grants;
        }
        if saved.grants.contains(&grant) {
            return Ok(Authorization::Granted(grant));
        }
        drop(saved);
        let decision = Arc::new(AtomicU8::new(0));
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|p| {
            p.decision
                .upgrade()
                .is_some_and(|d| d.load(Ordering::Acquire) == 0)
        });
        if pending.len() >= 32 {
            return Err("too many pending process permission requests (limit 32)".into());
        }
        pending.push_back(Pending {
            prompt: ProcessPrompt {
                id: self.next.fetch_add(1, Ordering::Relaxed),
                plugin: request.plugin.clone(),
                entry: grant.entry.clone(),
                file,
                args: request.args.clone(),
                cwd: request.cwd.clone(),
            },
            decision: Arc::downgrade(&decision),
            grant: grant.clone(),
        });
        Ok(Authorization::Pending(grant, decision))
    }
    pub(super) fn deny(&self, grant: ProcessGrant) {
        let mut denied = self.denied.lock().unwrap();
        if !denied.contains(&grant) && denied.len() < 256 {
            denied.push(grant.clone());
        }
        drop(denied);
        for pending in self
            .pending
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.grant == grant)
        {
            if let Some(d) = pending.decision.upgrade() {
                let _ = d.compare_exchange(
                    0,
                    ProcessDecision::Deny as u8,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
    }
    /// Called only by a preparation worker, after freshly verifying the approved snapshot.
    pub(super) fn remember(&self, grant: ProcessGrant) -> Result<(), String> {
        let path = self
            .path
            .as_ref()
            .ok_or("cannot save process permission: no user settings directory")?;
        let mut saved = self.saved.lock().unwrap();
        let mut grants = saved.grants.clone();
        grants.retain(|g| {
            !(g.entry == grant.entry
                && g.folder == grant.folder
                && g.helpers.keys().eq(grant.helpers.keys()))
        });
        grants.push(grant.clone());
        if grants.len() > 256 {
            return Err("too many saved process permissions (limit 256)".into());
        }
        let bytes = serde_json::to_vec_pretty(&ProcessGrants {
            grants: grants.clone(),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("saved process permissions exceed 1 MiB".into());
        }
        let parent = path.parent().ok_or("invalid process permissions path")?;
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot save process permission: {e}"))?;
        let temp = path.with_extension(format!("{}.new", std::process::id()));
        std::fs::write(&temp, bytes)
            .and_then(|_| std::fs::rename(&temp, path))
            .map_err(|e| format!("cannot save process permission: {e}"))?;
        saved.grants = grants;
        // Always allow also covers requests already queued for exactly this snapshot/file.
        for pending in self
            .pending
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.grant == grant)
        {
            if let Some(d) = pending.decision.upgrade() {
                let _ = d.compare_exchange(
                    0,
                    ProcessDecision::AllowOnce as u8,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
        Ok(())
    }
}
