use crate::{Draft, Error, Result, paths, snapshot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    pub schema_version: u32,
    pub sources: BTreeMap<String, Source>,
    pub rules: Vec<Rule>,
}
fn ignore() -> Vec<String> {
    ["**/*.crdownload", "**/*.part", "**/*.tmp", "**/.filet-*"]
        .iter()
        .map(|s| (*s).into())
        .collect()
}
fn reconcile() -> String {
    "10m".into()
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Source {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default = "ignore")]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub ready: Ready,
    #[serde(default = "reconcile")]
    pub reconcile_every: String,
}
fn stable() -> String {
    "3s".into()
}
fn retry() -> String {
    "10m".into()
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Ready {
    #[serde(default = "stable")]
    pub stable_for: String,
    #[serde(default = "retry")]
    pub retry_for: String,
}
impl Default for Ready {
    fn default() -> Self {
        Self {
            stable_for: stable(),
            retry_for: retry(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Trigger {
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    pub every: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Script {
    pub file: String,
    pub api_version: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub on: Trigger,
    pub when: Option<Condition>,
    pub actions: Option<Vec<Draft>>,
    pub script: Option<Script>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Condition {
    pub extension: Option<String>,
    pub name_glob: Option<String>,
    pub min_size_bytes: Option<u64>,
    pub max_size_bytes: Option<u64>,
    pub modified_older_than: Option<String>,
    pub created_older_than: Option<String>,
    pub all: Option<Vec<Condition>>,
    pub any: Option<Vec<Condition>>,
    pub not: Option<Box<Condition>>,
}
#[derive(Debug, Clone)]
pub struct Package {
    pub entry: String,
    pub modules: BTreeMap<String, String>,
}
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    pub path: PathBuf,
    pub base: PathBuf,
    pub revision: String,
    pub roots: BTreeMap<String, PathBuf>,
    pub packages: BTreeMap<String, Package>,
}
pub fn duration(s: &str) -> Result<Duration> {
    let (n, mult) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60_000)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3_600_000)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, 86_400_000)
    } else {
        return Err(Error::new(
            "CONFIG_INVALID",
            format!("invalid duration: {s}"),
        ));
    };
    let n = n
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .and_then(|n| n.checked_mul(mult))
        .ok_or_else(|| {
            Error::new(
                "CONFIG_INVALID",
                format!("positive integer duration required: {s}"),
            )
        })?;
    Ok(Duration::from_millis(n))
}
fn check_yaml(value: &yaml_serde::Value, depth: usize) -> Result<()> {
    use yaml_serde::Value;
    if depth > 24 {
        return Err(Error::new("CONFIG_INVALID", "YAML nesting exceeds 24"));
    }
    match value {
        Value::Tagged(_) => return Err(Error::new("CONFIG_INVALID", "YAML tags are unsupported")),
        Value::Mapping(map) => {
            for (k, v) in map {
                let key = k
                    .as_str()
                    .ok_or_else(|| Error::new("CONFIG_INVALID", "mapping keys must be strings"))?;
                if key == "<<" {
                    return Err(Error::new(
                        "CONFIG_INVALID",
                        "YAML merge keys are unsupported",
                    ));
                }
                check_yaml(v, depth + 1)?;
            }
        }
        Value::Sequence(v) => {
            for x in v {
                check_yaml(x, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn validate_condition(c: &Condition) -> Result<()> {
    if c.extension
        .as_ref()
        .is_some_and(|x| x.starts_with('.') || !x.is_ascii() || *x != x.to_ascii_lowercase())
    {
        return Err(Error::new(
            "CONFIG_INVALID",
            "extension must be ASCII lowercase without a dot",
        ));
    }
    if let Some(g) = &c.name_glob {
        globset::Glob::new(g).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
    }
    for d in [&c.modified_older_than, &c.created_older_than]
        .into_iter()
        .flatten()
    {
        duration(d)?;
    }
    if c.min_size_bytes
        .zip(c.max_size_bytes)
        .is_some_and(|(a, b)| a > b)
    {
        return Err(Error::new(
            "CONFIG_INVALID",
            "minSizeBytes exceeds maxSizeBytes",
        ));
    }
    for v in [&c.all, &c.any].into_iter().flatten() {
        for child in v {
            validate_condition(child)?;
        }
    }
    if let Some(child) = &c.not {
        validate_condition(child)?;
    }
    Ok(())
}
impl Loaded {
    pub fn load(path: &Path) -> Result<Self> {
        let path = paths::expand(
            &std::env::current_dir()?,
            path.to_str()
                .ok_or_else(|| Error::new("NON_UTF8_PATH", "config path is not Unicode"))?,
        )?;
        let base = path.parent().unwrap().to_path_buf();
        if fs::metadata(&path)?.len() > 1_048_576 {
            return Err(Error::new("CONFIG_INVALID", "configuration exceeds 1 MiB"));
        }
        let text = fs::read_to_string(&path)?;
        let value: yaml_serde::Value =
            yaml_serde::from_str(&text).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        check_yaml(&value, 0)?;
        // Deserialize the original stream too: preserves duplicate-field checks and line/column errors.
        let config: Config =
            yaml_serde::from_str(&text).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        if config.schema_version != 1 {
            return Err(Error::new("UNSUPPORTED_VERSION", "schemaVersion must be 1"));
        }
        if config.sources.is_empty() || config.rules.len() > 256 {
            return Err(Error::new(
                "CONFIG_INVALID",
                "require sources and at most 256 rules",
            ));
        }
        let mut roots = BTreeMap::new();
        for (id, s) in &config.sources {
            if id.is_empty() {
                return Err(Error::new("CONFIG_INVALID", "empty source ID"));
            }
            let root = paths::expand(&base, &s.path)?;
            if root.parent().is_none() {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    "filesystem roots are not valid sources",
                ));
            }
            duration(&s.ready.stable_for)?;
            duration(&s.ready.retry_for)?;
            duration(&s.reconcile_every)?;
            if duration(&s.ready.retry_for)? < duration(&s.ready.stable_for)? {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    "retryFor must be at least stableFor",
                ));
            }
            for g in &s.ignore {
                globset::Glob::new(g).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
            }
            roots.insert(id.clone(), root);
        }
        // Overlapping sources make ownership and self-event suppression ambiguous.
        let root_values: Vec<_> = roots.values().collect();
        if roots.values().any(|root| path.starts_with(root)) {
            return Err(Error::new(
                "CONFIG_INVALID",
                "configuration must be outside watched sources",
            ));
        }
        for (i, a) in root_values.iter().enumerate() {
            for b in root_values.iter().skip(i + 1) {
                if a.starts_with(b) || b.starts_with(a) {
                    return Err(Error::new(
                        "CONFIG_INVALID",
                        "source directories must not overlap",
                    ));
                }
            }
        }
        let mut ids = BTreeSet::new();
        let mut packages = BTreeMap::new();
        for r in &config.rules {
            if r.id.is_empty() || !ids.insert(&r.id) {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    "rule IDs must be nonempty and unique",
                ));
            }
            if !roots.contains_key(&r.on.source) {
                return Err(Error::new("CONFIG_INVALID", "unknown source").rule(&r.id));
            }
            match r.on.kind.as_str() {
                "file.ready" if r.on.every.is_none() => {}
                "scan" if r.on.every.is_some() => {
                    duration(r.on.every.as_ref().unwrap())?;
                }
                _ => {
                    return Err(Error::new(
                        "CONFIG_INVALID",
                        "file.ready has no every; scan requires every",
                    )
                    .rule(&r.id));
                }
            }
            match (&r.script, &r.actions) {
                (Some(script), None) if r.when.is_none() => {
                    if script.api_version != 1 {
                        return Err(
                            Error::new("UNSUPPORTED_VERSION", "apiVersion must be 1").rule(&r.id)
                        );
                    }
                    let entry = paths::expand(&base, &script.file)?;
                    if !matches!(
                        entry.extension().and_then(|v| v.to_str()),
                        Some("js" | "mjs")
                    ) {
                        return Err(Error::new("CONFIG_INVALID", "scripts must use .js or .mjs"));
                    }
                    let root = entry.parent().unwrap();
                    if roots.values().any(|p| root.starts_with(p)) {
                        return Err(Error::new(
                            "CONFIG_INVALID",
                            "script packages must be outside sources",
                        ));
                    }
                    let mut modules = BTreeMap::new();
                    let mut bytes = 0;
                    for item in walkdir::WalkDir::new(root).follow_links(false).max_open(16) {
                        let item = item.map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
                        if item.file_type().is_symlink() {
                            return Err(Error::new(
                                "UNSUPPORTED_PATH",
                                "symlinks in rule packages are unsupported",
                            ));
                        }
                        if item.file_type().is_file()
                            && matches!(
                                item.path().extension().and_then(|v| v.to_str()),
                                Some("js" | "mjs")
                            )
                        {
                            let len = item
                                .metadata()
                                .map_err(|e| Error::new("IO_ERROR", e.to_string()))?
                                .len();
                            if len > 1_048_576 || bytes + len > 4_194_304 || modules.len() >= 128 {
                                return Err(Error::new(
                                    "SCRIPT_LIMIT",
                                    "rule package exceeds source budget",
                                ));
                            }
                            bytes += len;
                            let name = item
                                .path()
                                .strip_prefix(root)
                                .unwrap()
                                .to_str()
                                .ok_or_else(|| Error::new("NON_UTF8_PATH", "module path"))?
                                .replace('\\', "/");
                            modules.insert(name, fs::read_to_string(item.path())?);
                        }
                    }
                    packages.insert(
                        r.id.clone(),
                        Package {
                            entry: entry.file_name().unwrap().to_str().unwrap().into(),
                            modules,
                        },
                    );
                }
                (None, Some(actions)) => {
                    if actions.len() > 64 {
                        return Err(Error::new("ACTION_LIMIT", "at most 64 actions per rule"));
                    }
                    if let Some(c) = &r.when {
                        validate_condition(c)?;
                    }
                    crate::planner::validate_drafts(actions)?;
                }
                _ => {
                    return Err(
                        Error::new("CONFIG_INVALID", "use either script or when/actions")
                            .rule(&r.id),
                    );
                }
            }
        }
        let mut revision_bytes = serde_json::to_vec(&config)?;
        revision_bytes.extend(serde_json::to_vec(&roots)?);
        for (id, p) in &packages {
            revision_bytes.extend(serde_json::to_vec(&(id, &p.entry, &p.modules))?);
        }
        let revision = snapshot::hash(&revision_bytes);
        Ok(Self {
            config,
            path,
            base,
            revision,
            roots,
            packages,
        })
    }
    pub fn check_layout(&self, data: &Path) -> Result<()> {
        if self.roots.values().any(|root| data.starts_with(root)) {
            return Err(Error::new(
                "CONFIG_INVALID",
                "data directory must be outside watched sources",
            ));
        }
        Ok(())
    }
}
