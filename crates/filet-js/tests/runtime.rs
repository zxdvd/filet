use filet_core::{Evaluation, Event, FileContext, config::Package, planner::ScriptEvaluator};
use filet_js::JsEvaluator;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
fn package(code: &str) -> Package {
    Package {
        entry: "rule.mjs".into(),
        modules: BTreeMap::from([("rule.mjs".into(), code.into())]),
    }
}
fn input() -> Evaluation {
    Evaluation {
        file: FileContext {
            file_ref: "input".into(),
            name: "report.PDF".into(),
            stem: "report".into(),
            extension: "pdf".into(),
            size_bytes: 42,
            modified_at: None,
            created_at: None,
            path: Some("/inbox/report.PDF".into()),
        },
        event: Event {
            reason: "manual".into(),
        },
        now: "2026-09-22T00:00:00Z".into(),
    }
}
#[test]
fn async_structured_rule_and_host_module() {
    let p = package(
        "import {actions as a} from '@app/api'; export const apiVersion=1; export async function match(c){return c.file.extension==='pdf'} export async function actions(c){return [a.move(c.file.ref,{to:'archive'})]}",
    );
    let js = JsEvaluator::default();
    js.check(&p).unwrap();
    assert_eq!(js.evaluate(&p, &input()).unwrap().len(), 1);
}
#[test]
fn empty_run_does_not_claim_file() {
    for code in [
        "export function run(){}",
        "export function run(){return null}",
        "export function run(){return []}",
    ] {
        assert!(
            JsEvaluator::default()
                .evaluate(&package(code), &input())
                .unwrap()
                .is_empty()
        );
    }
}
#[test]
fn incompatible_exports_versions_and_truthiness_fail() {
    for code in [
        "export function run(){} export function match(){return true}",
        "export const apiVersion=2; export function run(){}",
        "export function match(){return 'yes'} export function actions(){return []}",
        "export function match(){return true} export function actions(){return null}",
        "export function run(){throw new Error('bad')}",
        "export const run=42",
    ] {
        assert!(
            JsEvaluator::default()
                .evaluate(&package(code), &input())
                .is_err(),
            "{code}"
        );
    }
}
#[test]
fn frozen_context_and_task_isolation() {
    let js = JsEvaluator::default();
    assert!(
        js.evaluate(
            &package("export function run(c){c.file.ref='other';return []}"),
            &input()
        )
        .is_err()
    );
    let p = package("let n=0;export function run(){if(++n!==1) throw Error('leaked');return []}");
    js.evaluate(&p, &input()).unwrap();
    js.evaluate(&p, &input()).unwrap();
}
#[test]
fn frozen_helpers_and_dynamic_imports() {
    let mut p =
        package("export async function run(){let m=await import('./helper.mjs'); return m.result}");
    p.modules.insert(
        "helper.mjs".into(),
        "export const result=[{copy:{to:'archive'}}]".into(),
    );
    assert_eq!(
        JsEvaluator::default().evaluate(&p, &input()).unwrap().len(),
        1
    );
}
#[test]
fn no_filesystem_node_remote_or_package_escape_imports() {
    for module in [
        "node:fs",
        "https://example.com/rule.mjs",
        "../outside.mjs",
        "fs",
    ] {
        let p = package(&format!(
            "import x from '{module}';export function run(){{return []}}"
        ));
        assert!(JsEvaluator::default().check(&p).is_err());
    }
}
#[test]
fn infinite_loop_unsettled_promise_and_output_budget_are_bounded() {
    let js = JsEvaluator {
        timeout: Duration::from_millis(40),
        ..Default::default()
    };
    for code in [
        "export function run(){while(true){}}",
        "export async function run(){await new Promise(()=>{})}",
        "export function run(){return Array(65).fill({copy:{to:'x'}})}",
    ] {
        let start = Instant::now();
        assert!(js.evaluate(&package(code), &input()).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
#[test]
fn memory_limit_is_effective_with_selected_features() {
    let js = JsEvaluator {
        memory_bytes: 2 * 1024 * 1024,
        timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let start = Instant::now();
    assert!(js.evaluate(&package("export function run(){const x=[];for(let i=0;i<10000000;i++)x.push({value:i});return []}"),&input()).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn runtime_action_schema_rejects_invalid_ref_and_unknown_properties() {
    for code in [
        "export function run(){return [{move:{to:'x',file:'someone-else'}}]}",
        "export function run(){return [{move:{to:'x',overwrite:true}}]}",
        "export function run(){return [{delete:{}}]}",
    ] {
        assert!(
            JsEvaluator::default()
                .evaluate(&package(code), &input())
                .is_err()
        );
    }
}

#[test]
fn yaml_and_js_produce_equivalent_concrete_actions() {
    let t = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(t.path()).unwrap();
    std::fs::create_dir(root.join("inbox")).unwrap();
    std::fs::write(root.join("inbox/a.pdf"), b"a").unwrap();
    std::fs::write(root.join("filet.yaml"),"schemaVersion: 1\nsources:\n  inbox:\n    path: inbox\nrules:\n  - id: pdf\n    on:\n      type: file.ready\n      source: inbox\n    actions:\n      - copy:\n          to: backup\n      - rename:\n          name: renamed.pdf\n      - move:\n          to: archive\n").unwrap();
    let config = filet_core::config::Loaded::load(&root.join("filet.yaml")).unwrap();
    let yaml = filet_core::planner::plan(
        &config,
        &JsEvaluator::default(),
        &root.join("inbox/a.pdf"),
        "manual",
        None,
    )
    .unwrap()
    .unwrap();
    let p = package(
        "import {actions as a} from '@app/api';export function run(c){return [a.copy(c.file.ref,{to:'backup'}),a.rename(c.file.ref,{name:'renamed.pdf'}),a.move(c.file.ref,{to:'archive'})]}",
    );
    let drafts = JsEvaluator::default().evaluate(&p, &input()).unwrap();
    let js = filet_core::planner::resolve(&root, &root.join("inbox/a.pdf"), drafts).unwrap();
    assert_eq!(
        serde_json::to_value(yaml.actions).unwrap(),
        serde_json::to_value(js).unwrap()
    );
}
#[test]
fn endless_microtask_chain_has_a_deadline() {
    let js = JsEvaluator {
        timeout: Duration::from_millis(40),
        ..Default::default()
    };
    let p = package("export async function run(){while(true)await Promise.resolve()}");
    let start = Instant::now();
    assert!(js.evaluate(&p, &input()).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
}
