//! The SM-P2 gate.
//!
//! Three properties, each of which the format leans on somewhere else:
//!
//! - `surface → CBOR → surface` is lossless modulo formatting, so `fmt` can canonicalise
//!   without touching identity;
//! - a file with one malformed record still yields every other record, plus one
//!   diagnostic with a correct span, because the ingest repair loop resends spans;
//! - the parser does not panic on anything, because its input comes from other agents.

use smysl_core::surface::{parse_surface, write_surface, WriteContext};
use smysl_core::{Label, Record, Status};

/// The worked example of §6, verbatim.
const RFC_EXAMPLE: &str = r#"@doc smysl/0.1 {
  intent: incident-brief, lang: en
  requires: ["smysl.kernel/0.1", "x.sre/1"]
  granularity: { profile: default }
  roots: [c/auth-p95-regression]
}

@evidence e/trace-jul {
  status: measured
  source: { kind: metric, ref: "grafana://board/12/panel/4", captured: 2026-07-09 }
}
~ Seven days of auth request traces, all shards, 2026-07-02..09.

@claim c/auth-p95-regression {
  status: measured, grounds: [e/trace-jul], deps: [d/p95]
}
~ p95 auth latency tripled after the 4.2 rollout.

Between 2026-07-02 and 2026-07-09, p95 on `POST /auth` rose from 180 ms to
540 ms, confined to the eu-west shard.

--
Per-shard breakdown at hourly resolution, the pool-saturation hypothesis, the
pool metrics supporting it, and the canary result that does not.

@definition d/p95 { status: derived, grounds: [e/trace-jul] }
~ The 95th percentile of request latency over a one-minute window.

@claim c/pool-saturation { status: inferred, grounds: [e/trace-jul] }
~ The eu-west connection pool is saturated.

@claim c/canary-clean { status: measured, source: { kind: metric, ref: "canary.p95" } }
~ The 4.2 canary ran the same pool configuration without the regression.

@rel c/pool-saturation --causes--> c/auth-p95-regression
@rel c/canary-clean    --rebuts--> c/pool-saturation { weight: 0.6 }

@thread t/brief { schema: brief, owner: "model:anthropic/claude-opus-5" }
~ Auth p95 tripled in eu-west; pool saturation is leading but contested.
  bottom-line → c/auth-p95-regression
  support     → c/pool-saturation
  risk        → c/canary-clean
"#;

fn label(s: &str) -> Label {
    Label::new(s).unwrap()
}

/// The §6 example is shape-invalid as printed: its central claim is `measured` with no
/// `source`, which §1.4 forbids. Everything except the test that documents that defect
/// runs against this corrected copy.
fn corpus() -> String {
    RFC_EXAMPLE.replace(
        "status: measured, grounds: [e/trace-jul], deps: [d/p95]",
        "status: measured, grounds: [e/trace-jul], deps: [d/p95]\n  source: { kind: metric, ref: \"grafana://board/12/panel/4\" }",
    )
}

/// The RFC's own worked example does not satisfy the RFC's own rule. `check` catching it
/// is the machinery working, not failing - `measured` without a source is exactly the
/// shape rule M and rule T exist to keep honest.
#[test]
fn the_rfc_section_6_example_is_itself_shape_invalid() {
    let out = parse_surface(RFC_EXAMPLE).unwrap();
    let codes: Vec<smysl_core::Code> = out.diagnostics.iter().map(|d| d.code).collect();
    assert!(
        codes.contains(&smysl_core::Code::E032),
        "expected `measured` without `source` to be caught, got {codes:?}"
    );
}

#[test]
fn the_rfc_example_parses_without_diagnostics() {
    let out = parse_surface(&corpus()).unwrap();
    assert!(
        out.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        out.diagnostics
    );
    assert_eq!(out.recovered, 0);
}

#[test]
fn the_rfc_example_yields_every_record() {
    let out = parse_surface(&corpus()).unwrap();
    let units = out.units().count();
    let rels = out
        .records
        .iter()
        .filter(|r| matches!(r, Record::Relation(_)))
        .count();
    let threads = out
        .records
        .iter()
        .filter(|r| matches!(r, Record::Thread(_)))
        .count();
    assert_eq!((units, rels, threads), (5, 2, 1));
}

