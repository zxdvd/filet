use crate::{
    config::{Loaded, duration},
    executor,
    planner::{self, ScriptEvaluator},
    snapshot,
    state::Store,
    *,
};
use rusqlite::{OptionalExtension, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

struct Scan {
    iter: walkdir::IntoIter,
    due: BTreeSet<String>,
    initial: bool,
    failed: bool,
}
pub struct Engine {
    scans: BTreeMap<String, Scan>,
    next: BTreeMap<String, Instant>,
    schedule: BTreeMap<String, Instant>,
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
impl Engine {
    pub fn new() -> Self {
        Self {
            scans: BTreeMap::new(),
            next: BTreeMap::new(),
            schedule: BTreeMap::new(),
        }
    }
    pub fn mark_dirty(&mut self) {
        self.next.clear();
    }
    /// One bounded slice per source. WalkDir streams directory entries; the durable observations table is the queue.
    pub fn tick(
        &mut self,
        store: &Store,
        config: &Loaded,
        js: &dyn ScriptEvaluator,
    ) -> Result<Vec<serde_json::Value>> {
        let mut events = Vec::new();
        for (source, root) in &config.roots {
            let src = &config.config.sources[source];
            if !self.scans.contains_key(source) {
                let now = Instant::now();
                let due: BTreeSet<_> = config
                    .config
                    .rules
                    .iter()
                    .filter(|r| {
                        r.on.source == *source
                            && r.on.kind == "scan"
                            && self.schedule.get(&r.id).is_none_or(|next| now >= *next)
                    })
                    .map(|r| r.id.clone())
                    .collect();
                if due.is_empty() && self.next.get(source).is_some_and(|next| now < *next) {
                    continue;
                }
                let signature = snapshot::hash(&serde_json::to_vec(&(root, src))?);
                let old: Option<(String, bool, String)> = store
                    .conn
                    .query_row(
                        "SELECT signature,initialized,revision FROM sources WHERE id=?1",
                        [source],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let initial = old
                    .as_ref()
                    .is_none_or(|(s, initialized, _)| s != &signature || !*initialized);
                if old.as_ref().is_some_and(|(s, _, _)| s != &signature) {
                    store
                        .conn
                        .execute("DELETE FROM observations WHERE source=?1", [source])?;
                }
                store.conn.execute("INSERT INTO sources VALUES (?1,?2,0,'scanning',NULL,?3) ON CONFLICT(id) DO UPDATE SET initialized=CASE WHEN sources.signature=excluded.signature THEN sources.initialized ELSE 0 END,signature=excluded.signature,status='scanning',revision=excluded.revision",params![source,signature,config.revision])?;
                if !root.is_dir() {
                    store.conn.execute(
                        "UPDATE sources SET status='unavailable' WHERE id=?1",
                        [source],
                    )?;
                    self.next
                        .insert(source.clone(), now + Duration::from_secs(5));
                    events.push(serde_json::json!({"source":source,"code":"SOURCE_UNAVAILABLE"}));
                    continue;
                }
                let depth = if src.recursive { usize::MAX } else { 1 };
                self.scans.insert(
                    source.clone(),
                    Scan {
                        iter: walkdir::WalkDir::new(root)
                            .min_depth(1)
                            .max_depth(depth)
                            .follow_links(false)
                            .max_open(16)
                            .into_iter(),
                        due,
                        initial,
                        failed: false,
                    },
                );
            }
            let scan = self.scans.get_mut(source).unwrap();
            let mut done = false;
            for _ in 0..256 {
                let Some(entry) = scan.iter.next() else {
                    done = true;
                    break;
                };
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        scan.failed = true;
                        events.push(serde_json::json!({"source":source,"code":"SCAN_ERROR","message":e.to_string()}));
                        continue;
                    }
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.into_path();
                match observe(store, config, js, source, &path, scan.initial, &scan.due) {
                    Ok(Some(v)) => events.push(v),
                    Ok(None) => {}
                    Err(e) => {
                        events.push(serde_json::json!({"source":source,"path":path,"error":e}));
                    }
                }
            }
            if done {
                let scan = self.scans.remove(source).unwrap();
                store.conn.execute("UPDATE sources SET initialized=CASE WHEN ?2 THEN initialized ELSE 1 END,status=?3,last_scan=?4 WHERE id=?1",params![source,scan.failed,if scan.failed{"unavailable"}else{"available"},chrono::Utc::now().to_rfc3339()])?;
                let unstable: bool = store.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM observations WHERE source=?1 AND stable_since>?2)",
                    params![
                        source,
                        chrono::Utc::now().timestamp_millis()
                            - duration(&src.ready.stable_for)?.as_millis() as i64
                    ],
                    |r| r.get(0),
                )?;
                for rule in config
                    .config
                    .rules
                    .iter()
                    .filter(|r| scan.due.contains(&r.id))
                {
                    let delay = if unstable {
                        duration(&src.ready.stable_for)?
                    } else {
                        duration(rule.on.every.as_ref().unwrap())?
                    };
                    self.schedule
                        .insert(rule.id.clone(), Instant::now() + delay);
                }
                let waiting: bool = store.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM observations WHERE source=?1 AND handled=0)",
                    [source],
                    |r| r.get(0),
                )?;
                let delay = if waiting {
                    Duration::from_millis(250)
                } else {
                    duration(&src.reconcile_every)?
                };
                self.next.insert(source.clone(), Instant::now() + delay);
            }
        }
        Ok(events)
    }
}
fn observe(
    store: &Store,
    config: &Loaded,
    js: &dyn ScriptEvaluator,
    source: &str,
    path: &PathBuf,
    initial: bool,
    due: &BTreeSet<String>,
) -> Result<Option<serde_json::Value>> {
    let src = &config.config.sources[source];
    if planner::ignored(src, &config.roots[source], path)? {
        return Ok(None);
    }
    let version = snapshot::cheap_version(path)?;
    let path_str = path
        .to_str()
        .ok_or_else(|| Error::new("NON_UTF8_PATH", "source filename"))?;
    let now = chrono::Utc::now().timestamp_millis();
    let old:Option<(String,bool,i64,i64,String)>=store.conn.query_row("SELECT version,handled,stable_since,first_seen,status FROM observations WHERE source=?1 AND path=?2",params![source,path_str],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let changed = old.as_ref().is_none_or(|(v, _, _, _, _)| *v != version);
    if changed {
        let first_seen = old
            .as_ref()
            .filter(|(_, handled, _, _, _)| !*handled)
            .map(|(_, _, _, first, _)| *first)
            .unwrap_or(now);
        if !initial && now - first_seen > duration(&src.ready.retry_for)?.as_millis() as i64 {
            store.conn.execute("INSERT INTO observations VALUES (?1,?2,?3,1,?4,?5,'not_ready') ON CONFLICT(source,path) DO UPDATE SET version=excluded.version,handled=1,stable_since=excluded.stable_since,status='not_ready'",params![source,path_str,version,now,first_seen])?;
            return Ok(Some(
                serde_json::json!({"source":source,"path":path,"code":"NOT_READY"}),
            ));
        }
        store.conn.execute("INSERT INTO observations VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(source,path) DO UPDATE SET version=excluded.version,handled=excluded.handled,stable_since=excluded.stable_since,first_seen=excluded.first_seen,status=excluded.status",params![source,path_str,version,initial,now,first_seen,if initial{"baseline"}else{"waiting"}])?;
        return Ok(None);
    }
    let (_, handled, since, first, status) = old.unwrap();
    if status == "own_effect" && handled {
        return Ok(None);
    }
    let scheduled = !due.is_empty();
    if handled && !scheduled {
        return Ok(None);
    }
    if now - since < (duration(&src.ready.stable_for)?.as_millis() as i64) {
        if now - first > duration(&src.ready.retry_for)?.as_millis() as i64 {
            store.conn.execute(
                "UPDATE observations SET status='not_ready',handled=1 WHERE source=?1 AND path=?2",
                params![source, path_str],
            )?;
            return Ok(Some(
                serde_json::json!({"source":source,"path":path,"code":"NOT_READY"}),
            ));
        }
        return Ok(None);
    }
    if store.blocked(path, "")? {
        return Ok(None);
    }
    let snapshot = snapshot::take(path)?;
    if store.succeeded_input(&snapshot, &config.revision)? {
        return Ok(None);
    }
    let reason = if scheduled { "scheduled" } else { "reconcile" };
    let planned = planner::plan(config, js, path, reason, Some(due));
    let result = match planned {
        Ok(Some(plan)) => {
            store.save(&plan)?;
            executor::apply(store, config, &plan.plan_id)
        }
        Ok(None) => {
            let id = snapshot::hash(&serde_json::to_vec(&(&snapshot, &config.revision, reason))?);
            store.conn.execute(
                "INSERT OR REPLACE INTO jobs VALUES (?1,?2,?3,?4,'','no_match',?5,NULL)",
                params![
                    id,
                    path_str,
                    crate::state::input_key(&snapshot)?,
                    config.revision,
                    chrono::Utc::now().to_rfc3339()
                ],
            )?;
            Ok(serde_json::json!({"path":path,"status":"no_match"}))
        }
        Err(e) => {
            // Persist evaluation failures too. A failed JS rule must never look like no_match.
            let id = snapshot::hash(&serde_json::to_vec(&(&snapshot, &config.revision, reason))?);
            store.conn.execute(
                "INSERT OR REPLACE INTO jobs VALUES (?1,?2,?3,?4,?5,'failed',?6,?7)",
                params![
                    id,
                    path_str,
                    crate::state::input_key(&snapshot)?,
                    config.revision,
                    e.rule_id.as_deref().unwrap_or(""),
                    chrono::Utc::now().to_rfc3339(),
                    serde_json::to_string(&e)?
                ],
            )?;
            Err(e)
        }
    };
    store.conn.execute(
        "UPDATE observations SET handled=1,status=?3 WHERE source=?1 AND path=?2 AND version=?4 AND status<>'own_effect'",
        params![
            source,
            path_str,
            if result.is_ok() { "observed" } else { "failed" },
            version
        ],
    )?;
    result.map(Some)
}
