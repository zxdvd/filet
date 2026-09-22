use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    #[default]
    Error,
    Skip,
}

fn input_ref() -> String {
    "input".into()
}
fn timeout() -> String {
    "30s".into()
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Transfer {
    #[serde(default = "input_ref")]
    pub file: String,
    pub to: String,
    #[serde(default)]
    pub on_conflict: Conflict,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Rename {
    #[serde(default = "input_ref")]
    pub file: String,
    pub name: String,
    #[serde(default)]
    pub on_conflict: Conflict,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PathArg {
    pub path_of: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(untagged)]
pub enum Arg {
    Literal(String),
    Path(PathArg),
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Exec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<Arg>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Start from an empty environment. These explicit values are the complete child environment.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "timeout")]
    pub timeout: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Draft {
    Copy(Transfer),
    Move(Transfer),
    Rename(Rename),
    Exec(Exec),
}
// YAML v1 uses one-key mappings, not YAML's native enum tags (!move).
impl<'de> Deserialize<'de> for Draft {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let map = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        if map.len() != 1 {
            return Err(D::Error::custom("an action must have exactly one key"));
        }
        let (kind, value) = map.into_iter().next().unwrap();
        match kind.as_str() {
            "copy" => serde_json::from_value(value).map(Self::Copy),
            "move" => serde_json::from_value(value).map(Self::Move),
            "rename" => serde_json::from_value(value).map(Self::Rename),
            "exec" => serde_json::from_value(value).map(Self::Exec),
            _ => {
                return Err(D::Error::custom(
                    "unknown action; expected copy/move/rename/exec",
                ));
            }
        }
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Snapshot {
    pub path: PathBuf,
    pub identity: String,
    pub size_bytes: u64,
    pub modified_ns: String,
    pub modified_at: Option<String>,
    pub created_at: Option<String>,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Action {
    Copy {
        from: PathBuf,
        to: PathBuf,
        conflict: Conflict,
    },
    Move {
        from: PathBuf,
        to: PathBuf,
        conflict: Conflict,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        conflict: Conflict,
    },
    Exec {
        program: PathBuf,
        args: Vec<Arg>,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
        timeout_ms: u64,
    },
}
impl Action {
    pub fn transfer(&self) -> Option<(&PathBuf, &PathBuf, &Conflict)> {
        match self {
            Self::Copy { from, to, conflict }
            | Self::Move { from, to, conflict }
            | Self::Rename { from, to, conflict } => Some((from, to, conflict)),
            _ => None,
        }
    }
    pub fn is_move(&self) -> bool {
        matches!(self, Self::Move { .. } | Self::Rename { .. })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Plan {
    pub format_version: u32,
    pub plan_id: String,
    pub rule_id: String,
    pub rule_revision: String,
    pub source: String,
    pub input_snapshot: Snapshot,
    pub actions: Vec<Action>,
    pub risks: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
    pub file: FileContext,
    pub event: Event,
    pub now: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileContext {
    #[serde(rename = "ref")]
    pub file_ref: String,
    pub name: String,
    pub stem: String,
    pub extension: String,
    pub size_bytes: u64,
    pub modified_at: Option<String>,
    pub created_at: Option<String>,
    pub path: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub before: Snapshot,
    pub after: Option<Snapshot>,
    pub temp: Option<PathBuf>,
    pub skipped: bool,
    pub output: Option<serde_json::Value>,
}