#[test]
fn labels_resolve_to_uids() {
    let out = parse_surface(&corpus()).unwrap();
    for l in [
        "e/trace-jul",
        "c/auth-p95-regression",
        "d/p95",
        "c/pool-saturation",
        "c/canary-clean",
    ] {
        assert!(out.uid_of(&label(l)).is_some(), "`{l}` did not resolve");
    }
    assert_eq!(out.labels.len(), 5);
}

/// A unit's uid depends on the uids of everything it points at, so resolution has to run
/// in dependency order rather than document order. `c/auth-p95-regression` grounds on
/// `e/trace-jul` and depends on `d/p95`, which is declared *after* it.
#[test]
fn forward_references_resolve() {
    let out = parse_surface(&corpus()).unwrap();
    let claim = out.uid_of(&label("c/auth-p95-regression")).unwrap();
    let dep = out.uid_of(&label("d/p95")).unwrap();
    let core = out
        .units()
        .find(|u| smysl_core::canonical_uid(u) == claim)
        .unwrap();
    assert!(core.deps.contains(&dep), "the forward dep did not resolve");
}

#[test]
fn the_header_fields_land_where_they_belong() {
    let out = parse_surface(&corpus()).unwrap();
    let uid = out.uid_of(&label("e/trace-jul")).unwrap();
    let e = out
        .units()
        .find(|u| smysl_core::canonical_uid(u) == uid)
        .unwrap();
    assert_eq!(e.status, Status::Measured);
    let s = e.source.as_ref().unwrap();
    assert_eq!(s.reference, "grafana://board/12/panel/4");
    assert_eq!(s.captured.unwrap().to_string(), "2026-07-09");
}

#[test]
fn body_and_detail_are_split_at_the_separator() {
    let out = parse_surface(&corpus()).unwrap();
    let uid = out.uid_of(&label("c/auth-p95-regression")).unwrap();
    let c = out
        .units()
        .find(|u| smysl_core::canonical_uid(u) == uid)
        .unwrap();
    assert!(c.body.as_ref().unwrap().starts_with("Between 2026-07-02"));
    assert!(c.body.as_ref().unwrap().ends_with("eu-west shard."));
    assert!(c
        .detail
        .as_ref()
        .unwrap()
        .starts_with("Per-shard breakdown"));
}

#[test]
fn the_doc_header_becomes_a_view() {
    let out = parse_surface(&corpus()).unwrap();
    let v = out.view.as_ref().unwrap();
    assert_eq!(v.intent, "incident-brief");
    assert_eq!(v.lang.as_str(), "en");
    assert_eq!(v.requires.len(), 2);
    assert_eq!(v.roots.len(), 1);
    assert_eq!(v.granularity.profile, "default");
}

