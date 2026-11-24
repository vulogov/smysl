//! RFC SMYSL-8 TR-P3: `KeyFile::render` and `KeyFile::parse` are inverses, and a rewrite
//! refuses to lose a comment.

use smysl_core::{AgentId, Status};
use smysl_sign::{KeyFile, KeyStatus, MemoryKeyResolver, ResolvedKey, SigningKey};

/// SplitMix64: a deterministic stream, so a failure reproduces from its case number.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const AGENTS: [&str; 5] = [
    "human:alice",
    "human:bob",
    "tool:smysl-ingest",
    "tool:smysl-tel",
    "model:ollama/llama3.1:8b",
];

const REASONS: [&str; 4] = [
    "rotated in Q4",
    "said \"compromised\" # not a comment",
    "back\\slash, and a // too",
    "tab\there",
];

fn key(rng: &mut Rng) -> ResolvedKey {
    let mut seed = [0u8; 32];
    for b in &mut seed {
        *b = rng.next() as u8;
    }
    let agent: AgentId = AGENTS[rng.below(AGENTS.len() as u64) as usize]
        .parse()
        .unwrap();
    let pk = SigningKey::from_seed(agent.clone(), seed).public_key();
    let mut k = ResolvedKey::active(agent, pk);
    k = match rng.below(4) {
        0 => k,
        1 => k.with_status(KeyStatus::Revoked {
            after_ms: (rng.below(2) == 0).then(|| rng.next() >> 20),
            reason: (rng.below(2) == 0)
                .then(|| REASONS[rng.below(REASONS.len() as u64) as usize].to_string()),
        }),
        2 => {
            let a = rng.next() >> 22;
            k.with_status(KeyStatus::Bounded {
                not_before_ms: a,
                not_after_ms: a + (rng.next() >> 30),
            })
        }
        _ => k.with_status(KeyStatus::Revoked {
            after_ms: None,
            reason: None,
        }),
    };
    if rng.below(3) == 0 {
        k = k.with_max_status(Status::ALL[rng.below(Status::ALL.len() as u64) as usize]);
    }
    k
}

#[test]
fn render_then_parse_gives_an_equal_resolver() {
    for case in 0..300u64 {
        let mut rng = Rng(case);
        let mut keys = MemoryKeyResolver::new();
        for _ in 0..rng.below(8) {
            keys.insert(key(&mut rng));
        }
        let text = KeyFile::render(&keys);
        let back = KeyFile::parse(&text).unwrap_or_else(|e| panic!("case {case}: {e}\n{text}"));
        assert_eq!(back, keys, "case {case}:\n{text}");
        // The writer's own output holds no comment past its header.
        assert_eq!(KeyFile::comment_line(&text), None, "case {case}:\n{text}");
        // And rendering is a function of the keys alone.
        assert_eq!(KeyFile::render(&back), text);
    }
}

#[test]
fn set_status_finds_a_key_by_id() {
    let mut rng = Rng(7);
    let k = key(&mut rng);
    let mut keys = MemoryKeyResolver::new().with(k.clone());
    let revoked = KeyStatus::Revoked {
        after_ms: Some(5),
        reason: None,
    };
    assert!(keys.set_status(&k.kid, revoked.clone()));
    assert_eq!(keys.keys().next().unwrap().status, revoked);
    let other = key(&mut Rng(8));
    assert!(!keys.set_status(&other.kid, KeyStatus::Active));
}

#[test]
fn a_comment_a_rewrite_would_lose_is_found() {
    let header = "# my keys\n# kept by hand\n{ keys: [] }\n";
    assert_eq!(KeyFile::comment_line(header), None);
    for (text, line) in [
        ("{\n  keys: [] # the team\n}\n", 2),
        ("# top\n{\n  // who\n  keys: []\n}\n", 3),
        ("{\n  keys: [\n  /* old */ ]\n}\n", 3),
    ] {
        assert_eq!(KeyFile::comment_line(text), Some(line), "{text}");
    }
    // Inside a string, `#` and `//` are text.
    let quoted =
        "{ keys: [ { agent: \"human:a\", ed25519: \"hex:00\", reason: \"a # b // c\" } ] }";
    assert_eq!(KeyFile::comment_line(quoted), None);
}
