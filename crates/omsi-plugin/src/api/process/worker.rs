use super::{Child, ProcessGrants, Status};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, SyncSender},
};
use std::time::Duration;

const WORKERS: usize = 2;
const QUEUED: usize = 32;
type Prepare = Box<dyn FnOnce(&AtomicBool) -> Result<Child, String> + Send>;
struct Job {
    prepare: Prepare,
    control: Arc<Control>,
}
struct Finished {
    result: Result<Child, String>,
    control: Arc<Control>,
}
#[derive(Default)]
struct Control {
    cancel: AtomicBool,
    release: AtomicBool,
    status: Mutex<Status>,
}

pub(super) struct Pending {
    control: Arc<Control>,
    cached: Status,
}
impl Pending {
    pub fn status(&mut self) -> Status {
        if let Ok(status) = self.control.status.try_lock() {
            self.cached = status.clone();
        }
        self.cached.clone()
    }
    pub fn kill(&mut self) {
        self.control.cancel.store(true, Ordering::Release);
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.control.cancel.store(true, Ordering::Release);
        self.control.release.store(true, Ordering::Release);
    }
}

pub(crate) struct StartRequest {
    pub grants: Arc<ProcessGrants>,
    pub entry: PathBuf,
    pub folder: Option<PathBuf>,
    pub file: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}
impl StartRequest {
    pub(super) fn prepare(self, cancel: &AtomicBool) -> Result<Child, String> {
        #[cfg(test)]
        if let Some(before) = &self.grants.before_verify {
            before();
        }
        if cancel.load(Ordering::Acquire) {
            return Err("process start cancelled".into());
        }
        let helper = self
            .grants
            .helper(&self.entry, self.folder.as_deref(), &self.file)?;
        if cancel.load(Ordering::Acquire) {
            return Err("process start cancelled".into());
        }
        super::spawn(
            helper.to_str().ok_or("helper path is not UTF-8")?,
            &self.args,
            self.cwd.as_deref(),
            &self.env,
        )
    }
}

pub(super) fn submit(prepare: Prepare) -> Result<Pending, String> {
    static POOL: OnceLock<Result<SyncSender<Job>, String>> = OnceLock::new();
    let pool = POOL.get_or_init(pool).as_ref().map_err(Clone::clone)?;
    let status = Status {
        starting: true,
        ..Default::default()
    };
    let control = Arc::new(Control {
        status: Mutex::new(status.clone()),
        ..Default::default()
    });
    pool.try_send(Job {
        prepare,
        control: control.clone(),
    })
    .map_err(|_| "process preparation queue is full or unavailable")?;
    Ok(Pending {
        control,
        cached: status,
    })
}

fn pool() -> Result<SyncSender<Job>, String> {
    let (send, receive) = mpsc::sync_channel::<Job>(QUEUED);
    let receive = Arc::new(Mutex::new(receive));
    let (finished, results) = mpsc::sync_channel::<Finished>(WORKERS);
    std::thread::Builder::new()
        .name("plugin-process-owner".into())
        .spawn(move || own(results))
        .map_err(|e| e.to_string())?;
    for index in 0..WORKERS {
        let receive = receive.clone();
        let finished = finished.clone();
        std::thread::Builder::new()
            .name(format!("plugin-process-prepare-{index}"))
            .spawn(move || {
                loop {
                    let Ok(job) = receive.lock().unwrap().recv() else {
                        break;
                    };
                    let result = if job.control.cancel.load(Ordering::Acquire) {
                        Err("process start cancelled".into())
                    } else {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            (job.prepare)(&job.control.cancel)
                        }))
                        .unwrap_or_else(|_| Err("process preparation worker panicked".into()))
                    };
                    if let Err(error) = finished.send(Finished {
                        result,
                        control: job.control,
                    }) {
                        if let Ok(mut child) = error.0.result {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(send)
}

fn own(results: mpsc::Receiver<Finished>) {
    let mut children: Vec<(Child, Arc<Control>)> = Vec::new();
    loop {
        match results.recv_timeout(Duration::from_millis(5)) {
            Ok(done) => match done.result {
                Ok(mut child) => {
                    if done.control.cancel.load(Ordering::Acquire) {
                        let _ = child.kill();
                    }
                    *done.control.status.lock().unwrap() = Status {
                        pid: child.id(),
                        ..Default::default()
                    };
                    children.push((child, done.control));
                }
                Err(error) => {
                    *done.control.status.lock().unwrap() = Status {
                        error: Some(error),
                        ..Default::default()
                    }
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) if children.is_empty() => break,
            Err(_) => {}
        }
        children.retain_mut(|(child, control)| {
            if control.cancel.swap(false, Ordering::AcqRel)
                || control.release.load(Ordering::Acquire)
            {
                let _ = child.kill();
            }
            match child.try_wait() {
                Ok(exit) => {
                    control.status.lock().unwrap().exit = exit;
                    !(exit.is_some() && control.release.load(Ordering::Acquire))
                }
                Err(error) => {
                    control.status.lock().unwrap().error = Some(error.to_string());
                    let _ = child.kill();
                    true
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
