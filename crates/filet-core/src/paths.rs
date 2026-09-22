use crate::{Error, Result};
use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

pub fn expand(base: &Path, value: &str) -> Result<PathBuf> {
    let clean = expand_lexical(base, value)?;
    reject_links(&clean)?;
    // Use one native representation for existing ancestors. In particular, Windows
    // canonical/verbatim paths and ordinary drive paths must compare as the same source.
    // Preserve nonexistent target components; planning must not create directories.
    let mut ancestor = clean.clone();
    let mut tail = Vec::new();
    loop {
        match fs::canonicalize(&ancestor) {
            Ok(mut path) => {
                for name in tail.into_iter().rev() {
                    path.push(name);
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = ancestor
                    .file_name()
                    .ok_or_else(|| Error::new("INVALID_PATH", "path has no accessible root"))?
                    .to_os_string();
                tail.push(name);
                ancestor.pop();
            }
            Err(e) => return Err(e.into()),
        }
    }
}
fn expand_lexical(base: &Path, value: &str) -> Result<PathBuf> {
    let p = if value == "~" || value.starts_with("~/") || value.starts_with("~\\") {
        let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .ok_or_else(|| Error::new("INVALID_PATH", "home directory unavailable"))?;
        PathBuf::from(home).join(value.get(2..).unwrap_or(""))
    } else {
        PathBuf::from(value)
    };
    let p = if p.is_absolute() { p } else { base.join(p) };
    let mut clean = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            _ => clean.push(c.as_os_str()),
        }
    }
    if !clean.is_absolute() {
        return Err(Error::new("INVALID_PATH", "absolute path required"));
    }
    Ok(clean)
}

pub fn reject_links(path: &Path) -> Result<()> {
    let mut p = PathBuf::new();
    for c in path.components() {
        p.push(c.as_os_str());
        // A Windows drive/verbatim/UNC prefix is not a filesystem entry on its own.
        // Inspect the completed root after RootDir is appended, then every descendant.
        if matches!(c, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&p) {
            Ok(m) => {
                #[cfg(windows)]
                let link = {
                    use std::os::windows::fs::MetadataExt;
                    m.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let link = m.file_type().is_symlink();
                if link {
                    return Err(Error::new(
                        "UNSUPPORTED_PATH",
                        format!("links/reparse points are not followed: {}", p.display()),
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(Error::new(
                    "IO_ERROR",
                    format!("cannot inspect {}: {e}", p.display()),
                ));
            }
        }
    }
    Ok(())
}

pub fn valid_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(Error::new(
            "INVALID_PATH",
            "rename.name must be one filename",
        ));
    }
    #[cfg(windows)]
    {
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if name.contains(['<', '>', ':', '"', '|', '?', '*'])
            || name.ends_with(['.', ' '])
            || name.chars().any(|c| c < ' ')
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(Error::new("INVALID_PATH", "unsupported Windows filename"));
        }
    }
    Ok(())
}

pub fn resolve_program(base: &Path, program: &str) -> Result<PathBuf> {
    if program.is_empty() || program.contains('\0') {
        return Err(Error::new("INVALID_ACTION", "invalid executable"));
    }
    let path = Path::new(program);
    let candidates: Vec<PathBuf> = if path.is_absolute() || path.components().count() > 1 {
        // Executables are explicitly trusted code. System aliases such as /bin/sh are normal.
        vec![expand_lexical(base, program)?]
    } else {
        env::split_paths(&env::var_os("PATH").unwrap_or_default())
            .flat_map(|d| {
                let mut p = vec![d.join(program)];
                if cfg!(windows) && path.extension().is_none() {
                    p.push(d.join(format!("{program}.exe")));
                }
                p
            })
            .collect()
    };
    for p in candidates {
        if p.is_file() {
            let p = fs::canonicalize(p)?;
            #[cfg(windows)]
            if p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("bat") || e.eq_ignore_ascii_case("cmd"))
            {
                return Err(Error::new(
                    "UNSUPPORTED_EXEC",
                    "invoke a shell explicitly for batch files",
                ));
            }
            return Ok(p);
        }
    }
    Err(Error::new(
        "PROGRAM_NOT_FOUND",
        format!("executable not found: {program}"),
    ))
}
