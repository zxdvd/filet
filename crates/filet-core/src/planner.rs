use crate::{
    config::{Condition, Loaded, Package, duration},
    paths, snapshot, *,
};
use chrono::{DateTime, Utc};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub trait ScriptEvaluator {
    fn check(&self, package: &Package) -> Result<()>;
    fn evaluate(&self, package: &Package, input: &Evaluation) -> Result<Vec<Draft>>;
}
pub struct NoScripts;
impl ScriptEvaluator for NoScripts {
    fn check(&self, _: &Package) -> Result<()> {
        Err(Error::new("JS_UNAVAILABLE", "no script evaluator"))
    }
    fn evaluate(&self, _: &Package, _: &Evaluation) -> Result<Vec<Draft>> {
        Err(Error::new("JS_UNAVAILABLE", "no script evaluator"))
    }
}
pub fn matches(c: &Condition, input: &Evaluation) -> Result<bool> {
    let f = &input.file;
    if c.extension.as_ref().is_some_and(|e| *e != f.extension) {
        return Ok(false);
    }
    if let Some(g) = &c.name_glob
        && !globset::Glob::new(g)
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?
            .compile_matcher()
            .is_match(&f.name)
    {
        return Ok(false);
    }
    if c.min_size_bytes.is_some_and(|s| f.size_bytes < s)
        || c.max_size_bytes.is_some_and(|s| f.size_bytes > s)
    {
        return Ok(false);
    }
    let now = DateTime::parse_from_rfc3339(&input.now)
        .map_err(|e| Error::new("INVALID_DATA", e.to_string()))?;
    for (limit, time) in [
        (&c.modified_older_than, &f.modified_at),
        (&c.created_older_than, &f.created_at),
    ] {
        if let Some(limit) = limit {
            let Some(time) = time else { return Ok(false) };
            let time = DateTime::parse_from_rfc3339(time)
                .map_err(|e| Error::new("INVALID_DATA", e.to_string()))?;
            let threshold = duration(limit)?;
            if now
                .signed_duration_since(time)
                .to_std()
                .ok()
                .is_none_or(|age| age < threshold)
            {
                return Ok(false);
            }
        }
    }
    if let Some(all) = &c.all {
        for c in all {
            if !matches(c, input)? {
                return Ok(false);
            }
        }
    }
    if let Some(any) = &c.any {
        let mut hit = false;
        for c in any {
            hit |= matches(c, input)?;
        }
        if !hit {
            return Ok(false);
        }
    }
    if let Some(not) = &c.not
        && matches(not, input)?
    {
        return Ok(false);
    }
    Ok(true)
}
pub fn validate_drafts(drafts: &[Draft]) -> Result<()> {
    if drafts.len() > 64 {
        return Err(Error::new("ACTION_LIMIT", "at most 64 actions are allowed"));
    }
    for (i, d) in drafts.iter().enumerate() {
        match d {
            Draft::Copy(t) | Draft::Move(t) => {
                valid_ref(&t.file)?;
                if t.to.is_empty() {
                    return Err(Error::new("INVALID_ACTION", "empty destination"));
                }
            }
            Draft::Rename(t) => {
                valid_ref(&t.file)?;
                paths::valid_name(&t.name)?;
            }
            Draft::Exec(e) => {
                if i + 1 != drafts.len() {
                    return Err(Error::new(
                        "INVALID_ACTION",
                        "exec must be the final action",
                    ));
                }
                if e.program.is_empty() || e.program.contains('\0') || e.args.len() > 256 {
                    return Err(Error::new(
                        "INVALID_ACTION",
                        "invalid program/argument count",
                    ));
                }
                for a in &e.args {
                    match a {
                        Arg::Path(p) => valid_ref(&p.path_of)?,
                        Arg::Literal(s) if s.contains('\0') => {
                            return Err(Error::new("INVALID_ACTION", "NUL argument"));
                        }
                        _ => {}
                    }
                }
                if e.env
                    .iter()
                    .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
                {
                    return Err(Error::new("INVALID_ACTION", "invalid environment"));
                }
                if duration(&e.timeout)?.as_secs() > 3600 {
                    return Err(Error::new(
                        "INVALID_ACTION",
                        "exec timeout exceeds one hour",
                    ));
                }
            }
        }
    }
    Ok(())
}
fn valid_ref(r: &str) -> Result<()> {
    if r != "input" {
        Err(Error::new(
            "INVALID_FILE_REF",
            "only this task's input FileRef is valid",
        ))
    } else {
        Ok(())
    }
}
pub fn resolve(base: &Path, input: &Path, drafts: Vec<Draft>) -> Result<Vec<Action>> {
    validate_drafts(&drafts)?;
    let mut current = input.to_path_buf();
    let mut actions = Vec::new();
    let mut targets = BTreeSet::new();
    for d in drafts {
        let a = match d {
            Draft::Copy(t) => {
                let to = paths::expand(base, &t.to)?.join(current.file_name().unwrap());
                Action::Copy {
                    from: current.clone(),
                    to,
                    conflict: t.on_conflict,
                }
            }
            Draft::Move(t) => {
                let to = paths::expand(base, &t.to)?.join(current.file_name().unwrap());
                let from = current.clone();
                current = to.clone();
                Action::Move {
                    from,
                    to,
                    conflict: t.on_conflict,
                }
            }
            Draft::Rename(t) => {
                let to = current.with_file_name(t.name);
                let from = current.clone();
                current = to.clone();
                Action::Rename {
                    from,
                    to,
                    conflict: t.on_conflict,
                }
            }
            Draft::Exec(e) => Action::Exec {
                program: paths::resolve_program(base, &e.program)?,
                args: e.args,
                cwd: paths::expand(base, e.cwd.as_deref().unwrap_or("."))?,
                env: e.env,
                timeout_ms: duration(&e.timeout)?.as_millis() as u64,
            },
        };
        if let Some((from, to, _)) = a.transfer() {
            paths::reject_links(to)?;
            paths::valid_name(
                to.file_name()
                    .and_then(|v| v.to_str())
                    .ok_or_else(|| Error::new("NON_UTF8_PATH", "target filename"))?,
            )?;
            if from == to || to == input || !targets.insert(to.clone()) {
                return Err(Error::new(
                    "INVALID_ACTION",
                    "no-op, cyclic, or repeated targets are unsupported",
                ));
            }
            if to.exists() && !to.is_file() {
                return Err(Error::new(
                    "TARGET_CONFLICT",
                    "destination is not a regular file",
                ));
            }
        }
        actions.push(a);
    }
    Ok(actions)
}
pub fn check(config: &Loaded, js: &dyn ScriptEvaluator) -> Result<()> {
    for (id, p) in &config.packages {
        js.check(p).map_err(|e| e.rule(id))?;
    }
    Ok(())
}
pub fn plan(
    config: &Loaded,
    js: &dyn ScriptEvaluator,
    path: &Path,
    reason: &str,
    due: Option<&BTreeSet<String>>,
) -> Result<Option<Plan>> {
    let path = paths::expand(
        &std::env::current_dir()?,
        path.to_str()
            .ok_or_else(|| Error::new("NON_UTF8_PATH", "input path"))?,
    )?;
    let source = config
        .roots
        .iter()
        .find(|(_, root)| path.starts_with(root))
        .map(|(id, _)| id)
        .ok_or_else(|| Error::new("OUTSIDE_SOURCE", "input is outside configured sources"))?;
    let root = &config.roots[source];
    let source_config = &config.config.sources[source];
    if !source_config.recursive && path.parent() != Some(root.as_path()) {
        return Err(Error::new(
            "OUTSIDE_SOURCE",
            "input is below a nonrecursive source",
        ));
    }
    if ignored(source_config, root, &path)? {
        return Ok(None);
    }
    let snap = snapshot::take(&path)?;
    let ctx = snapshot::context(&snap, reason, Utc::now())?;
    for r in config
        .config
        .rules
        .iter()
        .filter(|r| r.on.source == *source)
    {
        if reason != "manual"
            && (if reason == "scheduled" {
                r.on.kind != "scan" || due.is_some_and(|ids| !ids.contains(&r.id))
            } else {
                r.on.kind != "file.ready"
            })
        {
            continue;
        }
        let drafts = if let Some(p) = config.packages.get(&r.id) {
            js.evaluate(p, &ctx).map_err(|e| e.rule(&r.id))?
        } else if r
            .when
            .as_ref()
            .map(|c| matches(c, &ctx))
            .transpose()?
            .unwrap_or(true)
        {
            r.actions.clone().unwrap_or_default()
        } else {
            Vec::new()
        };
        if drafts.is_empty() {
            continue;
        }
        let actions = resolve(&config.base, &path, drafts).map_err(|e| e.rule(&r.id))?;
        let risks = if actions.iter().any(|a| matches!(a, Action::Exec { .. })) {
            vec!["external_effects_not_previewable_or_undoable".into()]
        } else {
            vec![]
        };
        let id = snapshot::hash(&serde_json::to_vec(&(
            &config.revision,
            &r.id,
            source,
            &snap,
            &actions,
        ))?);
        return Ok(Some(Plan {
            format_version: 1,
            plan_id: id,
            rule_id: r.id.clone(),
            rule_revision: config.revision.clone(),
            source: source.clone(),
            input_snapshot: snap,
            actions,
            risks,
        }));
    }
    Ok(None)
}
pub fn ignored(source: &crate::config::Source, root: &Path, path: &Path) -> Result<bool> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    // Always exclude Filet's own intermediate names, even with custom ignore settings.
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(".filet-"))
    {
        return Ok(true);
    }
    for g in &source.ignore {
        if globset::Glob::new(g)
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?
            .compile_matcher()
            .is_match(rel)
        {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn output_paths(plan: &Plan) -> Vec<PathBuf> {
    plan.actions
        .iter()
        .filter_map(|a| a.transfer().map(|(_, to, _)| to.clone()))
        .collect()
}