#[test]
fn relations_and_threads_resolve_their_references() {
    let out = parse_surface(&corpus()).unwrap();
    let canary = out.uid_of(&label("c/canary-clean")).unwrap();
    let pool = out.uid_of(&label("c/pool-saturation")).unwrap();

    let rebuts = out
        .records
        .iter()
        .find_map(|r| match r {
            Record::Relation(rel) if rel.kind == smysl_core::RelKind::Rebuts => Some(rel),
            _ => None,
        })
        .unwrap();
    assert_eq!(rebuts.from, canary);
    assert_eq!(rebuts.to, pool);
    assert_eq!(rebuts.weight, Some(614.0 / 1024.0));

    let thread = out
        .records
        .iter()
        .find_map(|r| match r {
            Record::Thread(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(thread.steps.len(), 3);
    assert_eq!(thread.owner.as_str(), "model:anthropic/claude-opus-5");
}

/// The SM-P2 gate: surface -> records -> surface -> records is a fixed point.
#[test]
fn the_round_trip_is_lossless_modulo_formatting() {
    let first = parse_surface(&corpus()).unwrap();
    let ctx = WriteContext::from_labels(&first.labels).with_salience(first.salience.clone());
    let text = write_surface(first.view.as_ref(), &first.records, &ctx);

    let second = parse_surface(&text).unwrap();
    assert!(
        second.diagnostics.is_empty(),
        "re-parsing canonical output produced diagnostics: {:?}",
        second.diagnostics
    );
    assert_eq!(second.records, first.records);
    assert_eq!(second.labels, first.labels);
    assert_eq!(
        second.view.as_ref().map(|v| &v.roots),
        first.view.as_ref().map(|v| &v.roots)
    );
}

/// `fmt` must be idempotent, or `fmt --check` would never settle.
#[test]
fn formatting_is_idempotent() {
    let a = parse_surface(&corpus()).unwrap();
    let once = write_surface(
        a.view.as_ref(),
        &a.records,
        &WriteContext::from_labels(&a.labels),
    );
    let b = parse_surface(&once).unwrap();
    let twice = write_surface(
        b.view.as_ref(),
        &b.records,
        &WriteContext::from_labels(&b.labels),
    );
    assert_eq!(once, twice);
}

/// Reformatting must never change a uid: hashes are computed over CBOR only (§6).
#[test]
fn reformatting_does_not_change_identity() {
    let a = parse_surface(&corpus()).unwrap();
    let text = write_surface(
        a.view.as_ref(),
        &a.records,
        &WriteContext::from_labels(&a.labels),
    );
    let b = parse_surface(&text).unwrap();
    assert_eq!(a.labels, b.labels, "a label bound to a different uid");
}

/// Records also survive the CBOR leg, which is what `surface → CBOR → surface` means.
#[test]
fn records_survive_the_cbor_leg() {
    let out = parse_surface(&corpus()).unwrap();
    let bytes = smysl_core::to_cbor_seq(&out.records);
    let (back, off) = smysl_core::from_cbor_seq(&bytes).unwrap();
    assert_eq!(off, bytes.len());
    assert_eq!(back, out.records);
}

/// The recovery property: one malformed record costs one record, not the file.
#[test]
fn a_malformed_record_does_not_cost_the_others() {
    let src = "@claim c/a { status: speculative }\n~ first\n\n\
               @claim c/b { status: nonsense }\n~ second\n\n\
               @claim c/c { status: speculative }\n~ third\n";
    let out = parse_surface(src).unwrap();
    assert_eq!(out.units().count(), 2, "the good records must survive");
    assert_eq!(out.recovered, 1);
    assert_eq!(out.diagnostics.len(), 1);
    assert!(out.uid_of(&label("c/a")).is_some());
    assert!(out.uid_of(&label("c/c")).is_some());
    assert!(out.uid_of(&label("c/b")).is_none());
}

#[test]
fn a_diagnostic_carries_a_span_that_points_at_the_defect() {
    let src = "@claim c/a { status: speculative }\n~ first\n\n@claim c/b { status: nonsense }\n~ second\n";
    let out = parse_surface(src).unwrap();
    let d = &out.diagnostics[0];
    let span = d.span().expect("a parse diagnostic must carry a span");
    let text = span.slice(src).unwrap();
    assert!(
        text.contains("nonsense"),
        "the span points at `{text}`, not at the defect"
    );
}

#[test]
fn a_missing_gist_is_reported_against_the_header() {
    let src = "@claim c/a { status: speculative }\n\nbody without a gist\n";
    let out = parse_surface(src).unwrap();
    assert_eq!(out.units().count(), 0);
    assert_eq!(out.diagnostics[0].code, smysl_core::Code::E021);
}

#[test]
fn an_unresolvable_reference_is_reported_and_dropped() {
    let src = "@claim c/a { status: inferred, grounds: [e/missing] }\n~ a claim\n";
    let out = parse_surface(src).unwrap();
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == smysl_core::Code::E060));
    // `inferred` with no surviving grounds is E031, so the unit is not produced.
    assert_eq!(out.units().count(), 0);
}

#[test]
fn a_reference_cycle_is_reported_rather_than_hanging() {
    let src = "@claim c/a { status: speculative, deps: [c/b] }\n~ a\n\n\
               @claim c/b { status: speculative, deps: [c/a] }\n~ b\n";
    let out = parse_surface(src).unwrap();
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == smysl_core::Code::E061));
}

#[test]
fn an_unsupported_format_version_is_the_one_hard_error() {
    assert!(parse_surface("@doc smysl/9.9 {}\n").is_err());
    assert!(parse_surface("@doc smysl/0.1 {}\n").is_ok());
}

