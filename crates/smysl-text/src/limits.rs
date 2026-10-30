//! Resource caps, and the fuel that makes a refusal reproducible (SMYSL-2.4 §3.9.1).
//!
//! Every reader runs under a [`Budget`]. Exceeding any cap is `SMY-E440` naming the cap, the
//! limit, what the input asked for and the flag that raises it; nothing is written, because
//! objects are staged and moved into place only after a manifest is complete.
//!
//! # Fuel rather than time
//!
//! The one cap a reader must not have is a clock. A reader that measured wall time would
//! refuse a file on a slow machine and accept it on a fast one, so the same input would be
//! both valid and invalid depending on who read it — and a format whose acceptance depends on
//! the reader's hardware has no conformance to speak of. Fuel is charged per byte scanned,
//! per node created and per archive entry opened, so it runs out at the same point on every
//! machine. The wall-clock backstop exists for a bug that burns time without charging fuel,
//! and it lives in the CLI (`src/main.rs`), outside this crate, which is why `--timeout` is
//! not in [`Caps`].
//!
//! # What is charged, and by whom
//!
//! A `Budget` is a mutable counter passed to a reader; the reader charges it. Nothing here
//! can enforce that a reader charges honestly — what it can do is make the honest path the
//! easy one, so [`Budget::scan`], [`Budget::node`] and [`Budget::entry`] charge fuel *and*
//! the matching counter in one call. The limits suite (`tests/limits.rs`) is where a cap's
//! arithmetic is pinned; a crafted over-limit input per reader arrives with the readers.

use smysl_core::error::LibError;

/// The caps, with the defaults from SMYSL-2.4 §3.9.1.
///
/// Generous on purpose: the five Bibles and a multi-year chat export fit comfortably, so a
/// refusal means something unusual rather than something large.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Any one input file.
    pub input_bytes: u64,
    /// Total decompressed bytes over an archive.
    pub decompressed_bytes: u64,
    /// One archive entry, decompressed.
    pub entry_bytes: u64,
    /// Archive entry count.
    pub entries: u32,
    /// Decompressed ÷ compressed, per entry over 1 MiB.
    pub ratio: u32,
    /// JSON, XML or HTML nesting depth.
    pub nesting: u32,
    /// Structure nodes per manifest.
    pub nodes: u64,
    /// One part, as a hard ceiling: a single node larger than the part policy's maximum
    /// becomes one part, up to this.
    pub part_bytes: u64,
    /// Raw metadata kept, per segment and per manifest.
    pub raw_bytes: (u64, u64),
    /// Fuel, as a constant plus a multiple of the input size. Not a flat number: a fixed
    /// allowance is either too small for a large corpus or no constraint at all on a small
    /// file, and what the cap is really bounding is work *per byte*.
    pub fuel: Fuel,
}

/// Fuel as `per_byte × input + base` (§3.9.1: 64 × input bytes + 10⁸).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fuel {
    pub per_byte: u64,
    pub base: u64,
}

impl Fuel {
    /// The allowance for an input of this size, saturating rather than wrapping: a caller who
    /// sets `per_byte` absurdly high gets an enormous allowance, not a tiny one.
    pub const fn for_input(&self, input_bytes: u64) -> u64 {
        self.per_byte
            .saturating_mul(input_bytes)
            .saturating_add(self.base)
    }
}

impl Caps {
    /// The defaults. `const` so a caller can build on them in a `const` context.
    pub const DEFAULT: Caps = Caps {
        input_bytes: 1 << 30,
        decompressed_bytes: 2 << 30,
        entry_bytes: 256 << 20,
        entries: 100_000,
        ratio: 200,
        // The format's own encoder refuses deeper than this, so a reader that accepted more
        // could read a document it cannot represent. The one cap with no flag, for that
        // reason rather than by omission.
        nesting: smysl_core::cbor::MAX_NESTING as u32,
        nodes: 50_000_000,
        part_bytes: 64 << 20,
        raw_bytes: (1 << 20, 16 << 20),
        fuel: Fuel {
            per_byte: 64,
            base: 100_000_000,
        },
    };
}

