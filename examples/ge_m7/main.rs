//! GE-M7, stage M7-R (RFC SMYSL-10): does an agent doing root-cause analysis do it better when it
//! writes into smysl? The harness: RCAEval RE2 cases, four arms, one oracle.
//!
//! - **(a)** the agent alone: tools return text, nothing is recorded;
//! - **(b)** the agent with smysl: each tool's units are committed with `consulted`, each
//!   hypothesis is a unit with grounds and quotes, and `hypothesise` answers with its effective
//!   status, rule TQ's `SMY-W606` per quote, and kind 6;
//! - **(c)** BARO's ranking alone;
//! - **(d)** (b) with BARO's ranking in the ledger and listed in the first message;
//! - **(e)** (d)'s run, its answer completed from BARO's ranking (RFC SMYSL-10 A4-2): no run of
//!   its own, and the agent is not told.
//!
//! A run that spends its budget without answering gets one forced step whose only branch is
//! `answer` (A4-3); its score records `forced`.
//!
//! **Modes:**
//!
//! - `run --data DIR --cases FILE [--arms a,b,c,d,e] [--model M] [--endpoint URL] [--steps 25]
//!   [--ctx 32768] [--shuffle SEED] [--out DIR]`: each case × arm, a JSON line per run in `DIR/runs.jsonl`, the
//!   transcripts beside it, a table at the end. `--shuffle SEED` runs the cases in a seeded
//!   order: a run's result depends on what the model server ran before it (A4-10). `FILE` lists cases as `<system>/<service>_<fault>/<rep>`.
//! - `check-data --data DIR`: what R1 left to the data (R1-3, R1-7): the files per system, and
//!   the leakage check over every case: no tool result names the root cause's service by any
//!   spelling.
//! - `sample --data DIR [--rep N]`: the scored sample and the pilot's six, as `fixtures/ge_m7/`
//!   lists them; with `--rep 3`, the held-out cases of A4-1 (`held-out.txt`).
//!
//! The data is not in the repository (CC BY 4.0, about 32 GB extracted): `make ge-m7-data` names
//! where it comes from. The fixtures under `fixtures/ge_m7/` are two synthetic cases in RE2's
//! layout, for the test below, which runs every arm with a scripted agent and no model.

mod agent;
mod case;
mod oracle;
mod world;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use smysl::{AgentId, ProviderConfig, ProviderId};

use agent::{Agent, Loop, ModelAgent, Run};
use case::Raw;
use world::{World, CASE_S};

/// One run's scores (§6).
#[derive(Debug, Clone, Default)]
struct Score {
    case: String,
    arm: String,
    model: String,
    root: String,
    fault: String,
    ranking: Vec<String>,
    /// (c), (d), (e): BARO's ranking.
    baro: Vec<String>,
    /// The answer came from the forced step (A4-3).
    forced: bool,
    /// 1-based rank of the root cause, 0 when absent.
    rank: usize,
    claims: usize,
    fabricated: usize,
    fabricated_strict: usize,
    /// (b), (d): quotes in hypotheses, and rule TQ against the oracle.
    tq_quotes: usize,
    tq_tp: usize,
    tq_fp: usize,
    tq_fn: usize,
    consulted: usize,
    cited: usize,
    hypotheses: usize,
    steps: usize,
    protocol: Option<String>,
    arg_errors: usize,
    capped: usize,
    input_tokens: u64,
    output_tokens: u64,
    secs: f64,
    calls: BTreeMap<String, usize>,
    /// Every quote the oracle calls fabricated, or not TQ's spelling: where, the quote, why.
    verdicts: Vec<serde_json::Value>,
}

