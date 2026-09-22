use crate::{Error, Evaluation, Event, FileContext, Result, Snapshot, paths};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata},
    io::Read,
    path::Path,
    time::UNIX_EPOCH,
};

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn modified_ns(m: &Metadata) -> Result<String> {
    let t = m.modified()?;
    Ok(match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos().to_string(),
        Err(e) => format!("-{}", e.duration().as_nanos()),
    })
}
pub fn identity(file: &File, m: &Metadata) -> Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = file;
        Ok(format!("{}:{}", m.dev(), m.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let _ = m;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
}
pub fn take(path: &Path) -> Result<Snapshot> {
    paths::reject_links(path)?;
    let mut f = File::open(path)?;
    let before = f.metadata()?;
    if !before.is_file() {
        return Err(Error::new(
            "UNSUPPORTED_FILE",
            "only regular files are supported",
        ));
    }
    let id = identity(&f, &before)?;
    let mut hasher = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let after = f.metadata()?;
    let path_f = File::open(path)?;
    if before.len() != after.len()
        || modified_ns(&before)? != modified_ns(&after)?
        || id != identity(&path_f, &path_f.metadata()?)?
    {
        return Err(Error::new("NOT_READY", "input changed while being read"));
    }
    Ok(Snapshot {
        path: path.to_path_buf(),
        identity: id,
        size_bytes: after.len(),
        modified_ns: modified_ns(&after)?,
        modified_at: after
            .modified()
            .ok()
            .map(|t| DateTime::<Utc>::from(t).to_rfc3339()),
        created_at: after
            .created()
            .ok()
            .map(|t| DateTime::<Utc>::from(t).to_rfc3339()),
        sha256: format!("{:x}", hasher.finalize()),
    })
}
pub fn verify(expected: &Snapshot) -> Result<()> {
    let actual = take(&expected.path).map_err(|e| Error::new("PLAN_STALE", e.to_string()))?;
    if !same_version(expected, &actual) {
        return Err(Error::new(
            "PLAN_STALE",
            format!("input changed: {}", expected.path.display()),
        ));
    }
    Ok(())
}
pub fn same_version(a: &Snapshot, b: &Snapshot) -> bool {
    a.identity == b.identity
        && a.size_bytes == b.size_bytes
        && a.modified_ns == b.modified_ns
        && a.created_at == b.created_at
        && a.sha256 == b.sha256
}
pub fn context(s: &Snapshot, reason: &str, now: DateTime<Utc>) -> Result<Evaluation> {
    let name = s
        .path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| Error::new("NON_UTF8_PATH", "file name is not valid Unicode"))?
        .to_string();
    let stem = s
        .path
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or(&name)
        .to_string();
    let extension = s
        .path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if s.path.to_str().is_none() {
        return Err(Error::new(
            "NON_UTF8_PATH",
            "path cannot be represented without loss",
        ));
    }
    if s.size_bytes > 9_007_199_254_740_991 {
        return Err(Error::new(
            "INTEGER_RANGE",
            "file size exceeds JavaScript safe integer range",
        ));
    }
    Ok(Evaluation {
        file: FileContext {
            file_ref: "input".into(),
            name,
            stem,
            extension,
            size_bytes: s.size_bytes,
            modified_at: s.modified_at.clone(),
            created_at: s.created_at.clone(),
            path: s.path.to_str().map(str::to_owned),
        },
        event: Event {
            reason: reason.into(),
        },
        now: now.to_rfc3339(),
    })
}
pub fn cheap_version(path: &Path) -> Result<String> {
    paths::reject_links(path)?;
    let f = fs::File::open(path)?;
    let m = f.metadata()?;
    if !m.is_file() {
        return Err(Error::new("UNSUPPORTED_FILE", "not a regular file"));
    }
    Ok(format!(
        "{}:{}:{}",
        identity(&f, &m)?,
        m.len(),
        modified_ns(&m)?
    ))
}
