use crate::{config::Loaded, paths, process, snapshot, state::Store, *};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub fn apply(store: &Store, config: &Loaded, id: &str) -> Result<serde_json::Value> {
    let p = store.plan(id)?;
    let status = store.status(id)?;
    if status == "succeeded" {
        return Ok(serde_json::json!({"planId":id,"status":status,"reused":true}));
    }
    if status == "failed" || status == "needs_review" {
        return Err(Error::new(
            "JOB_BLOCKED",
            format!("{id}: {status}; inspect history before taking manual action"),
        ));
    }
    if p.format_version != 1 || p.rule_revision != config.revision {
        return Err(Error::new(
            "PLAN_STALE",
            "configuration or script dependencies changed",
        ));
    }
    if store.blocked(&p.input_snapshot.path, id)? {
        return Err(Error::new(
            "JOB_BLOCKED",
            "unresolved prior work blocks this file",
        ));
    }
    if status == "planned" {
        snapshot::verify(&p.input_snapshot)?;
        // Reject conflicts across the whole plan before starting any mutation.
        for a in &p.actions {
            if let Some((_, to, policy)) = a.transfer() {
                paths::reject_links(to)?;
                if to.try_exists()? && *policy == Conflict::Error {
                    return Err(Error::new("TARGET_CONFLICT", to.display().to_string()));
                }
            }
        }
    }
    store.set_status(id, "applying", None)?;
    let mut current = p.input_snapshot.clone();
    for (i, a) in p.actions.iter().enumerate() {
        if let Some((state, receipt)) = store.get_operation(id, i)? {
            if state == "succeeded" {
                if let Some(after) = &receipt.after
                    && snapshot::verify(after).is_ok()
                {
                    store.suppress(config, after)?;
                }
                if a.is_move() && !receipt.skipped {
                    current = receipt
                        .after
                        .ok_or_else(|| Error::new("STATE_ERROR", "missing move receipt"))?;
                }
                if receipt.skipped && a.is_move() {
                    break;
                }
                continue;
            }
            // A started external command must never be blindly replayed.
            let recovered = recover_step(a, &receipt);
            match recovered {
                Ok(Some(after)) => {
                    let receipt = Receipt {
                        after: Some(after.clone()),
                        ..receipt
                    };
                    store.operation(id, i, "succeeded", &receipt, None)?;
                    store.suppress(config, &after)?;
                    if a.is_move() {
                        current = after;
                    }
                    continue;
                }
                _ => {
                    let e = Error::new(
                        "OPERATION_UNCERTAIN",
                        "interrupted operation requires review; it was not retried",
                    );
                    store.set_status(id, "needs_review", Some(&e))?;
                    return Err(e);
                }
            }
        }
        let mut receipt = Receipt {
            before: current.clone(),
            after: None,
            temp: None,
            skipped: false,
            output: None,
        };
        if let Some((_, to, _)) = a.transfer() {
            receipt.temp = Some(to.with_file_name(format!(".filet-{}-{i}.tmp", p.plan_id)));
        }
        store.operation(id, i, "intent", &receipt, None)?;
        let result = (|| -> Result<()> {
            snapshot::verify(&current)?;
            if let Some((from, to, policy)) = a.transfer() {
                if from != &current.path {
                    return Err(Error::new(
                        "STATE_ERROR",
                        "FileRef and resolved plan disagree",
                    ));
                }
                paths::reject_links(to)?;
                if to.try_exists()? {
                    if *policy == Conflict::Skip {
                        receipt.skipped = true;
                        return Ok(());
                    }
                    return Err(Error::new("TARGET_CONFLICT", to.display().to_string()));
                }
                fs::create_dir_all(to.parent().unwrap())?;
                paths::reject_links(to)?;
                store.operation(id, i, "started", &receipt, None)?;
                let linked = if a.is_move() {
                    match fs::hard_link(&current.path, to) {
                        Ok(()) => true,
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                            return Err(Error::new("TARGET_CONFLICT", to.display().to_string()));
                        }
                        Err(_) => false,
                    }
                } else {
                    false
                };
                if !linked {
                    let temp = receipt.temp.as_ref().unwrap();
                    copy_verified(&current, temp)?;
                    fs::hard_link(temp,to).map_err(|e|if e.kind()==std::io::ErrorKind::AlreadyExists {Error::new("TARGET_CONFLICT",to.display().to_string())}else{Error::new("PUBLISH_FAILED",format!("no-clobber publication failed; original and temporary file retained: {e}"))})?;
                }
                sync_parent(to)?;
                let after = snapshot::take(to)?;
                if after.sha256 != current.sha256 || after.size_bytes != current.size_bytes {
                    return Err(Error::new(
                        "OPERATION_UNCERTAIN",
                        "destination changed before verification",
                    ));
                }
                receipt.after = Some(after.clone());
                store.operation(id, i, "published", &receipt, None)?;
                snapshot::verify(&current)?;
                if a.is_move() {
                    fs::remove_file(&current.path)?;
                    sync_parent(&current.path)?;
                    current = after;
                }
                if !linked {
                    fs::remove_file(receipt.temp.as_ref().unwrap())?;
                    sync_parent(to)?;
                }
            } else {
                store.operation(id, i, "started", &receipt, None)?;
                let result = process::run(a, &current.path)?;
                receipt.output = Some(result.clone());
                if result["timedOut"].as_bool() == Some(true)
                    || result["outputIncomplete"].as_bool() == Some(true)
                {
                    return Err(Error::new(
                        "EXEC_UNCERTAIN",
                        "command timed out or output pipes did not close; external effects need review",
                    ));
                }
                if result["exitCode"].as_i64() != Some(0) {
                    return Err(Error::new(
                        "EXEC_FAILED",
                        "command failed; external effects need review",
                    ));
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            // Even a failed built-in can have published an output. Keep every artifact and stop.
            let needs_review = receipt.after.is_some()
                || matches!(a, Action::Exec { .. })
                || receipt.temp.as_ref().is_some_and(|p| p.exists())
                || a.transfer().is_some_and(|(_, to, _)| to.exists());
            let state = if needs_review {
                "needs_review"
            } else {
                "failed"
            };
            store.operation(
                id,
                i,
                if needs_review { "uncertain" } else { "failed" },
                &receipt,
                Some(&e),
            )?;
            store.set_status(id, state, Some(&e))?;
            return Err(e);
        }
        store.operation(id, i, "succeeded", &receipt, None)?;
        if let Some(after) = &receipt.after {
            store.suppress(config, after)?;
        }
        // A skipped move/rename stops the plan: later concrete targets were derived from that move.
        if receipt.skipped && a.is_move() {
            break;
        }
    }
    // exec may have modified the referenced file; observe its actual post-state.
    if let Ok(after) = snapshot::take(&current.path)
        && (p.actions.iter().any(|a| matches!(a, Action::Exec { .. }))
            || snapshot::same_version(&current, &after))
    {
        store.suppress(config, &after)?;
    }
    store.set_status(id, "succeeded", None)?;
    Ok(serde_json::json!({"planId":id,"status":"succeeded","reused":false}))
}
fn copy_verified(source: &Snapshot, temp: &Path) -> Result<()> {
    paths::reject_links(temp)?;
    let mut out = OpenOptions::new().write(true).create_new(true).open(temp)?;
    let mut input = File::open(&source.path)?;
    let mut buffer = [0; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        out.write_all(&buffer[..n])?;
    }
    out.set_permissions(input.metadata()?.permissions())?;
    filetime::set_file_handle_times(
        &out,
        None,
        Some(filetime::FileTime::from_last_modification_time(
            &input.metadata()?,
        )),
    )?;
    out.sync_all()?;
    snapshot::verify(source)?;
    let copy = snapshot::take(temp)?;
    if copy.sha256 != source.sha256 || copy.size_bytes != source.size_bytes {
        return Err(Error::new(
            "COPY_VERIFY_FAILED",
            "copy verification failed; source retained",
        ));
    }
    Ok(())
}
pub fn recover_step(action: &Action, r: &Receipt) -> Result<Option<Snapshot>> {
    let Some((_, to, _)) = action.transfer() else {
        return Ok(None);
    };
    let Ok(after) = snapshot::take(to) else {
        return Ok(None);
    };
    if after.sha256 != r.before.sha256 || after.size_bytes != r.before.size_bytes {
        return Ok(None);
    }
    let owned = if let Some(recorded) = &r.after {
        snapshot::same_version(recorded, &after)
    } else if action.is_move() && after.identity == r.before.identity {
        true
    } else {
        r.temp
            .as_ref()
            .and_then(|p| snapshot::take(p).ok())
            .is_some_and(|s| snapshot::same_version(&s, &after))
    };
    if !owned {
        return Ok(None);
    }
    if action.is_move() {
        // Both locations present may be a partial move or outside interference. Never guess which to delete.
        if r.before.path.try_exists()? {
            return Ok(None);
        }
    } else {
        snapshot::verify(&r.before)?;
    }
    Ok(Some(after))
}
pub fn recover(store: &Store, config: &Loaded) -> Result<Vec<serde_json::Value>> {
    let mut results = Vec::new();
    for id in store.active()? {
        match apply(store, config, &id) {
            Ok(v) => results.push(v),
            Err(e) => {
                store.set_status(&id, "needs_review", Some(&e))?;
                results.push(serde_json::json!({"planId":id,"status":"needs_review","error":e}));
            }
        }
    }
    Ok(results)
}
fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path.parent().unwrap())?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