#[test]
fn an_unsupported_kernel_major_is_refused_not_degraded() {
    let src = "@doc smysl/0.1 { requires: [\"smysl.kernel/9\"] }\n";
    assert!(
        parse_surface(src).is_err(),
        "a consumer MUST NOT silently degrade"
    );
}

#[test]
fn unknown_header_keys_are_preserved_into_payload() {
    let src =
        "@claim c/a { status: speculative, sre_severity: 2, owner_team: platform }\n~ a claim\n";
    let out = parse_surface(src).unwrap();
    let u = out.units().next().unwrap();
    let payload = u.payload.as_ref().expect("unknown keys must reach payload");
    let o = smysl_core::surface::payload_to_object(payload).unwrap();
    assert_eq!(o.get("sre_severity").unwrap().value.as_int(), Some(2));
    assert_eq!(
        o.get("owner_team").unwrap().value.as_str(),
        Some("platform")
    );
}

#[test]
fn unknown_header_keys_survive_a_full_round_trip() {
    let src = "@claim c/a { status: speculative, sre_severity: 2 }\n~ a claim\n";
    let a = parse_surface(src).unwrap();
    let text = write_surface(None, &a.records, &WriteContext::from_labels(&a.labels));
    let b = parse_surface(&text).unwrap();
    assert_eq!(a.records, b.records, "rule X violated on re-emission");
}

#[test]
fn both_arrow_spellings_are_accepted_and_one_is_emitted() {
    let src = "@claim c/a { status: speculative }\n~ a\n\n\
               @thread t/x { schema: brief, owner: \"human:v\" }\n~ g\n  bottom-line -> c/a\n";
    let out = parse_surface(src).unwrap();
    let text = write_surface(None, &out.records, &WriteContext::from_labels(&out.labels));
    assert!(text.contains('\u{2192}'));
    assert!(!text.contains("->"));
}

/// A1: no panics on untrusted input. Every prefix of a valid document, plus a set of
/// deliberately broken ones, must parse or diagnose - never panic.
#[test]
fn the_parser_does_not_panic_on_truncated_input() {
    let src = corpus();
    for i in 0..src.len() {
        if !src.is_char_boundary(i) {
            continue;
        }
        let _ = parse_surface(&src[..i]);
    }
}

#[test]
fn the_parser_does_not_panic_on_adversarial_input() {
    let cases = [
        "",
        "\n\n\n",
        "@",
        "@claim",
        "@claim {",
        "@claim c/a {",
        "@claim c/a { status:",
        "~",
        "~~~",
        "--",
        "@rel",
        "@rel a",
        "@rel a --",
        "@rel a --causes-->",
        "@thread",
        "@thread t/x",
        "@thread t/x {}",
        "@doc",
        "  -> ",
        "@claim c/a {}\n~ g\n  \u{2192}\n",
        "@claim c/a { deps: [] }\n~ g\n",
        "@claim c/a { deps: notanarray }\n~ g\n",
        "@claim \u{4f60}\u{597d} {}\n~ g\n",
        "@claim c/a {}\n~ \u{4f60}\u{597d}\n",
    ];
    for c in cases {
        let _ = parse_surface(c);
    }
}

#[test]
fn an_empty_document_is_an_empty_outcome() {
    let out = parse_surface("").unwrap();
    assert!(out.records.is_empty());
    assert!(out.view.is_none());
    assert!(out.diagnostics.is_empty());
}

#[test]
fn a_gist_only_unit_is_the_minimum_valid_record() {
    let out = parse_surface("@prose p/a {}\n~ just a gist\n").unwrap();
    assert_eq!(out.units().count(), 1);
    let u = out.units().next().unwrap();
    assert!(u.is_gist_only());
    assert_eq!(
        u.status,
        Status::Speculative,
        "status defaults to speculative"
    );
}

#[test]
fn an_at_sign_in_prose_does_not_truncate_a_record() {
    let src = "@claim c/a { status: speculative }\n~ a claim\n\nAsk @vladimir about this.\nAnd @claimant too.\n";
    let out = parse_surface(src).unwrap();
    assert_eq!(out.units().count(), 1);
    let body = out.units().next().unwrap().body.as_ref().unwrap();
    assert!(body.contains("@vladimir"));
    assert!(body.contains("@claimant"));
}

