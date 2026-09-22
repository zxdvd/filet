use crate::{Error, Plan, Receipt, Result, Snapshot, paths, snapshot};
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub struct ExecutionLock {
    _file: File,
}
impl ExecutionLock {
    pub fn acquire(dir: &Path) -> Result<Self> {
        paths::reject_links(dir)?;
        fs::create_dir_all(dir)?;
        let path = dir.join("executor.lock");
        paths::reject_links(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive().map_err(|e| {
            Error::new(
                "INSTANCE_LOCKED",
                format!("another executor holds this data directory: {e}"),
            )
        })?;
        Ok(Self { _file: file })
    }
}
pub struct Store {
    pub conn: Connection,
    pub dir: PathBuf,
}
impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        paths::reject_links(dir)?;
        fs::create_dir_all(dir)?;
        let db = dir.join("state.sqlite3");
        paths::reject_links(&db)?;
        let conn = Connection::open(&db)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let v: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v > 1 {
            return Err(Error::new(
                "STATE_VERSION",
                "database is newer than this application",
            ));
        }
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS sources (id TEXT PRIMARY KEY, signature TEXT NOT NULL, initialized INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL, last_scan TEXT, revision TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS observations (source TEXT NOT NULL, path TEXT NOT NULL, version TEXT NOT NULL, handled INTEGER NOT NULL, stable_since INTEGER NOT NULL, first_seen INTEGER NOT NULL, status TEXT NOT NULL, PRIMARY KEY(source,path));
          CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, resource TEXT NOT NULL, input_key TEXT NOT NULL, revision TEXT NOT NULL, rule_id TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL, error TEXT);
          CREATE TABLE IF NOT EXISTS plans (id TEXT PRIMARY KEY REFERENCES jobs(id), json TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS operations (plan_id TEXT NOT NULL REFERENCES plans(id), step INTEGER NOT NULL, status TEXT NOT NULL, receipt TEXT NOT NULL, error TEXT, PRIMARY KEY(plan_id,step));
          CREATE INDEX IF NOT EXISTS jobs_input ON jobs(input_key,revision);
          CREATE INDEX IF NOT EXISTS jobs_resource ON jobs(resource,status);
          PRAGMA user_version=1;")?;
        Ok(Self {
            conn,
            dir: dir.to_path_buf(),
        })
    }
    pub fn save(&self, p: &Plan) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO jobs VALUES (?1,?2,?3,?4,?5,'planned',?6,NULL)",
            params![
                p.plan_id,
                p.input_snapshot.path.to_str(),
                input_key(&p.input_snapshot)?,
                p.rule_revision,
                p.rule_id,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO plans VALUES (?1,?2)",
            params![p.plan_id, serde_json::to_string(p)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn plan(&self, id: &str) -> Result<Plan> {
        let s: Option<String> = self
            .conn
            .query_row("SELECT json FROM plans WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        serde_json::from_str(&s.ok_or_else(|| Error::new("PLAN_NOT_FOUND", id))?)
            .map_err(Into::into)
    }
    pub fn status(&self, id: &str) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT status FROM jobs WHERE id=?1", [id], |r| r.get(0))?)
    }
    pub fn set_status(&self, id: &str, status: &str, error: Option<&Error>) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status=?2,error=?3,updated_at=?4 WHERE id=?1",
            params![
                id,
                status,
                error.map(serde_json::to_string).transpose()?,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }
    pub fn operation(
        &self,
        id: &str,
        step: usize,
        status: &str,
        receipt: &Receipt,
        error: Option<&Error>,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO operations VALUES (?1,?2,?3,?4,?5) ON CONFLICT(plan_id,step) DO UPDATE SET status=excluded.status,receipt=excluded.receipt,error=excluded.error",params![id,step as i64,status,serde_json::to_string(receipt)?,error.map(serde_json::to_string).transpose()?])?;
        Ok(())
    }
    pub fn get_operation(&self, id: &str, step: usize) -> Result<Option<(String, Receipt)>> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT status,receipt FROM operations WHERE plan_id=?1 AND step=?2",
                params![id, step as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(s, r)| Ok((s, serde_json::from_str(&r)?)))
            .transpose()
    }
    pub fn blocked(&self, path: &Path, except: &str) -> Result<bool> {
        let mut q=self.conn.prepare("SELECT p.json FROM plans p JOIN jobs j ON j.id=p.id WHERE j.id<>?1 AND j.status IN ('applying','needs_review','failed')")?;
        let rows = q.query_map([except], |r| r.get::<_, String>(0))?;
        for row in rows {
            let p: Plan = serde_json::from_str(&row?)?;
            if p.input_snapshot.path == path
                || p.actions.iter().any(|a| {
                    a.transfer()
                        .is_some_and(|(from, to, _)| from == path || to == path)
                })
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn succeeded_input(&self, s: &Snapshot, revision: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE input_key=?1 AND revision=?2 AND status='succeeded')",params![input_key(s)?,revision],|r|r.get(0))?)
    }
    pub fn active(&self) -> Result<Vec<String>> {
        let mut q = self
            .conn
            .prepare("SELECT id FROM jobs WHERE status='applying' ORDER BY updated_at")?;
        Ok(q.query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn history(&self, limit: usize) -> Result<Value> {
        let mut q = self.conn.prepare(
            "SELECT id,rule_id,status,updated_at,error FROM jobs ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = q.query_map([limit.min(1000) as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, rule, status, time, error) = row?;
            let mut op = self.conn.prepare(
                "SELECT step,status,receipt,error FROM operations WHERE plan_id=?1 ORDER BY step",
            )?;
            let ops=op.query_map([&id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?)))?.map(|row|{let (step,status,receipt,error)=row?;Ok(json!({"step":step,"status":status,"receipt":serde_json::from_str::<Value>(&receipt).unwrap_or(Value::Null),"error":error.and_then(|s|serde_json::from_str::<Value>(&s).ok())}))}).collect::<std::result::Result<Vec<_>,rusqlite::Error>>()?;
            out.push(json!({"planId":id,"ruleId":rule,"status":status,"updatedAt":time,"error":error.and_then(|s|serde_json::from_str::<Value>(&s).ok()),"operations":ops}));
        }
        Ok(json!(out))
    }
    pub fn overview(&self) -> Result<Value> {
        let mut q = self
            .conn
            .prepare("SELECT id,status,last_scan,revision FROM sources ORDER BY id")?;
        let sources=q.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"lastScan":r.get::<_,Option<String>>(2)?,"revision":r.get::<_,String>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut q = self
            .conn
            .prepare("SELECT status,COUNT(*) FROM jobs GROUP BY status")?;
        let jobs = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<std::result::Result<std::collections::BTreeMap<_, _>, _>>()?;
        Ok(json!({"sources":sources,"jobs":jobs,"databaseVersion":1}))
    }
    pub fn suppress(&self, config: &crate::config::Loaded, s: &Snapshot) -> Result<()> {
        if let Some((source, _)) = config
            .roots
            .iter()
            .find(|(_, root)| s.path.starts_with(root))
        {
            let version = crate::snapshot::cheap_version(&s.path)?;
            let time = chrono::Utc::now().timestamp_millis();
            self.conn.execute("INSERT INTO observations VALUES (?1,?2,?3,1,?4,?4,'own_effect') ON CONFLICT(source,path) DO UPDATE SET version=excluded.version,handled=1,status='own_effect'",params![source,s.path.to_str(),version,time])?;
        }
        Ok(())
    }
}
pub fn input_key(s: &Snapshot) -> Result<String> {
    Ok(snapshot::hash(&serde_json::to_vec(&(
        s.path.clone(),
        &s.identity,
        &s.modified_ns,
        s.size_bytes,
        &s.sha256,
    ))?))
}
