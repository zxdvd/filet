use filet_core::{
    Draft, Error, Evaluation, Result,
    config::Package,
    planner::{ScriptEvaluator, validate_drafts},
};
use rquickjs::{
    CatchResultExt, Context, Ctx, FromJs, Function, Module, Promise, Runtime,
    loader::{ImportAttributes, Loader, Resolver},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const API: &str = include_str!("api.mjs");
const RUNNER: &str = include_str!("runner.mjs");

#[derive(Clone)]
pub struct JsEvaluator {
    pub timeout: Duration,
    pub memory_bytes: usize,
    pub stack_bytes: usize,
}
impl Default for JsEvaluator {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(2),
            memory_bytes: 32 * 1024 * 1024,
            stack_bytes: 512 * 1024,
        }
    }
}
struct SnapshotResolver {
    entry: String,
}
impl Resolver for SnapshotResolver {
    fn resolve<'js>(
        &mut self,
        _: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if attributes.is_some() {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "import attributes are unsupported",
            ));
        }
        if name == "@app/api" {
            return Ok(name.into());
        }
        if base == "filet:runner" && name == "filet:entry" {
            return Ok(self.entry.clone());
        }
        if !name.starts_with("./") && !name.starts_with("../") {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "only package-relative imports and @app/api are allowed",
            ));
        }
        let mut parts: Vec<&str> = base.split('/').collect();
        parts.pop();
        for p in name.split('/') {
            match p {
                "." | "" => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(rquickjs::Error::new_resolving(base, name));
                    }
                }
                p => parts.push(p),
            }
        }
        Ok(parts.join("/"))
    }
}
struct SnapshotLoader {
    modules: BTreeMap<String, String>,
}
fn settle<'js, T: FromJs<'js>>(
    ctx: &Ctx<'js>,
    promise: &Promise<'js>,
    started: Instant,
    limit: Duration,
) -> rquickjs::Result<T> {
    loop {
        if started.elapsed() >= limit {
            return Err(rquickjs::Error::WouldBlock);
        }
        if let Some(result) = promise.result::<T>() {
            return result;
        }
        if !ctx.execute_pending_job() {
            return Err(rquickjs::Error::WouldBlock);
        }
    }
}
impl Loader for SnapshotLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        let text = if name == "@app/api" {
            API
        } else {
            self.modules
                .get(name)
                .map(String::as_str)
                .ok_or_else(|| rquickjs::Error::new_loading(name))?
        };
        Module::declare(ctx.clone(), name, text)
    }
}
impl JsEvaluator {
    fn execute(&self, package: &Package, input: Option<&Evaluation>) -> Result<Vec<Draft>> {
        let started = Instant::now();
        let limit = self.timeout;
        let runtime = Runtime::new().map_err(|e| Error::new("JS_RUNTIME", e.to_string()))?;
        runtime.set_memory_limit(self.memory_bytes);
        runtime.set_max_stack_size(self.stack_bytes);
        runtime.set_interrupt_handler(Some(Box::new(move || started.elapsed() >= limit)));
        runtime.set_loader(
            SnapshotResolver {
                entry: package.entry.clone(),
            },
            SnapshotLoader {
                modules: package.modules.clone(),
            },
        );
        let context =
            Context::full(&runtime).map_err(|e| Error::new("JS_RUNTIME", e.to_string()))?;
        context.with(|ctx| {
            let result = (|| -> rquickjs::Result<String> {
                let module = Module::declare(ctx.clone(), "filet:runner", RUNNER)?;
                let (module, promise) = module.eval()?;
                settle::<()>(&ctx, &promise, started, limit)?;
                if let Some(input) = input {
                    let function: Function = module.get("evaluate")?;
                    let promise: Promise = function
                        .call((serde_json::to_string(input).expect("serializable evaluation"),))?;
                    settle::<String>(&ctx, &promise, started, limit)
                } else {
                    Ok("[]".into())
                }
            })();
            let json = result.catch(&ctx).map_err(|e| {
                Error::new(
                    if started.elapsed() >= limit {
                        "JS_TIMEOUT"
                    } else {
                        "JS_ERROR"
                    },
                    e.to_string(),
                )
            })?;
            if json.len() > 1_048_576 {
                return Err(Error::new("JS_OUTPUT_LIMIT", "script result exceeds 1 MiB"));
            }
            let drafts: Vec<Draft> = serde_json::from_str(&json)
                .map_err(|e| Error::new("INVALID_ACTION", e.to_string()))?;
            validate_drafts(&drafts)?;
            Ok(drafts)
        })
    }
}
impl ScriptEvaluator for JsEvaluator {
    fn check(&self, package: &Package) -> Result<()> {
        self.execute(package, None).map(|_| ())
    }
    fn evaluate(&self, package: &Package, input: &Evaluation) -> Result<Vec<Draft>> {
        self.execute(package, Some(input))
    }
}
