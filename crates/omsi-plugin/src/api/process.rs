mod grants;
pub use grants::ProcessGrants;
use std::collections::BTreeMap;
use std::process::{Child, Command, ExitStatus, Stdio};

const MAX_CHILDREN: usize = 32;
#[derive(Default)]
pub struct Processes {
    next: u64,
    children: BTreeMap<u64, Child>,
}

pub struct Status {
    pub pid: u32,
    pub exit: Option<ExitStatus>,
}

impl Processes {
    pub fn start(
        &mut self,
        file: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &[(String, String)],
    ) -> Result<u64, String> {
        if self.children.len() >= MAX_CHILDREN {
            return Err("too many process handles (limit 32); forget completed processes".into());
        }
        let id = self
            .next
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or("process handle limit reached")?;
        let mut cmd = Command::new(file);
        cmd.args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let child = cmd.spawn().map_err(|e| e.to_string())?;
        self.next = id;
        self.children.insert(id, child);
        Ok(id)
    }

    pub fn status(&mut self, id: u64) -> Result<Status, String> {
        let child = self.children.get_mut(&id).ok_or("unknown process handle")?;
        Ok(Status {
            pid: child.id(),
            exit: child.try_wait().map_err(|e| e.to_string())?,
        })
    }

    pub fn kill(&mut self, id: u64) -> Result<(), String> {
        let child = self.children.get_mut(&id).ok_or("unknown process handle")?;
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            child.kill().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn forget(&mut self, id: u64) -> Result<(), String> {
        if self.status(id)?.exit.is_none() {
            return Err("process still running".into());
        }
        self.children.remove(&id);
        Ok(())
    }

    pub fn poll(&mut self) {
        for child in self.children.values_mut() {
            let _ = child.try_wait();
        }
    }

    pub fn stop_all(&mut self) {
        let mut pending = Vec::new();
        for (_, mut child) in std::mem::take(&mut self.children) {
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
                pending.push(child);
            }
        }
        if !pending.is_empty() {
            std::thread::spawn(move || {
                for mut child in pending {
                    let _ = child.wait();
                }
            });
        }
    }
}

impl Drop for Processes {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn shell(p: &mut Processes, script: &str) -> u64 {
        #[cfg(unix)]
        let (file, args) = ("/bin/sh", vec!["-c".into(), script.into()]);
        #[cfg(windows)]
        let (file, args) = ("cmd.exe", vec!["/C".into(), script.into()]);
        p.start(file, &args, None, &[]).unwrap()
    }
    fn exited(p: &mut Processes, id: u64) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(exit) = p.status(id).unwrap().exit {
                return exit;
            }
            assert!(Instant::now() < deadline, "child did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn retains_nonzero_exit_code_until_forgotten() {
        let mut p = Processes::default();
        let id = shell(&mut p, "exit 37");
        assert_eq!(exited(&mut p, id).code(), Some(37));
        assert_eq!(p.status(id).unwrap().exit.unwrap().code(), Some(37));
        p.forget(id).unwrap();
        assert!(p.status(id).is_err());
        assert!(p.kill(id).is_err());
    }
    #[test]
    fn running_poll_and_kill_do_not_wait_for_child() {
        let mut p = Processes::default();
        #[cfg(unix)]
        let id = shell(&mut p, "exec sleep 60");
        #[cfg(windows)]
        let id = shell(&mut p, "for /L %i in (1,1,2147483647) do @rem");
        assert!(p.status(id).unwrap().exit.is_none());
        assert!(p.forget(id).is_err());
        p.kill(id).unwrap();
        assert!(!exited(&mut p, id).success());
    }
    #[test]
    fn failed_start_does_not_allocate_a_handle() {
        let mut p = Processes::default();
        assert!(p
            .start("/nonexistent/openomsi-test-program", &[], None, &[])
            .is_err());
        assert!(p.children.is_empty());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn unload_kills_and_reaps_owned_children() {
        let mut p = Processes::default();
        let id = shell(&mut p, "exec sleep 60");
        let pid = p.status(id).unwrap().pid;
        p.stop_all();
        assert!(p.status(id).is_err());
        let path = std::path::PathBuf::from(format!("/proc/{pid}"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!path.exists(), "child was not killed and reaped");
    }

    #[cfg(unix)]
    #[test]
    fn arguments_are_literal_and_cwd_and_environment_are_passed() {
        let mut p = Processes::default();
        let args = ["-c", "test \"$1\" = '$HOME; exit 99' && test \"$PLUGIN_TEST\" = 'hello world' && test \"$PWD\" = '/tmp'", "test", "$HOME; exit 99"].map(str::to_owned);
        let id = p
            .start(
                "/bin/sh",
                &args,
                Some("/tmp"),
                &[("PLUGIN_TEST".into(), "hello world".into())],
            )
            .unwrap();
        assert!(exited(&mut p, id).success());
    }
}