#[test]
fn salience_round_trips_outside_the_core() {
    let src = "@claim c/a { status: speculative, salience: 0.75 }\n~ a claim\n";
    let a = parse_surface(src).unwrap();
    let uid = a.uid_of(&label("c/a")).unwrap();
    assert_eq!(a.salience.get(&uid), Some(&0.75));
    assert!(
        a.units().next().unwrap().payload.is_none(),
        "salience is not identity, so it must not reach payload"
    );

    let ctx = WriteContext::from_labels(&a.labels).with_salience(a.salience.clone());
    let text = write_surface(None, &a.records, &ctx);
    let b = parse_surface(&text).unwrap();
    assert_eq!(b.salience.get(&uid), Some(&0.75));
}

// ---------------------------------------------------------------------------
// Deterministic mutation harness
// ---------------------------------------------------------------------------

/// A tiny xorshift, so the mutation sweep is reproducible. Rule D forbids RNG in library
/// code; a test may use one as long as it is seeded and therefore deterministic.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Guarantee A1, exercised without cargo-fuzz so it runs in ordinary CI.
///
/// The corpus is mutated a few thousand ways - bytes deleted, duplicated, replaced with
/// syntactically loaded characters - and every result must parse or diagnose. A panic
/// here is a crash in a component whose input comes from other agents.
#[test]
fn mutated_documents_never_panic() {
    let base = corpus();
    let interesting = [
        b'{', b'}', b'[', b']', b'"', b'~', b'@', b'-', b'>', b':', b',', b'\n', b'\\', 0, 0xFF,
    ];
    let mut rng = Rng(0x5D_EE_CE_66_D1_25_u64 | 1);

    for _ in 0..3_000 {
        let mut bytes = base.clone().into_bytes();
        let edits = 1 + rng.below(4);
        for _ in 0..edits {
            if bytes.is_empty() {
                break;
            }
            let at = rng.below(bytes.len());
            match rng.below(3) {
                0 => {
                    bytes[at] = interesting[rng.below(interesting.len())];
                }
                1 => {
                    bytes.remove(at);
                }
                _ => {
                    let b = bytes[at];
                    bytes.insert(at, b);
                }
            }
        }
        // Only well-formed UTF-8 reaches the parser; the byte layer is the codec's job.
        if let Ok(src) = String::from_utf8(bytes) {
            let _ = parse_surface(&src);
        }
    }
}

/// Anything the parser accepts must survive emission and re-parsing. This is the property
/// the `surface` fuzz target asserts, checked here over mutations that stay valid.
#[test]
fn every_accepted_mutation_round_trips() {
    let base = corpus();
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    let mut checked = 0usize;

    for _ in 0..2_000 {
        let mut bytes = base.clone().into_bytes();
        let at = rng.below(bytes.len());
        bytes.remove(at);
        let Ok(src) = String::from_utf8(bytes) else {
            continue;
        };
        let Ok(a) = parse_surface(&src) else { continue };
        if a.has_errors() {
            continue;
        }
        let ctx = WriteContext::from_labels(&a.labels).with_salience(a.salience.clone());
        let text = write_surface(a.view.as_ref(), &a.records, &ctx);
        let b = parse_surface(&text).expect("canonical output must re-parse");
        assert_eq!(b.records, a.records, "round trip changed the records");
        checked += 1;
    }
    assert!(
        checked > 0,
        "no mutation stayed valid, so nothing was checked"
    );
}

/// A thread step's target may be a canonical uid, not only a label.
///
/// The note separator in `role -> target: note` is a colon, and a canonical uid contains
/// one — so splitting on the first colon tore `b3:xxxx` into the reference `b3` plus a
/// note, and `SMY-E001: malformed reference `b3`` was the only way a step could ever name
/// a uid. That mattered beyond the syntax: `write_surface` falls back to the canonical uid
/// for any target with no label bound, which is exactly what `merge` produces for a unit
/// none of its inputs named — so the writer emitted documents its own parser rejected.
#[test]
fn a_thread_step_may_target_a_canonical_uid() {
    let uid = "b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let src = format!(
        "@claim c/a {{ status: speculative }}\n~ a\n\n\
         @thread t/x {{ schema: brief, owner: \"human:v\" }}\n~ g\n  bottom-line -> {uid}\n"
    );
    let out = parse_surface(&src).unwrap();
    assert!(
        !out.has_errors(),
        "canonical uid rejected in a thread step: {:?}",
        out.diagnostics
    );
}