impl Default for Caps {
    fn default() -> Caps {
        Caps::DEFAULT
    }
}

/// The flag that raises each cap, for the `SMY-E440` message.
///
/// A refusal an operator cannot act on is a refusal they will work around, and the way they
/// work around this one is by not using the library. `nesting` is the exception and has
/// `None`: see [`Caps::DEFAULT`].
const FLAGS: &[(&str, Option<&str>)] = &[
    ("input_bytes", Some("--max-input")),
    ("decompressed_bytes", Some("--max-decompressed")),
    ("entry_bytes", Some("--max-entry")),
    ("entries", Some("--max-entries")),
    ("ratio", Some("--max-ratio")),
    ("nesting", None),
    ("nodes", Some("--max-nodes")),
    ("part_bytes", Some("--max-part")),
    ("raw_bytes", Some("--max-raw")),
    ("fuel", Some("--max-fuel")),
];

/// The cap names, in the order §3.9.1's table lists them.
pub const CAPS: &[&str] = &[
    "input_bytes",
    "decompressed_bytes",
    "entry_bytes",
    "entries",
    "ratio",
    "nesting",
    "nodes",
    "part_bytes",
    "raw_bytes",
    "fuel",
];

fn exceeded(cap: &'static str, limit: u64, saw: u64) -> LibError {
    let flag = FLAGS
        .iter()
        .find(|(n, _)| *n == cap)
        .and_then(|(_, f)| *f)
        .map(|f| -> &'static str { f });
    LibError::Limit {
        cap,
        limit,
        saw,
        flag,
    }
}

/// What a reader spends, and what it has spent.
///
/// Created for one input, because two of the caps are about that input: `input_bytes` is
/// checked at construction, so a reader cannot be handed a budget for a file that was already
/// too large, and the fuel allowance is derived from the same number.
#[derive(Debug, Clone)]
pub struct Budget {
    caps: Caps,
    fuel_left: u64,
    fuel_spent: u64,
    nodes: u64,
    entries: u32,
    decompressed: u64,
    depth: u32,
    raw_total: u64,
}

impl Budget {
    /// A budget for an input of this size, or `SMY-E440` if the input itself is over the cap.
    pub fn new(caps: Caps, input_bytes: u64) -> Result<Budget, LibError> {
        if input_bytes > caps.input_bytes {
            return Err(exceeded("input_bytes", caps.input_bytes, input_bytes));
        }
        Ok(Budget {
            fuel_left: caps.fuel.for_input(input_bytes),
            caps,
            fuel_spent: 0,
            nodes: 0,
            entries: 0,
            decompressed: 0,
            depth: 0,
            raw_total: 0,
        })
    }

    pub fn caps(&self) -> &Caps {
        &self.caps
    }

    pub fn fuel_remaining(&self) -> u64 {
        self.fuel_left
    }

    pub fn fuel_spent(&self) -> u64 {
        self.fuel_spent
    }

    pub fn nodes(&self) -> u64 {
        self.nodes
    }

    pub fn entries(&self) -> u32 {
        self.entries
    }

    pub fn decompressed(&self) -> u64 {
        self.decompressed
    }

    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Charge fuel alone.
    ///
    /// A charge that does not fit **exhausts** the budget rather than leaving behind the
    /// units that were not enough. Two reasons, and the second is the one that matters: a
    /// refusal ends the operation, so there is nothing left to pay for; and a budget that
    /// kept a remainder would let a reader that ignored the error retry in smaller pieces and
    /// get further than the cap allows. `fuel_spent` therefore reports the whole allowance
    /// after a refusal, which is also what the `SMY-E440` message says the limit was.
    pub fn fuel(&mut self, units: u64) -> Result<(), LibError> {
        match self.fuel_left.checked_sub(units) {
            Some(left) => {
                self.fuel_left = left;
                self.fuel_spent = self.fuel_spent.saturating_add(units);
                Ok(())
            }
            None => {
                let limit = self.fuel_spent.saturating_add(self.fuel_left);
                let saw = self.fuel_spent.saturating_add(units);
                self.fuel_left = 0;
                self.fuel_spent = limit;
                Err(exceeded("fuel", limit, saw))
            }
        }
    }

    /// Scanning `n` bytes: one fuel per byte.
    pub fn scan(&mut self, n: u64) -> Result<(), LibError> {
        self.fuel(n)
    }

    /// Creating a structure node: one fuel, and one against the node cap.
    pub fn node(&mut self) -> Result<(), LibError> {
        if self.nodes >= self.caps.nodes {
            return Err(exceeded(
                "nodes",
                self.caps.nodes,
                self.nodes.saturating_add(1),
            ));
        }
        self.fuel(1)?;
        self.nodes += 1;
        Ok(())
    }

    /// Opening an archive entry of `decompressed` bytes from `compressed` on disk.
    ///
    /// Four caps at once, because they are four questions about the same event, and a reader
    /// that asked them separately would be a reader that could forget one. The ratio is only
    /// checked over 1 MiB: a 400-byte entry that decompresses from four bytes is a run of
    /// zeroes in a header, not an attack.
    pub fn entry(&mut self, decompressed: u64, compressed: u64) -> Result<(), LibError> {
        if self.entries >= self.caps.entries {
            return Err(exceeded(
                "entries",
                u64::from(self.caps.entries),
                u64::from(self.entries) + 1,
            ));
        }
        if decompressed > self.caps.entry_bytes {
            return Err(exceeded("entry_bytes", self.caps.entry_bytes, decompressed));
        }
        let total = self.decompressed.saturating_add(decompressed);
        if total > self.caps.decompressed_bytes {
            return Err(exceeded(
                "decompressed_bytes",
                self.caps.decompressed_bytes,
                total,
            ));
        }
        if decompressed > (1 << 20) {
            // A zero compressed size is not a reason to skip the check — it is the most
            // extreme case of the thing being checked, and an entry that claims to expand
            // from nothing is refused rather than divided by. Guarding the division by
            // skipping the check would have left exactly one hole in this cap, in the shape
            // of the input most likely to be deliberate.
            let ratio = match compressed {
                0 => u64::MAX,
                c => decompressed / c,
            };
            if ratio > u64::from(self.caps.ratio) {
                return Err(exceeded("ratio", u64::from(self.caps.ratio), ratio));
            }
        }
        self.fuel(1)?;
        self.entries += 1;
        self.decompressed = total;
        Ok(())
    }

    /// Entering a nested structure: `SMY-E440` on the level that would exceed the cap.
    ///
    /// Paired with [`Budget::leave`]. A reader that forgets to leave reports a depth it is not
    /// at, which fails closed: it refuses a document it could have read, rather than reading
    /// one it could not represent.
    pub fn enter(&mut self) -> Result<(), LibError> {
        if self.depth >= self.caps.nesting {
            return Err(exceeded(
                "nesting",
                u64::from(self.caps.nesting),
                u64::from(self.depth) + 1,
            ));
        }
        self.depth += 1;
        Ok(())
    }

    pub fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Keeping `n` bytes of raw metadata for one segment or manifest.
    ///
    /// Two limits from one call: the per-item cap, and the running total for the manifest.
    pub fn raw(&mut self, n: u64) -> Result<(), LibError> {
        let (per_item, total_cap) = self.caps.raw_bytes;
        if n > per_item {
            return Err(exceeded("raw_bytes", per_item, n));
        }
        let total = self.raw_total.saturating_add(n);
        if total > total_cap {
            return Err(exceeded("raw_bytes", total_cap, total));
        }
        self.raw_total = total;
        Ok(())
    }

    /// Admitting a part of `n` bytes against the hard ceiling.
    ///
    /// The part *policy*'s maximum is a target that a single oversized node is allowed to
    /// exceed (§3.1); this is the ceiling that node cannot exceed.
    pub fn part(&mut self, n: u64) -> Result<(), LibError> {
        if n > self.caps.part_bytes {
            return Err(exceeded("part_bytes", self.caps.part_bytes, n));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::Code;

    fn cap_of(e: &LibError) -> &'static str {
        match e {
            LibError::Limit { cap, .. } => cap,
            other => panic!("not a limit: {other}"),
        }
    }

    #[test]
    fn the_defaults_are_the_table_in_the_rfc() {
        let c = Caps::DEFAULT;
        assert_eq!(c.input_bytes, 1_073_741_824);
        assert_eq!(c.decompressed_bytes, 2_147_483_648);
        assert_eq!(c.entry_bytes, 268_435_456);
        assert_eq!(c.entries, 100_000);
        assert_eq!(c.ratio, 200);
        assert_eq!(c.nesting, 128);
        assert_eq!(c.nodes, 50_000_000);
        assert_eq!(c.part_bytes, 67_108_864);
        assert_eq!(c.raw_bytes, (1_048_576, 16_777_216));
        assert_eq!(c.fuel.for_input(1_000), 64_000 + 100_000_000);
    }

    /// The nesting cap is the format's own, not a number of its own. If `MAX_NESTING` moves,
    /// this moves with it — which is the point: a reader that accepted a document the encoder
    /// cannot write would read something it could not store.
    #[test]
    fn the_nesting_cap_is_the_formats_own() {
        assert_eq!(
            u64::from(Caps::DEFAULT.nesting),
            smysl_core::cbor::MAX_NESTING as u64
        );
    }

    #[test]
    fn every_cap_has_a_flag_except_nesting() {
        assert_eq!(CAPS.len(), FLAGS.len());
        for cap in CAPS {
            let (_, flag) = FLAGS.iter().find(|(n, _)| n == cap).expect("named");
            if *cap == "nesting" {
                assert!(flag.is_none());
            } else {
                assert!(flag.is_some(), "{cap} has no flag");
            }
        }
    }

    #[test]
    fn an_input_over_the_cap_is_refused_before_a_budget_exists() {
        let caps = Caps {
            input_bytes: 10,
            ..Caps::DEFAULT
        };
        let e = Budget::new(caps, 11).unwrap_err();
        assert_eq!(e.code(), Some(Code::E440));
        assert_eq!(cap_of(&e), "input_bytes");
        assert!(e.to_string().contains("--max-input"), "{e}");
        assert!(Budget::new(caps, 10).is_ok(), "the cap itself is allowed");
    }

    #[test]
    fn fuel_runs_out_at_the_same_point_every_time() {
        let caps = Caps {
            fuel: Fuel {
                per_byte: 1,
                base: 0,
            },
            ..Caps::DEFAULT
        };
        for _ in 0..3 {
            let mut b = Budget::new(caps, 100).unwrap();
            assert!(b.scan(60).is_ok());
            assert_eq!(b.fuel_remaining(), 40);
            let e = b.scan(41).unwrap_err();
            assert_eq!(cap_of(&e), "fuel");
            assert_eq!(b.fuel_remaining(), 0, "exhausted, not negative");
            // Still refuses afterwards: a budget that recovered would let a reader retry its
            // way past the cap one unit at a time.
            assert!(b.scan(1).is_err());
        }
    }

    #[test]
    fn the_node_cap_counts_nodes_and_fuel_separately() {
        let caps = Caps {
            nodes: 2,
            ..Caps::DEFAULT
        };
        let mut b = Budget::new(caps, 1_000).unwrap();
        b.node().unwrap();
        b.node().unwrap();
        let e = b.node().unwrap_err();
        assert_eq!(cap_of(&e), "nodes");
        assert_eq!(b.nodes(), 2, "the refused node was not counted");
        assert_eq!(b.fuel_spent(), 2, "nor charged");
    }

    #[test]
    fn an_archive_entry_is_checked_against_four_caps() {
        let caps = Caps {
            entries: 2,
            entry_bytes: 1_000,
            decompressed_bytes: 1_500,
            ratio: 10,
            ..Caps::DEFAULT
        };

        let mut b = Budget::new(caps, 100).unwrap();
        assert_eq!(cap_of(&b.entry(1_001, 1_001).unwrap_err()), "entry_bytes");

        let mut b = Budget::new(caps, 100).unwrap();
        b.entry(900, 900).unwrap();
        assert_eq!(
            cap_of(&b.entry(900, 900).unwrap_err()),
            "decompressed_bytes"
        );

        let mut b = Budget::new(caps, 100).unwrap();
        b.entry(700, 700).unwrap();
        b.entry(700, 700).unwrap();
        assert_eq!(cap_of(&b.entry(1, 1).unwrap_err()), "entries");

        // The ratio is only asked about over 1 MiB, so the small one passes and the big one
        // does not, at the same ratio.
        let caps = Caps {
            ratio: 10,
            ..Caps::DEFAULT
        };
        let mut b = Budget::new(caps, 100).unwrap();
        b.entry(1_000, 1).unwrap();
        assert_eq!(cap_of(&b.entry(2 << 20, 1).unwrap_err()), "ratio");

        // And an entry that claims to expand from nothing at all is the extreme case of the
        // same cap, not an exemption from it.
        let mut b = Budget::new(caps, 100).unwrap();
        assert_eq!(cap_of(&b.entry(2 << 20, 0).unwrap_err()), "ratio");
        let mut b = Budget::new(caps, 100).unwrap();
        b.entry(1_000, 0)
            .unwrap_or_else(|e| panic!("below the floor: {e}"));
    }

    #[test]
    fn nesting_is_a_depth_and_not_a_count() {
        let caps = Caps {
            nesting: 2,
            ..Caps::DEFAULT
        };
        let mut b = Budget::new(caps, 10).unwrap();
        b.enter().unwrap();
        b.enter().unwrap();
        assert_eq!(cap_of(&b.enter().unwrap_err()), "nesting");
        b.leave();
        b.enter().unwrap();
        assert_eq!(b.depth(), 2);
        // A document of a thousand sibling objects nests one deep.
        for _ in 0..1_000 {
            b.leave();
            b.enter().unwrap();
        }
    }

    #[test]
    fn raw_metadata_has_a_per_item_cap_and_a_total() {
        let caps = Caps {
            raw_bytes: (100, 250),
            ..Caps::DEFAULT
        };
        let mut b = Budget::new(caps, 10).unwrap();
        assert_eq!(cap_of(&b.raw(101).unwrap_err()), "raw_bytes");
        b.raw(100).unwrap();
        b.raw(100).unwrap();
        let e = b.raw(100).unwrap_err();
        assert_eq!(cap_of(&e), "raw_bytes");
        assert!(e.to_string().contains("250"), "the total is named: {e}");
    }

    #[test]
    fn a_part_over_the_hard_ceiling_is_refused() {
        let caps = Caps {
            part_bytes: 1_000,
            ..Caps::DEFAULT
        };
        let mut b = Budget::new(caps, 10_000).unwrap();
        b.part(1_000).unwrap();
        assert_eq!(cap_of(&b.part(1_001).unwrap_err()), "part_bytes");
    }

    #[test]
    fn every_refusal_is_e440_and_names_what_it_refused() {
        let mut b = Budget::new(
            Caps {
                nodes: 0,
                ..Caps::DEFAULT
            },
            1,
        )
        .unwrap();
        let e = b.node().unwrap_err();
        assert_eq!(e.code(), Some(Code::E440));
        let text = e.to_string();
        assert!(text.starts_with("SMY-E440"), "{text}");
        assert!(text.contains("nodes"), "{text}");
        assert!(text.contains("--max-nodes"), "{text}");
    }
}
