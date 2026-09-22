use crate::{Action, Arg, Error, Result};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 65536;
trait Pipe: Read + Send + 'static {
    fn ready(&self) -> std::io::Result<bool>;
}
macro_rules! impl_pipe {
    ($t:ty) => {
        impl Pipe for $t {
            fn ready(&self) -> std::io::Result<bool> {
                #[cfg(unix)]
                {
                    use std::os::fd::AsRawFd;
                    let mut poll = libc::pollfd {
                        fd: self.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    let n = unsafe { libc::poll(&mut poll, 1, 10) };
                    if n < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(n > 0)
                }
                #[cfg(windows)]
                {
                    use std::os::windows::io::AsRawHandle;
                    let mut available = 0;
                    let ok = unsafe {
                        windows_sys::Win32::System::Pipes::PeekNamedPipe(
                            self.as_raw_handle(),
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                            &mut available,
                            std::ptr::null_mut(),
                        )
                    };
                    if ok == 0 {
                        return Ok(true);
                    }
                    if available == 0 {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Ok(available > 0)
                }
            }
        }
    };
}
impl_pipe!(std::process::ChildStdout);
impl_pipe!(std::process::ChildStderr);
fn drain(mut reader: impl Pipe, stop: Arc<AtomicBool>) -> thread::JoinHandle<(Vec<u8>, bool)> {
    thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0; 4096];
        let mut stopping = None;
        loop {
            if stop.load(Ordering::SeqCst) {
                let since = stopping.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_millis(200) {
                    return (out, true);
                }
            }
            match reader.ready() {
                Ok(false) => continue,
                Err(_) => return (out, true),
                Ok(true) => match reader.read(&mut buf) {
                    Ok(0) => return (out, false),
                    Err(_) => return (out, true),
                    Ok(n) => {
                        let keep = n.min(OUTPUT_LIMIT.saturating_sub(out.len()));
                        out.extend_from_slice(&buf[..keep]);
                    }
                },
            }
        }
    })
}
pub fn run(action: &Action, path: &Path) -> Result<serde_json::Value> {
    let Action::Exec {
        program,
        args,
        cwd,
        env,
        timeout_ms,
    } = action
    else {
        return Err(Error::new("INVALID_ACTION", "expected exec"));
    };
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Windows needs SystemRoot for normal system DLL/program initialization. Nothing else is inherited.
    #[cfg(windows)]
    if !env.keys().any(|k| k.eq_ignore_ascii_case("SystemRoot")) {
        if let Some(v) = std::env::var_os("SystemRoot") {
            cmd.env("SystemRoot", v);
        }
    }
    for a in args {
        match a {
            Arg::Literal(s) => {
                cmd.arg(s);
            }
            Arg::Path(_) => {
                cmd.arg(path);
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::new("EXEC_START_FAILED", e.to_string()))?;
    #[cfg(windows)]
    let job = match WindowsJob::attach(&child) {
        Ok(j) => j,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let stdout = drain(child.stdout.take().unwrap(), stop.clone());
    let stderr = drain(child.stderr.take().unwrap(), stop.clone());
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= Duration::from_millis(*timeout_ms) {
            timed_out = true;
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            #[cfg(windows)]
            job.terminate();
            let _ = child.kill();
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    // Commands may not leave descendants holding output pipes open indefinitely.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    job.terminate();
    stop.store(true, Ordering::SeqCst);
    let (out, out_incomplete) = stdout.join().unwrap_or((Vec::new(), true));
    let (err, err_incomplete) = stderr.join().unwrap_or((Vec::new(), true));
    Ok(
        serde_json::json!({"exitCode":status.code(),"timedOut":timed_out,"outputIncomplete":out_incomplete||err_incomplete,"stdout":String::from_utf8_lossy(&out),"stderr":String::from_utf8_lossy(&err),"outputLimitBytes":OUTPUT_LIMIT,"stdoutMayBeTruncated":out.len()==OUTPUT_LIMIT,"stderrMayBeTruncated":err.len()==OUTPUT_LIMIT}),
    )
}

#[cfg(windows)]
struct WindowsJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl WindowsJob {
    fn attach(child: &std::process::Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let job = Self(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&info as *const _) as *const _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(job)
        }
    }
    fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
}
#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