/// The colon that *is* a note separator still separates, whichever form the target takes.
#[test]
fn a_step_note_survives_both_target_forms() {
    let uid = "b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    for target in [uid, "c/a"] {
        let src = format!(
            "@claim c/a {{ status: speculative }}\n~ a\n\n\
             @thread t/x {{ schema: brief, owner: \"human:v\" }}\n~ g\n  \
             bottom-line -> {target}: the headline\n"
        );
        let out = parse_surface(&src).unwrap();
        assert!(!out.has_errors(), "{target}: {:?}", out.diagnostics);
        let text = write_surface(None, &out.records, &WriteContext::from_labels(&out.labels));
        assert!(
            text.contains("the headline"),
            "note lost for {target}: {text}"
        );
    }
}

/// Whatever `write_surface` emits, `parse_surface` must accept — including for a store
/// whose steps point at units nobody labelled.
#[test]
fn an_unlabelled_step_target_round_trips_through_surface() {
    let uid = "b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let src = format!(
        "@claim c/a {{ status: speculative }}\n~ a\n\n\
         @thread t/x {{ schema: brief, owner: \"human:v\" }}\n~ g\n  bottom-line -> {uid}\n"
    );
    let a = parse_surface(&src).unwrap();
    // Without this the test passes vacuously: a setup that failed to parse leaves no
    // thread to emit, so the round-trip below has nothing to get wrong.
    assert!(!a.has_errors(), "setup did not parse: {:?}", a.diagnostics);
    assert!(
        a.records.iter().any(|r| matches!(r, Record::Thread(_))),
        "setup produced no thread record"
    );
    // Emit with an *empty* label map, forcing the canonical-uid fallback on every target.
    let text = write_surface(
        None,
        &a.records,
        &WriteContext::from_labels(&Default::default()),
    );
    let b = parse_surface(&text).unwrap();
    assert!(
        !b.has_errors(),
        "writer emitted what the parser rejects: {text}\n{:?}",
        b.diagnostics
    );
    // Label bindings are excluded deliberately. This emission uses an *empty* label map to
    // force the canonical-uid fallback on every step target, which also strips the unit's
    // own label - so the re-parse has no binding to produce. What is under test is whether
    // a uid-shaped step target survives, not whether labels do; `labels_survive_a_store_
    // round_trip` covers that.
    let semantic = |o: &smysl_core::surface::ParseOutcome| -> Vec<Record> {
        o.records
            .iter()
            .filter(|r| !matches!(r, Record::LabelBinding(_)))
            .cloned()
            .collect()
    };
    assert_eq!(semantic(&a), semantic(&b));
}

// ── Comments (0.2.0) ────────────────────────────────────────────────────────
//
// HJSON headers already accepted `#` and `//` *inside* a record, so rejecting them
// between records made the surface contradict itself. A format whose selling point is
// human review has to let a reviewer annotate what they are reviewing.

/// The case the feature exists for: a note above the record it is about.
#[test]
fn a_comment_between_records_is_not_stray_text() {
    for marker in ["#", "//"] {
        let src = format!(
            "{marker} a note for the reviewer\n\
             @claim c/a {{ status: speculative }}\n~ A claim.\n"
        );
        let out = parse_surface(&src).unwrap();
        assert!(!out.has_errors(), "{marker}: {:?}", out.diagnostics);
        // One unit plus its label binding: a labelled unit now yields two records, because
        // a label has to reach the wire as its own record to stay outside identity.
        assert_eq!(out.units().count(), 1, "{marker}");
        assert_eq!(out.comments, 1, "{marker}: not counted");
    }
}