impl Score {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "case": self.case, "arm": self.arm, "model": self.model, "root": self.root, "fault": self.fault,
            "ranking": self.ranking, "baro": self.baro, "forced": self.forced, "rank": self.rank, "claims": self.claims,
            "fabricated": self.fabricated, "fabricated_strict": self.fabricated_strict,
            "tq_quotes": self.tq_quotes, "tq_tp": self.tq_tp, "tq_fp": self.tq_fp,
            "tq_fn": self.tq_fn, "consulted": self.consulted, "cited": self.cited,
            "hypotheses": self.hypotheses, "steps": self.steps, "protocol": self.protocol,
            "arg_errors": self.arg_errors, "capped": self.capped,
            "input_tokens": self.input_tokens, "output_tokens": self.output_tokens,
            "secs": self.secs, "calls": self.calls, "verdicts": self.verdicts,
        })
    }
}

/// `cases` in an order seeded by `seed` (Fisher–Yates over `case::Rng`).
fn shuffled(mut cases: Vec<String>, seed: &str) -> Vec<String> {
    let mut rng = case::Rng(case::fnv(seed));
    for i in (1..cases.len()).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        cases.swap(i, j);
    }
    cases
}

/// `answer` followed by `baro`, repeats removed, the first five kept (A4-2).
fn complete(answer: &[String], baro: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in answer.iter().chain(baro) {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    out.truncate(5);
    out
}

/// Arm (e)'s score from (d)'s: the ranking completed, everything else (d)'s.
fn completed(d: &Score) -> Score {
    let ranking = complete(&d.ranking, &d.baro);
    Score {
        arm: "e".into(),
        rank: ranking
            .iter()
            .position(|r| *r == d.root)
            .map_or(0, |i| i + 1),
        ranking,
        ..d.clone()
    }
}

fn note(out: &mut Vec<serde_json::Value>, at: &str, q: &world::Quote, v: &oracle::Verdict) {
    if v.fabricated || !v.strict {
        out.push(serde_json::json!({
            "at": at, "quote": q.text(), "fabricated": v.fabricated, "strict": v.strict, "why": v.why,
        }));
    }
}

fn score(world: &World, arm: &str, model: &str, run: Option<&Run>, ranking: Vec<String>) -> Score {
    let root = world
        .anon
        .to
        .get(&world.raw.root)
        .cloned()
        .unwrap_or_default();
    let mut s = Score {
        case: format!("{}/{}", world.raw.system, world.raw.rel),
        arm: arm.into(),
        model: model.into(),
        rank: ranking.iter().position(|r| *r == root).map_or(0, |i| i + 1),
        root,
        fault: world.raw.fault.clone(),
        ranking,
        ..Score::default()
    };
    let Some(r) = run else { return s };
    if let Some(a) = &r.answer {
        for q in &a.quotes {
            let v = oracle::judge(world, q);
            note(&mut s.verdicts, "answer", q, &v);
            s.claims += 1;
            s.fabricated += usize::from(v.fabricated);
            s.fabricated_strict += usize::from(!v.strict);
        }
    }
    for w in &world.written {
        s.hypotheses += 1;
        if w.uid.is_none() {
            continue;
        }
        for (q, tq) in w.quotes.iter().zip(&w.tq) {
            let v = oracle::judge(world, q);
            note(&mut s.verdicts, "hypothesis", q, &v);
            s.tq_quotes += 1;
            match (tq.is_some(), v.fabricated) {
                (true, true) => s.tq_tp += 1,
                (true, false) => s.tq_fp += 1,
                (false, true) => s.tq_fn += 1,
                (false, false) => {}
            }
        }
    }
    (s.consulted, s.cited) = world.evidence_use();
    s.steps = r.steps;
    s.protocol = r.protocol.clone();
    s.arg_errors = r.arg_errors;
    s.capped = r.capped;
    s.input_tokens = r.input_tokens;
    s.output_tokens = r.output_tokens;
    s.secs = r.secs;
    s.calls = r.calls.clone();
    s.forced = r.forced;
    s
}

/// The alert, the first message of every arm.
fn alert(world: &World) -> String {
    format!(
        "Alert: something is wrong in the system from t = {} s. Find the root-cause service.",
        world.alert_s()
    )
}

struct Settings {
    loop_: Loop,
    model_id: AgentId,
    out: PathBuf,
}

/// Run `arm` on a fresh world of `raw`; `agent` for the arms that have one.
fn run_arm(
    raw: Raw,
    arm: &str,
    agent: Option<&mut dyn Agent>,
    set: &Settings,
) -> (Score, World, Option<Run>) {
    let mut w = World::new(raw);
    let tag = format!("{}__{}", w.raw.system, w.raw.rel.replace('/', "_"));
    if arm == "c" {
        let (_, ranking) = oracle::baro(&w);
        let mut s = score(&w, arm, "baro/1", None, ranking.clone());
        s.baro = ranking;
        return (s, w, None);
    }
    let agent = agent.expect("an agent for arms a, b, d");
    if arm == "b" || arm == "d" {
        w.open_ledger(
            &set.out.join("stores").join(format!("{tag}__{arm}")),
            &set.model_id,
        );
    }
    let mut first = alert(&w);
    let mut baro = Vec::new();
    if arm == "d" {
        let (units, ranking) = oracle::baro(&w);
        w.commit_ranking(&units);
        first.push('\n');
        first.push_str(&oracle::ranking_text(&w, &units, &ranking));
        baro = ranking;
    }
    let r = agent::run(&mut w, agent, &set.loop_, first);
    let ranking = r
        .answer
        .as_ref()
        .map(|a| a.ranking.clone())
        .unwrap_or_default();
    let mut s = score(&w, arm, &set.loop_.model, Some(&r), ranking);
    s.baro = baro;
    (s, w, Some(r))
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn system_dir(data: &Path, case: &str) -> (PathBuf, String) {
    let (system, rel) = case
        .split_once('/')
        .expect("<system>/<service>_<fault>/<rep>");
    (data.join(system), rel.to_string())
}

fn mean(xs: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = xs.collect();
    if v.is_empty() {
        f64::NAN
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

fn table(scores: &[Score]) {
    let mut arms: Vec<&str> = scores.iter().map(|s| s.arm.as_str()).collect();
    arms.sort_unstable();
    arms.dedup();
    println!(
        "\narm  runs  AC@1   AC@3   AC@5   fab(tol)  fab(strict)  claims  TQ P    TQ R    unused  protocol  forced  steps  in-tok/run  s/run"
    );
    for arm in arms {
        let v: Vec<&Score> = scores.iter().filter(|s| s.arm == arm).collect();
        let ac = |k: usize| {
            mean(
                v.iter()
                    .map(|s| f64::from(u8::from(s.rank >= 1 && s.rank <= k))),
            )
        };
        let claims: usize = v.iter().map(|s| s.claims).sum();
        let rate = |n: usize| {
            if claims == 0 {
                f64::NAN
            } else {
                n as f64 / claims as f64
            }
        };
        let (tp, fp, fnn) = v.iter().fold((0, 0, 0), |a, s| {
            (a.0 + s.tq_tp, a.1 + s.tq_fp, a.2 + s.tq_fn)
        });
        let p = if tp + fp == 0 {
            f64::NAN
        } else {
            tp as f64 / (tp + fp) as f64
        };
        let r = if tp + fnn == 0 {
            f64::NAN
        } else {
            tp as f64 / (tp + fnn) as f64
        };
        let (cons, cited) = v
            .iter()
            .fold((0, 0), |a, s| (a.0 + s.consulted, a.1 + s.cited));
        let unused = if cons == 0 {
            f64::NAN
        } else {
            (cons - cited) as f64 / cons as f64
        };
        let proto = v.iter().filter(|s| s.protocol.is_some()).count();
        let forced = v.iter().filter(|s| s.forced).count();
        println!(
            "{arm:<4} {:>4}  {:.3}  {:.3}  {:.3}  {:>8.3}  {:>11.3}  {claims:>6}  {p:.3}  {r:.3}  {unused:>6.3}  {proto:>8}  {forced:>6}  {:>5.1}  {:>10.0}  {:>5.1}",
            v.len(),
            ac(1),
            ac(3),
            ac(5),
            rate(v.iter().map(|s| s.fabricated).sum()),
            rate(v.iter().map(|s| s.fabricated_strict).sum()),
            mean(v.iter().map(|s| s.steps as f64)),
            mean(v.iter().map(|s| s.input_tokens as f64)),
            mean(v.iter().map(|s| s.secs)),
        );
    }
}

fn cmd_run(args: &[String]) {
    let data = PathBuf::from(arg(args, "--data").expect("--data DIR"));
    let cases_file = arg(args, "--cases").expect("--cases FILE");
    let arms: Vec<String> = arg(args, "--arms")
        .unwrap_or_else(|| "a,b,c,d".into())
        .split(',')
        .map(str::to_string)
        .collect();
    // (e) has no run of its own: it is (d)'s, completed (A4-2).
    let with_e = arms.iter().any(|a| a == "e");
    assert!(
        !with_e || arms.iter().any(|a| a == "d"),
        "arm e is d's run completed: list d too"
    );
    let model = arg(args, "--model").unwrap_or_else(|| "qwen2.5:14b-instruct".into());
    let endpoint = arg(args, "--endpoint").unwrap_or_else(|| "http://localhost:11434".into());
    let steps: usize = arg(args, "--steps").map_or(25, |s| s.parse().expect("--steps N"));
    let ctx: usize = arg(args, "--ctx").map_or(32_768, |s| s.parse().expect("--ctx N"));
    let out = PathBuf::from(arg(args, "--out").unwrap_or_else(|| "target/ge-m7".into()));
    let digits = arg(args, "--digits").map(|s| s.parse::<usize>().expect("--digits N"));
    world::DIGITS.set(digits).expect("set once");
    std::fs::create_dir_all(out.join("transcripts")).expect("the output directory");
    let cases: Vec<String> = std::fs::read_to_string(&cases_file)
        .unwrap_or_else(|e| panic!("{cases_file}: {e}"))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    let cases = match arg(args, "--shuffle") {
        Some(seed) => shuffled(cases, &seed),
        None => cases,
    };
    let mut cfg = ProviderConfig::new(ProviderId::new("ollama").expect("an id"), "ollama");
    cfg.endpoint = endpoint;
    cfg.model = model.clone();
    cfg.context_window = ctx;
    let provider = smysl::build_provider(&cfg).expect("the Ollama provider");
    let mut agent = ModelAgent { provider };
    let model_id = AgentId::new(format!("model:ollama/{}", model.replace(':', "-")))
        .expect("a model agent id");
    let set = Settings {
        loop_: Loop {
            model: model.clone(),
            steps,
            context_window: ctx,
            max_output: 1024,
        },
        model_id,
        out: out.clone(),
    };
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("runs.jsonl"))
        .expect("runs.jsonl");
    let mut scores = Vec::new();
    for case in &cases {
        for arm in arms.iter().filter(|a| *a != "e") {
            let (dir, rel) = system_dir(&data, case);
            let raw = Raw::load(&dir, &rel).unwrap_or_else(|e| panic!("{e}"));
            let (s, _, r) = run_arm(
                raw,
                arm,
                if arm == "c" { None } else { Some(&mut agent) },
                &set,
            );
            eprintln!(
                "{case} ({arm}): rank {} of {:?}, {} claims, {} fabricated, {} steps{}{}, {:.0} s",
                s.rank,
                s.ranking,
                s.claims,
                s.fabricated,
                s.steps,
                if s.forced { ", forced" } else { "" },
                s.protocol
                    .as_deref()
                    .map(|p| format!(", protocol failure: {p}"))
                    .unwrap_or_default(),
                s.secs
            );
            if let Some(r) = r {
                let t: Vec<serde_json::Value> = r
                    .transcript
                    .iter()
                    .map(|m| serde_json::json!({"role": m.role.as_str(), "content": m.content}))
                    .collect();
                let name = format!("{}__{arm}.json", case.replace('/', "_"));
                std::fs::write(
                    out.join("transcripts").join(name),
                    serde_json::to_string_pretty(&t).unwrap(),
                )
                .expect("a transcript");
            }
            writeln!(log, "{}", s.json()).expect("runs.jsonl");
            if arm == "d" && with_e {
                let e = completed(&s);
                eprintln!("{case} (e): rank {} of {:?}", e.rank, e.ranking);
                writeln!(log, "{}", e.json()).expect("runs.jsonl");
                scores.push(e);
            }
            scores.push(s);
        }
    }
    table(&scores);
}

/// Every tool's result over a case, for the leakage check.
fn every_result(w: &mut World) -> Vec<String> {
    let mut out = vec![w.tool_services()];
    for svc in w.services() {
        out.extend(w.tool_series(&svc).ok());
        let names: Vec<String> = w
            .series
            .keys()
            .filter(|k| k.starts_with(&format!("{svc}/")))
            .cloned()
            .collect();
        for n in names.iter().take(3) {
            out.extend(w.tool_compare(n, w.alert_s()).ok());
        }
        out.extend(w.tool_logs(&svc, 360, 1080).ok());
        out.extend(w.tool_traces(&svc, 360, 1080).ok());
    }
    out
}

fn cmd_check_data(args: &[String]) {
    let data = PathBuf::from(arg(args, "--data").expect("--data DIR"));
    let mut leaks = 0usize;
    for system in ["RE2-OB", "RE2-SS", "RE2-TT"] {
        let dir = data.join(system);
        let cases = case::list(&dir);
        let mut files: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &cases {
            for f in [
                "inject_time.txt",
                "simple_metrics.csv",
                "logs.csv",
                "traces.csv",
            ] {
                if dir.join(c).join(f).exists() {
                    *files.entry(f).or_default() += 1;
                }
            }
        }
        println!("{system}: {} cases; files {files:?}", cases.len());
        // Tokens still holding a root's stem (`username` for `user`): not leaks by §5.2's rule,
        // listed so a missed spelling shows.
        let mut residue: BTreeMap<String, usize> = BTreeMap::new();
        for c in &cases {
            let raw = Raw::load(&dir, c).unwrap_or_else(|e| panic!("{e}"));
            let root = raw.root.clone();
            let mut w = World::new(raw);
            let spell = w.anon.spellings(&root);
            if spell.is_empty() {
                println!("  {c}: the root cause {root} has no spelling to check");
                leaks += 1;
                continue;
            }
            let st = case::stem(&root);
            for r in every_result(&mut w) {
                if let Some(v) = case::leak(&r, &spell) {
                    leaks += 1;
                    println!("  LEAK {c}: `{v}` in {}", r.lines().next().unwrap_or(""));
                    break;
                }
                for t in case::residue(&r, &st) {
                    *residue.entry(t.to_string()).or_default() += 1;
                }
            }
        }
        // The spellings each system's data uses, for the record (R1-7).
        if let Some(c) = cases.first() {
            let raw = Raw::load(&dir, c).unwrap_or_else(|e| panic!("{e}"));
            let traced: std::collections::BTreeSet<String> =
                raw.spans.iter().flatten().map(|s| s.1.clone()).collect();
            let w = World::new(raw);
            let unmatched: Vec<&String> =
                traced.iter().filter(|t| w.anon.of(t).is_none()).collect();
            println!("  trace services not matched to a metric or log service: {unmatched:?}");
        }
        let mut res: Vec<(String, usize)> = residue.into_iter().collect();
        res.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        println!(
            "  residue (token: results): {:?}",
            &res[..res.len().min(25)]
        );
    }
    println!("leaks: {leaks}");
    std::process::exit(i32::from(leaks > 0));
}

/// The scored sample (§5.1): per system, each fault on each of its five services, repetition 1.
/// The pilot: two cases per system from repetition 2, the first and last fault directory.
fn cmd_sample(args: &[String]) {
    let data = PathBuf::from(arg(args, "--data").expect("--data DIR"));
    // The held-out cases of A4-1: every case of one repetition, none of it run before.
    if let Some(rep) = arg(args, "--rep") {
        let suffix = format!("/{rep}");
        let mut held = Vec::new();
        for system in ["RE2-OB", "RE2-SS", "RE2-TT"] {
            held.extend(
                case::list(&data.join(system))
                    .iter()
                    .filter(|c| c.ends_with(&suffix))
                    .map(|c| format!("{system}/{c}")),
            );
        }
        println!("# held-out: repetition {rep}, {} cases", held.len());
        for h in &held {
            println!("{h}");
        }
        return;
    }
    let mut sample = Vec::new();
    let mut pilot = Vec::new();
    for system in ["RE2-OB", "RE2-SS", "RE2-TT"] {
        let cases = case::list(&data.join(system));
        sample.extend(
            cases
                .iter()
                .filter(|c| c.ends_with("/1"))
                .map(|c| format!("{system}/{c}")),
        );
        let reps2: Vec<&String> = cases.iter().filter(|c| c.ends_with("/2")).collect();
        if let (Some(a), Some(b)) = (reps2.first(), reps2.last()) {
            pilot.push(format!("{system}/{a}"));
            pilot.push(format!("{system}/{b}"));
        }
    }
    println!("# scored sample: {} cases", sample.len());
    for s in &sample {
        println!("{s}");
    }
    println!("# pilot");
    for p in &pilot {
        println!("# {p}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..]),
        Some("check-data") => cmd_check_data(&args[1..]),
        Some("sample") => cmd_sample(&args[1..]),
        _ => {
            eprintln!("usage: ge_m7 run|check-data|sample --data DIR …   (see the module docs)");
            std::process::exit(2);
        }
    }
    let _ = CASE_S;
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent::Scripted;
    use smysl::Message;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ge_m7")
    }

    /// The id and a field's value on the line of `kind` (`exact`, `sketch`) after `marker`.
    fn pick(text: &str, marker: &str, kind: &str, field: &str) -> (String, String) {
        let after = &text[text.find(marker).expect("the marker")..];
        let line = after
            .lines()
            .find(|l| l.trim_start().starts_with(kind))
            .expect("the line");
        let get = |k: &str| {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(&format!("{k}=")))
                .expect("the field")
                .to_string()
        };
        (get("id"), get(field))
    }

    /// The scripted agent: services, compare the root cause's cpu at the alert, one hypothesis
    /// with a true quote (the after-window's max) and a fabricated one (its mean, invented),
    /// then the answer with both.
    fn script(root: String) -> Scripted {
        let series = format!("{root}/cpu");
        let r1 = root.clone();
        let r2 = root.clone();
        let quotes = |m: &[Message]| -> serde_json::Value {
            let last = &m.last().expect("a tool result").content;
            let (id, max) = pick(last, "after:", "exact", "max");
            serde_json::json!([
                {"id": id, "field": "max", "value": max},
                {"id": id, "field": "mean", "value": "123.456"}
            ])
        };
        Scripted {
            steps: vec![
                Box::new(|_| r#"{"tool":"services"}"#.to_string()),
                Box::new(move |_| {
                    serde_json::json!({"tool": "compare", "series": series, "at": 720}).to_string()
                }),
                Box::new(move |m| {
                    serde_json::json!({"tool": "hypothesise", "service": r1,
                        "claim": "cpu rose after the alert", "quotes": quotes(m)})
                    .to_string()
                }),
                Box::new(move |m| {
                    // The quotes are the compare result's, two messages back.
                    let q = quotes(&m[..m.len() - 2]);
                    serde_json::json!({"tool": "answer", "ranking": [r2], "quotes": q}).to_string()
                }),
            ],
            next: 0,
            schemas: Vec::new(),
        }
    }

    fn settings(out: &Path) -> Settings {
        Settings {
            loop_: Loop {
                model: "scripted".into(),
                steps: 25,
                context_window: 32_768,
                max_output: 1024,
            },
            model_id: AgentId::new("model:ollama/scripted").unwrap(),
            out: out.to_path_buf(),
        }
    }

    #[test]
    fn the_scripted_agent_scores_as_expected_in_every_arm() {
        let out = std::env::temp_dir().join(format!("ge-m7-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let set = settings(&out);
        for case in ["RE2-FX/cartservice_cpu/1", "RE2-FY/orders_cpu/1"] {
            let (dir, rel) = system_dir(&fixtures(), case);
            let root = {
                let w = World::new(Raw::load(&dir, &rel).unwrap());
                w.anon.to[&w.raw.root].clone()
            };
            for arm in ["a", "b", "c", "d"] {
                let raw = Raw::load(&dir, &rel).unwrap();
                let mut sc = script(root.clone());
                let agent: Option<&mut dyn Agent> = if arm == "c" { None } else { Some(&mut sc) };
                let (s, _, r) = run_arm(raw, arm, agent, &set);
                assert_eq!(s.rank, 1, "{case} ({arm}): the root cause first: {s:?}");
                if arm == "c" {
                    assert_eq!(s.claims, 0);
                    continue;
                }
                let r = r.unwrap();
                assert_eq!(r.protocol, None, "{case} ({arm}): {:?}", r.transcript);
                assert_eq!(
                    (s.claims, s.fabricated, s.fabricated_strict),
                    (2, 1, 1),
                    "{case} ({arm})"
                );
                assert_eq!(s.hypotheses, 1);
                if arm == "a" {
                    assert_eq!(s.tq_quotes, 0, "(a) writes nothing");
                } else {
                    assert_eq!(
                        (s.tq_quotes, s.tq_tp, s.tq_fp, s.tq_fn),
                        (2, 1, 0, 0),
                        "{case} ({arm}): TQ fails the invented quote and only it"
                    );
                    let hyp = &r.transcript[6].content;
                    assert!(hyp.contains("SMY-W606"), "{hyp}");
                }
                assert!(s.consulted >= 2 && s.cited == 1, "{s:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_shuffle_is_a_seeded_permutation() {
        let cases: Vec<String> = (0..30).map(|i| format!("c{i}")).collect();
        let a = shuffled(cases.clone(), "1");
        assert_eq!(a, shuffled(cases.clone(), "1"));
        assert_ne!(a, shuffled(cases.clone(), "2"));
        assert_ne!(a, cases);
        let mut s = a.clone();
        s.sort();
        let mut c = cases.clone();
        c.sort();
        assert_eq!(s, c);
    }

    #[test]
    fn completion_keeps_the_answer_first_and_fills_from_baro() {
        let v = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let baro = v(&["s1", "s2", "s3", "s4", "s5"]);
        assert_eq!(
            complete(&v(&["s4", "s9"]), &baro),
            v(&["s4", "s9", "s1", "s2", "s3"])
        );
        assert_eq!(complete(&[], &baro), baro);
        assert_eq!(complete(&v(&["s1", "s1"]), &v(&["s2"])), v(&["s1", "s2"]));
        let d = Score {
            arm: "d".into(),
            root: "s3".into(),
            ranking: v(&["s4"]),
            baro: baro.clone(),
            rank: 0,
            claims: 2,
            fabricated: 1,
            ..Score::default()
        };
        let e = completed(&d);
        assert_eq!(
            (e.arm.as_str(), e.rank, e.claims, e.fabricated),
            ("e", 4, 2, 1)
        );
    }

    /// A script that never answers: the forced step's schema is `answer`'s alone, and its answer
    /// is scored; a forced reply that is not an answer is a budget failure.
    #[test]
    fn a_spent_budget_gets_one_forced_answer() {
        let (dir, rel) = system_dir(&fixtures(), "RE2-FX/cartservice_cpu/1");
        let root = {
            let w = World::new(Raw::load(&dir, &rel).unwrap());
            w.anon.to[&w.raw.root].clone()
        };
        let out = std::env::temp_dir().join(format!("ge-m7-forced-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let mut set = settings(&out);
        set.loop_.steps = 2;
        for good in [true, false] {
            let r2 = root.clone();
            let mut sc = Scripted {
                steps: vec![
                    Box::new(|_| r#"{"tool":"services"}"#.to_string()),
                    Box::new(|_| r#"{"tool":"metrics","service":"svc-01"}"#.to_string()),
                    Box::new(move |m: &[Message]| {
                        assert!(m.last().unwrap().content.ends_with(agent::FORCED));
                        if good {
                            serde_json::json!({"tool": "answer", "ranking": [r2], "quotes": []})
                                .to_string()
                        } else {
                            r#"{"tool":"services"}"#.to_string()
                        }
                    }),
                ],
                next: 0,
                schemas: Vec::new(),
            };
            let (s, _, r) = run_arm(Raw::load(&dir, &rel).unwrap(), "b", Some(&mut sc), &set);
            assert_eq!(sc.next, 3, "two tool calls and the forced step");
            assert_eq!(
                sc.schemas[2].as_deref(),
                Some(agent::answer_schema().as_str())
            );
            assert_eq!(sc.schemas[0].as_deref(), Some(agent::schema().as_str()));
            assert!(s.forced && r.unwrap().forced);
            if good {
                assert_eq!((s.rank, s.protocol.as_deref()), (1, None), "{s:?}");
            } else {
                assert_eq!(
                    (s.rank, s.protocol.as_deref()),
                    (0, Some("budget")),
                    "{s:?}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_rounded_quote_is_not_fabricated_and_is_not_strict() {
        let (dir, rel) = system_dir(&fixtures(), "RE2-FX/cartservice_cpu/1");
        let mut w = World::new(Raw::load(&dir, &rel).unwrap());
        let root = w.anon.to[&w.raw.root].clone();
        let text = w.tool_window(&format!("{root}/cpu"), 720, 1080).unwrap();
        let (id, mean) = pick(&text, "[720", "exact", "mean");
        let q = |v: &str| world::Quote {
            id: id.clone(),
            field: "mean".into(),
            value: v.into(),
        };
        let exact = oracle::judge(&w, &q(&mean));
        assert!(!exact.fabricated && exact.strict, "{mean}: {exact:?}");
        let x: f64 = mean.parse().unwrap();
        let rounded = format!("{x:.2}");
        if rounded != mean {
            let v = oracle::judge(&w, &q(&rounded));
            assert!(!v.fabricated && !v.strict, "{rounded}: {v:?}");
        }
        let v = oracle::judge(&w, &q(&format!("{:.2}", x + 1.0)));
        assert!(v.fabricated);
        let (sid, p99) = pick(&text, "[720", "sketch", "q[2]");
        let v = oracle::judge(
            &w,
            &world::Quote {
                id: sid,
                field: "q[2]".into(),
                value: p99,
            },
        );
        assert!(!v.fabricated && v.strict, "{v:?}");
    }

    #[test]
    fn no_tool_result_names_the_root_cause() {
        for case in ["RE2-FX/cartservice_cpu/1", "RE2-FY/orders_cpu/1"] {
            let (dir, rel) = system_dir(&fixtures(), case);
            let raw = Raw::load(&dir, &rel).unwrap();
            let root = raw.root.clone();
            let mut w = World::new(raw);
            let spell = w.anon.spellings(&root);
            assert!(!spell.is_empty());
            for r in every_result(&mut w) {
                assert_eq!(case::leak(&r, &spell), None, "{case}: {r}");
            }
            // And the raw data does name it, so the check can fail.
            assert!(w
                .raw
                .logs
                .iter()
                .any(|l| case::leak(&l.2, &spell).is_some()));
        }
    }
}
