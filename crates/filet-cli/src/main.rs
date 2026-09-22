use clap::{Parser, Subcommand};
use filet_core::{
    Draft, Error, Result,
    config::Loaded,
    daemon::Engine,
    executor, paths, planner,
    state::{ExecutionLock, Store},
};
use filet_js::JsEvaluator;
use notify::{RecursiveMode, Watcher};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(name = "filet", version, about = "Explainable local file automation")]
struct Cli {
    #[arg(short, long, default_value = "filet.yaml", global = true)]
    config: PathBuf,
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Emit authoritative JSON schemas for configuration and persisted plans.
    Schema,
    /// Validate schema, script syntax, versions and exported entry points.
    Check,
    /// Generate and persist an offline action plan without executing its actions.
    Plan { path: PathBuf },
    /// Apply an existing plan after revalidating its preconditions.
    Apply { plan_id: String },
    /// Evaluate JSON fixtures without executing or persisting actions.
    Test { fixtures: PathBuf },
    /// Watch, reconcile, schedule and execute in the foreground. Ctrl-C stops cleanly.
    Daemon,
    /// Show persisted source and job state.
    Status,
    /// Show operation receipts, errors and command output.
    History {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Inspect source paths, native capabilities and resolved executables.
    Doctor,
}
fn envelope(result: Result<Value>) -> Value {
    match result {
        Ok(data) => json!({"formatVersion":1,"ok":true,"data":data,"errors":[]}),
        Err(e) => json!({"formatVersion":1,"ok":false,"data":null,"errors":[e]}),
    }
}
fn main() {
    // Clap errors also respect --json; successful --help/--version remain ordinary text.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if std::env::args().any(|s| s == "--json") && e.exit_code() != 0 {
                println!("{}", envelope(Err(Error::new("CLI_USAGE", e.to_string()))));
                std::process::exit(2);
            }
            e.exit();
        }
    };
    let json_mode = cli.json;
    let result = run(cli);
    let success = result.is_ok();
    let output = envelope(result);
    if json_mode {
        println!("{output}");
    } else {
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    }
    if !success {
        std::process::exit(1);
    }
}
fn data_dir(cli: &Cli) -> Result<PathBuf> {
    let p = if let Some(p) = &cli.data_dir {
        p.clone()
    } else {
        directories::ProjectDirs::from("dev", "filet", "filet")
            .ok_or_else(|| Error::new("DATA_DIR", "cannot resolve data directory"))?
            .data_local_dir()
            .to_path_buf()
    };
    paths::expand(
        &std::env::current_dir()?,
        p.to_str()
            .ok_or_else(|| Error::new("NON_UTF8_PATH", "data directory"))?,
    )
}
fn run(cli: Cli) -> Result<Value> {
    if matches!(&cli.command, Commands::Schema) {
        return Ok(filet_core::schemas());
    }
    let dir = data_dir(&cli)?;
    // Diagnostics do not depend on the currently edited config being valid.
    if matches!(&cli.command, Commands::Status | Commands::History { .. }) {
        let store = Store::open(&dir)?;
        return match cli.command {
            Commands::Status => store.overview(),
            Commands::History { limit } => store.history(limit),
            _ => unreachable!(),
        };
    }
    let mut config = Loaded::load(&cli.config)?;
    config.check_layout(&dir)?;
    let js = JsEvaluator::default();
    match cli.command {
        Commands::Check => {
            planner::check(&config, &js)?;
            Ok(
                json!({"revision":config.revision,"rules":config.config.rules.len(),"schemaVersion":1,"jsApiVersion":1}),
            )
        }
        Commands::Plan { path } => {
            planner::check(&config, &js)?;
            let plan = planner::plan(&config, &js, &path, "manual", None)?;
            if let Some(p) = &plan {
                Store::open(&dir)?.save(p)?;
            }
            Ok(json!({"plan":plan}))
        }
        Commands::Apply { plan_id } => {
            // Never run rule code during apply, including check(): execute only the saved action data.
            let _lock = ExecutionLock::acquire(&dir)?;
            let store = Store::open(&dir)?;
            executor::apply(&store, &config, &plan_id)
        }
        Commands::Test { fixtures } => fixture_test(&config, &js, &fixtures),
        Commands::Doctor => {
            planner::check(&config, &js)?;
            let sources: Vec<_> = config
                .roots
                .iter()
                .map(|(id, path)| json!({"id":id,"path":path,"available":path.is_dir()}))
                .collect();
            let programs:Vec<_>=config.config.rules.iter().flat_map(|r|r.actions.iter().flatten()).filter_map(|a|if let Draft::Exec(e)=a{Some(json!({"program":e.program,"resolved":paths::resolve_program(&config.base,&e.program).map(|p|json!(p)).unwrap_or(Value::Null)}))}else{None}).collect();
            let store = Store::open(&dir)?;
            let db = store
                .conn
                .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))?;
            Ok(
                json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"dataDir":dir,"sources":sources,"executables":programs,"database":db,"revision":config.revision,"nativePublish":"hard-link no-clobber; unsupported filesystems fail closed","jsMemoryBytes":js.memory_bytes,"jsTimeoutMs":js.timeout.as_millis()}),
            )
        }
        Commands::Daemon => {
            planner::check(&config, &js)?;
            let _lock = ExecutionLock::acquire(&dir)?;
            let store = Store::open(&dir)?;
            eprintln!(
                "{}",
                json!({"event":"recovery","results":executor::recover(&store,&config)?})
            );
            let running = Arc::new(AtomicBool::new(true));
            let stop = running.clone();
            ctrlc::set_handler(move || stop.store(false, Ordering::SeqCst))
                .map_err(|e| Error::new("SIGNAL_ERROR", e.to_string()))?;
            // A coalescing bit is the bounded hint queue. Overflow cannot hide files: scan the source.
            let dirty = Arc::new(AtomicBool::new(true));
            let mut watcher = make_watcher(&config, dirty.clone())?;
            let mut engine = Engine::new();
            let mut reload = Instant::now();
            let mut rejected = None;
            while running.load(Ordering::SeqCst) {
                if dirty.swap(false, Ordering::SeqCst) {
                    engine.mark_dirty();
                }
                if reload.elapsed() >= Duration::from_secs(2) {
                    reload = Instant::now();
                    match Loaded::load(&config.path).and_then(|c| {
                        c.check_layout(&dir)?;
                        if c.revision != config.revision {
                            planner::check(&c, &js)?;
                        }
                        Ok(c)
                    }) {
                        Ok(next) if next.revision != config.revision => {
                            // Build and register the complete replacement before swapping the active revision.
                            match make_watcher(&next, dirty.clone()) {
                                Ok(next_watcher) => {
                                    watcher = next_watcher;
                                    config = next;
                                    engine = Engine::new();
                                    rejected = None;
                                    eprintln!(
                                        "{}",
                                        json!({"event":"config_activated","revision":config.revision})
                                    );
                                }
                                Err(e) => {
                                    eprintln!("{}", json!({"event":"reload_rejected","error":e}))
                                }
                            }
                        }
                        Ok(_) => {
                            rejected = None;
                        }
                        Err(e) => {
                            let signature = e.to_string();
                            if rejected.as_ref() != Some(&signature) {
                                eprintln!("{}", json!({"event":"reload_rejected","error":e}));
                                rejected = Some(signature);
                            }
                        }
                    }
                }
                for event in engine.tick(&store, &config, &js)? {
                    eprintln!("{event}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            drop(watcher);
            Ok(json!({"status":"stopped"}))
        }
        Commands::Schema | Commands::Status | Commands::History { .. } => unreachable!(),
    }
}
fn make_watcher(config: &Loaded, dirty: Arc<AtomicBool>) -> Result<notify::RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        dirty.store(true, Ordering::SeqCst);
        match result {
            Ok(e) => eprintln!(
                "{}",
                json!({"event":"watch_hint","kind":format!("{:?}",e.kind),"paths":e.paths})
            ),
            Err(e) => eprintln!("{}", json!({"event":"watch_error","message":e.to_string()})),
        }
    })
    .map_err(|e| Error::new("WATCH_ERROR", e.to_string()))?;
    for (id, path) in &config.roots {
        if let Err(e) = watcher.watch(
            path,
            if config.config.sources[id].recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            },
        ) {
            // Offline roots and unavailable native watches still get periodic reconciliation.
            eprintln!(
                "{}",
                json!({"event":"watch_unavailable","source":id,"message":e.to_string()})
            );
        }
    }
    Ok(watcher)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Fixture {
    path: PathBuf,
    expected_rule: Option<String>,
    expected_action_count: usize,
}
fn fixture_test(config: &Loaded, js: &JsEvaluator, path: &PathBuf) -> Result<Value> {
    planner::check(config, js)?;
    let fixtures: Vec<Fixture> = serde_json::from_slice(&std::fs::read(path)?)?;
    let base = std::fs::canonicalize(path)?.parent().unwrap().to_path_buf();
    let mut results = Vec::new();
    for f in fixtures {
        let input = base.join(&f.path);
        let plan = planner::plan(config, js, &input, "manual", None)?;
        let actual = plan.as_ref().map(|p| p.rule_id.as_str());
        let count = plan.as_ref().map_or(0, |p| p.actions.len());
        if actual != f.expected_rule.as_deref() || count != f.expected_action_count {
            return Err(Error::new(
                "FIXTURE_FAILED",
                format!(
                    "{}: rule={actual:?}, action count={count}",
                    f.path.display()
                ),
            ));
        }
        results.push(json!({"path":f.path,"passed":true}));
    }
    Ok(json!({"fixtures":results}))
}