/// **The bug the first attempt at this shipped.** A body runs from the gist to the next
/// record, so a comment between two records falls inside that range. Keeping it made the
/// comment become the previous unit's body - content invented out of a note, with a
/// granularity warning fired about it.
#[test]
fn a_comment_after_a_gist_does_not_become_the_body() {
    let src = "@claim c/a { status: speculative }\n~ A claim.\n\n\
               // TODO: get the dashboard link\n\
               @claim c/b { status: speculative }\n~ Another claim.\n";
    let out = parse_surface(src).unwrap();
    assert!(!out.has_errors(), "{:?}", out.diagnostics);
    let bodies: Vec<_> = out.units().filter_map(|u| u.body.as_deref()).collect();
    assert!(
        bodies.is_empty(),
        "a comment was absorbed as a body: {bodies:?}"
    );
    assert_eq!(out.comments, 1);
}

/// A comment is a comment wherever it sits, including inside a body. That costs a body
/// the ability to open a line with `#`, and the alternative was worse: a line whose
/// meaning depended on how far it happened to be from the next record.
#[test]
fn a_comment_inside_a_body_is_still_a_comment() {
    let src = "@claim c/a { status: speculative }\n~ A claim.\n\n\
               First paragraph.\n# not a heading, a comment\nSecond paragraph.\n";
    let out = parse_surface(src).unwrap();
    assert!(!out.has_errors(), "{:?}", out.diagnostics);
    let body = out.units().next().unwrap().body.as_deref().unwrap();
    assert!(!body.contains("comment"), "comment kept in body: {body:?}");
    assert!(body.contains("First paragraph."));
    assert!(body.contains("Second paragraph."));
    assert_eq!(out.comments, 1);
}

/// Only at column 0. An indented `#` is inside prose a reader wrote on purpose.
#[test]
fn an_indented_hash_is_not_a_comment() {
    let src = "@claim c/a { status: speculative }\n~ A claim.\n\n\
               A paragraph.\n  # indented, so prose\n";
    let out = parse_surface(src).unwrap();
    let body = out.units().next().unwrap().body.as_deref().unwrap();
    assert!(body.contains("# indented"), "body was {body:?}");
    assert_eq!(out.comments, 0);
}

/// A document of nothing but comments is empty, not malformed.
#[test]
fn a_file_of_only_comments_parses_to_nothing() {
    let out = parse_surface("# just a note\n// and another\n").unwrap();
    assert!(!out.has_errors(), "{:?}", out.diagnostics);
    assert!(out.records.is_empty());
    assert_eq!(out.comments, 2);
}

/// Canonical form cannot carry a comment, so a re-emission must still round trip - the
/// property `fmt` asserts before it writes anything.
#[test]
fn a_commented_document_still_round_trips() {
    let src = "# a note\n@claim c/a { status: speculative }\n~ A claim.\n";
    let a = parse_surface(src).unwrap();
    let text = write_surface(
        a.view.as_ref(),
        &a.records,
        &WriteContext::from_labels(&a.labels),
    );
    let b = parse_surface(&text).unwrap();
    assert_eq!(a.records, b.records);
    assert_eq!(b.comments, 0, "canonical form should carry no comments");
}

// ── Label bindings (0.2.0) ──────────────────────────────────────────────────

/// **The gap this closes.** Before `Record::LabelBinding`, labels survived a parse and not
/// a store round trip: a document that had been through `merge` came back with every
/// reference spelled as a canonical uid. It re-checked clean and no reader could follow it.
#[test]
fn labels_survive_a_cbor_round_trip() {
    use smysl_core::{from_cbor_seq, to_cbor_seq};

    let out = parse_surface(&corpus()).unwrap();
    let bytes = to_cbor_seq(&out.records);
    let (back, _) = from_cbor_seq(&bytes).unwrap();

    let bound: std::collections::BTreeMap<_, _> = back
        .iter()
        .filter_map(|r| match r {
            Record::LabelBinding(b) => Some((b.label.clone(), b.uid)),
            _ => None,
        })
        .collect();

    assert_eq!(
        bound, out.labels,
        "labels did not survive the wire; every reference would come back as a bare uid"
    );
    assert!(!bound.is_empty(), "the corpus binds labels");
}

/// A label is not identity: binding one cannot move the unit it names.
#[test]
fn a_binding_does_not_change_the_uid_it_names() {
    let out = parse_surface(&corpus()).unwrap();
    let uid = out.uid_of(&label("e/trace-jul")).unwrap();
    let core = out
        .units()
        .find(|u| smysl_core::canonical_uid(u) == uid)
        .unwrap();
    // Recomputing from the core alone must agree: nothing about the label is hashed.
    assert_eq!(smysl_core::canonical_uid(core), uid);
}

/// Re-emitting a document must not multiply its bindings.
#[test]
fn bindings_are_not_duplicated_by_a_round_trip() {
    let a = parse_surface(&corpus()).unwrap();
    let text = write_surface(
        a.view.as_ref(),
        &a.records,
        &WriteContext::from_labels(&a.labels),
    );
    let b = parse_surface(&text).unwrap();
    let count = |o: &smysl_core::surface::ParseOutcome| {
        o.records
            .iter()
            .filter(|r| matches!(r, Record::LabelBinding(_)))
            .count()
    };
    assert_eq!(count(&a), count(&b));
    assert_eq!(a.labels, b.labels);
}

// ── Forward compatibility (0.2.0) ───────────────────────────────────────────

/// A unit whose type a later version added must decode, not fail the record.
///
/// Before this, an unrecognised type produced `SMY-E004: malformed envelope` — corruption,
/// not degradation — while an unknown *record* type and an unknown *extension* type both
/// degraded correctly. One kernel type added in a later 0.x made every store carrying it
/// unreadable to an earlier build.
#[test]
fn an_unknown_kernel_type_decodes_and_re_encodes_unchanged() {
    use smysl_core::{from_cbor, to_cbor, SchemaId};

    let core = smysl_core::UnitCoreBuilder::new(
        SchemaId::parse_forward("postmortem").expect("a bare identifier must degrade"),
        "a gist",
        Status::Speculative,
    )
    .build()
    .expect("an unknown type is still a valid unit");

    let bytes = to_cbor(&Record::Unit(core.clone()));
    let (back, n) = from_cbor(&bytes).expect("must decode, not fail the record");
    assert_eq!(n, bytes.len());
    assert_eq!(back.as_unit(), Some(&core), "the type did not survive");
    // Re-encoding must be byte-identical, or identity would move under a reader that
    // happens not to know the type.
    assert_eq!(to_cbor(&back), bytes);
}

/// Surface parsing and decoding need *opposite* behaviour here, and this pins the split.
/// On the wire an unrecognised type is forward compatibility; `parse` still refuses the
/// shapes that are malformed rather than merely unfamiliar.
#[test]
fn parse_forward_admits_bare_identifiers_and_nothing_else() {
    use smysl_core::SchemaId;

    // Degrades: a well-formed bare identifier.
    for s in ["postmortem", "post-mortem", "sev2"] {
        assert!(
            matches!(SchemaId::parse_forward(s), Ok(SchemaId::UnknownKernel(_))),
            "`{s}` should degrade"
        );
        assert!(SchemaId::parse(s).is_err(), "`{s}` must still fail `parse`");
    }
    // Known types are unaffected by either.
    assert!(matches!(
        SchemaId::parse_forward("claim"),
        Ok(SchemaId::Kernel(_))
    ));
    // Malformed, not unfamiliar: still refused by both.
    for s in ["", "Postmortem", "post!mortem", "x.sre", "a/b/c"] {
        assert!(
            SchemaId::parse_forward(s).is_err(),
            "`{s}` is malformed and must still fail"
        );
    }
}

/// The writer must not emit what the parser rejects. `write_surface` emits the exact type
/// string it decoded — it has to, since the type is hashed — so the lexer has to accept it.
#[test]
fn an_unknown_type_survives_a_surface_round_trip() {
    let src = "@postmortem p/a { status: speculative }\n~ a gist\n";
    let a = parse_surface(src).unwrap();
    assert!(!a.has_errors(), "{:?}", a.diagnostics);
    assert_eq!(a.units().count(), 1);

    let text = write_surface(
        a.view.as_ref(),
        &a.records,
        &WriteContext::from_labels(&a.labels),
    );
    let b = parse_surface(&text).unwrap();
    assert!(
        !b.has_errors(),
        "writer emitted what the parser rejects: {text}\n{:?}",
        b.diagnostics
    );
    assert_eq!(a.records, b.records);
}
