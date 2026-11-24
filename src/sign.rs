//! `smysl keygen`, `sign` and `verify`, the `--sign` flags, `check --verify-sigs`, and the
//! signature files (RFC SMYSL-7 §8, SG-P3).
//!
//! The crate is pure (G2); this is where its I/O lives. A store's signatures are a CBOR
//! sequence of `COSE_Sign1` envelopes beside it: `<store>.sig` for a file, and
//! `<library>/log/sigs.cbor` for a library. Every writer writes the file whole, in one order
//! (by target, then key, then bytes) with duplicates dropped, so two merges of the same stores
//! in either order write the same signature file.

use std::path::{Path, PathBuf};

use clap::ArgMatches;
use smysl::sign::{
    read_signatures, sign_record, verify_store, DetachedSignature, KeyFile, MemoryKeyResolver,
    RecordVerdict, ResolvedKey, SigningKey, VerificationMode, VerificationReport,
};
use smysl::{AgentId, ExitCode, Record, Status, Store};

/// The signature file beside a store: `<library>/log/sigs.cbor` for a library, `<store>.sig`
/// for a file. None for stdin.
pub(crate) fn sig_path(store: &str) -> Option<PathBuf> {
    if store == "-" {
        return None;
    }
    let p = Path::new(store);
    if smysl::text::library::Library::is_library(p) {
        return Some(p.join("log").join("sigs.cbor"));
    }
    Some(PathBuf::from(format!("{store}.sig")))
}

/// The signature file `--sigs` names, or the one beside `store`.
fn sigs_file(m: &ArgMatches, store: &str, cmd: &str) -> Result<PathBuf, ExitCode> {
    if let Some(p) = m.try_get_one::<String>("sigs").ok().flatten() {
        return Ok(PathBuf::from(p));
    }
    sig_path(store).ok_or_else(|| {
        eprintln!("smysl {cmd}: a store read from stdin has no signature file beside it; name one with --sigs");
        ExitCode::Usage
    })
}

/// A signature file's envelopes; none when there is no file. A file that does not read is an
/// error, not an empty list: a damaged signature is not an absent one.
pub(crate) fn read_sigs(path: &Path) -> Result<Vec<DetachedSignature>, String> {
    match std::fs::read(path) {
        Ok(bytes) => read_signatures(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Write a signature file whole, in its one order, through a temporary file and a rename.
/// No signatures and no file leaves no file; no signatures and a file removes it.
pub(crate) fn write_sigs(path: &Path, sigs: &[DetachedSignature]) -> Result<(), String> {
    let mut envelopes: Vec<(&DetachedSignature, Vec<u8>)> =
        sigs.iter().map(|s| (s, s.to_cose())).collect();
    envelopes.sort_by(|(a, x), (b, y)| (a.target_hash, a.kid, x).cmp(&(b.target_hash, b.kid, y)));
    envelopes.dedup_by(|(_, x), (_, y)| x == y);
    if envelopes.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {e}", path.display()))
            }
            _ => Ok(()),
        };
    }
    let bytes: Vec<u8> = envelopes.into_iter().flat_map(|(_, b)| b).collect();
    let tmp = path.with_extension("sig.tmp");
    std::fs::write(&tmp, &bytes)
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The store a command names positionally or with `-s`.
fn store_arg(m: &ArgMatches, global: &ArgMatches, cmd: &str) -> Result<String, ExitCode> {
    match m
        .get_one::<String>("path")
        .or_else(|| global.get_one::<String>("store"))
    {
        Some(p) => Ok(p.clone()),
        None => {
            eprintln!("smysl {cmd}: no store given");
            Err(ExitCode::Usage)
        }
    }
}

/// Read a signing key file.
pub(crate) fn load_signing_key(path: &str) -> Result<SigningKey, String> {
    let text = smysl::sign::Zeroizing::new(
        std::fs::read_to_string(path).map_err(|e| format!("--key {path}: {e}"))?,
    );
    KeyFile::parse_secret(&text).map_err(|e| format!("--key {path}: {e}"))
}

/// Sign `records`' attestations by `key`'s agent into the file at `path`, keeping what is there.
/// Returns how many were signed now and how many already were.
pub(crate) fn sign_into<'a>(
    path: &Path,
    key: &SigningKey,
    records: impl IntoIterator<Item = &'a Record>,
) -> Result<(usize, usize), String> {
    let mut sigs = read_sigs(path)?;
    let (mut new, mut had) = (0, 0);
    for r in records {
        let Record::Attestation(a) = r else { continue };
        if &a.agent != key.agent() {
            continue;
        }
        let hash = Store::record_hash(r);
        if sigs
            .iter()
            .any(|s| s.target_hash == hash && s.kid == key.kid())
        {
            had += 1;
            continue;
        }
        sigs.push(sign_record(key, r).map_err(|e| e.to_string())?);
        new += 1;
    }
    write_sigs(path, &sigs)?;
    Ok((new, had))
}

/// The mode named by `name`, with `verify_args`' scope.
pub(crate) fn mode_of(m: &ArgMatches, name: &str) -> Result<VerificationMode, String> {
    let prefixes: Vec<String> = m
        .get_many::<String>("agent-prefix")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let status = match m.get_one::<String>("status-at-least") {
        None => None,
        Some(s) => Some(Status::parse(s).ok_or_else(|| {
            format!(
                "--status-at-least {s}: the statuses are {}",
                Status::ALL
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?),
    };
    let scoped = !prefixes.is_empty() || status.is_some();
    match name {
        "advisory" | "strict" if scoped => Err(format!(
            "--agent-prefix and --status-at-least set the scope of selective, not of {name}"
        )),
        "advisory" => Ok(VerificationMode::Advisory),
        "strict" => Ok(VerificationMode::Strict),
        "selective" if !scoped => {
            Err("selective needs a scope: --agent-prefix, --status-at-least, or both".into())
        }
        "selective" => Ok(VerificationMode::selective(prefixes, status)),
        other => Err(format!(
            "no mode `{other}`; the modes are advisory, selective and strict"
        )),
    }
}

/// The key file `--keys` names, or `.smysl/keys.hjson` beside the store. Missing or malformed,
/// it is a usage error (§8).
pub(crate) fn load_keys(m: &ArgMatches, store: &str) -> Result<MemoryKeyResolver, String> {
    let path = match m.get_one::<String>("keys") {
        Some(p) => PathBuf::from(p),
        None => {
            let p = Path::new(store);
            let dir = if smysl::text::library::Library::is_library(p) {
                p.to_path_buf()
            } else {
                p.parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            };
            dir.join(".smysl").join("keys.hjson")
        }
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    KeyFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn mode_name(m: &VerificationMode) -> &'static str {
    match m {
        VerificationMode::Advisory => "advisory",
        VerificationMode::Strict => "strict",
        _ => "selective",
    }
}

fn verdict_name(v: &RecordVerdict) -> &'static str {
    match v {
        RecordVerdict::Verified { .. } => "verified",
        RecordVerdict::Unsigned => "unsigned",
        RecordVerdict::UnknownKey { .. } => "unknown-key",
        RecordVerdict::StatusOverCap { .. } => "status-over-cap",
        RecordVerdict::RevokedKey { .. } => "revoked-key",
        RecordVerdict::AgentMismatch { .. } => "agent-mismatch",
        RecordVerdict::InvalidSignature { .. } => "invalid-signature",
        _ => "unknown",
    }
}

fn verdict_text(v: &RecordVerdict) -> String {
    match v {
        RecordVerdict::Verified { agent, kid } => format!("signed by {agent} with key {kid}"),
        RecordVerdict::Unsigned => "no signature".into(),
        RecordVerdict::UnknownKey { agent, kid } => {
            format!("signed by {agent} with key {kid}, which the key file does not hold")
        }
        RecordVerdict::StatusOverCap {
            agent,
            kid,
            status,
            max_status,
        } => format!(
            "{agent}'s key {kid} vouches up to {}; the unit is {}",
            max_status.as_str(),
            status.as_str()
        ),
        RecordVerdict::RevokedKey { agent, kid, reason } => format!(
            "{agent}'s key {kid} is revoked or out of its bounds{}",
            reason
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default()
        ),
        RecordVerdict::AgentMismatch {
            header_agent,
            record_agent,
        } => format!("the signature names {header_agent}; the record or key is {record_agent}'s"),
        RecordVerdict::InvalidSignature { agent, kid } => {
            format!("the signature by {agent} with key {kid} does not verify over this record")
        }
        other => format!("{other:?}"),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// What a record is, for a finding.
fn describe(r: &Record) -> String {
    match r {
        Record::Attestation(a) => format!("attestation by {} on {}", a.agent, a.uid),
        other => format!(
            "{} {}",
            other.type_name(),
            &hex(&Store::record_hash(other))[..16]
        ),
    }
}

/// One finding per verdict short of verified: (record hash, error?, record, verdict).
fn findings<'a>(
    store: &'a Store,
    report: &'a VerificationReport,
) -> Vec<([u8; 32], bool, Option<&'a Record>, &'a RecordVerdict)> {
    let by_hash: std::collections::BTreeMap<[u8; 32], &Record> =
        store.iter().map(|r| (Store::record_hash(r), r)).collect();
    report
        .verdicts
        .iter()
        .filter(|(_, v)| !v.is_verified())
        .map(|(h, v)| (*h, report.failing.contains(h), by_hash.get(h).copied(), v))
        .collect()
}

/// Print a report: the findings, then a summary. JSON is one object per store.
pub(crate) fn print_report(
    path: &str,
    store: &Store,
    report: &VerificationReport,
    json: bool,
    quiet: bool,
) {
    let found = findings(store, report);
    if json {
        let items: Vec<String> = found
            .iter()
            .map(|(h, err, r, v)| {
                format!(
                    "{{\"record\":\"{}\",\"severity\":\"{}\",\"verdict\":\"{}\",\"subject\":{},\"message\":{}}}",
                    hex(h),
                    if *err { "error" } else { "warning" },
                    verdict_name(v),
                    smysl::json_escape(&r.map(describe).unwrap_or_default()),
                    smysl::json_escape(&verdict_text(v)),
                )
            })
            .collect();
        println!(
            "{{\"store\":{},\"mode\":\"{}\",\"passed\":{},\"verified\":{},\"unsigned\":{},\"warnings\":{},\"errors\":{},\"unmatched\":{},\"findings\":[{}]}}",
            smysl::json_escape(path),
            mode_name(&report.mode),
            report.passed,
            report.verified_count,
            report.unsigned_count,
            report.warning_count,
            report.error_count,
            report.unmatched_count,
            items.join(",")
        );
        return;
    }
    for (_, err, r, v) in &found {
        if quiet && !err {
            continue;
        }
        println!(
            "{path}: {}: {}: {}",
            if *err { "error" } else { "warning" },
            r.map(describe).unwrap_or_default(),
            verdict_text(v)
        );
    }
    if quiet && report.passed {
        return;
    }
    let unmatched = if report.unmatched_count > 0 {
        format!(
            ", {} signature(s) for records the store does not hold",
            report.unmatched_count
        )
    } else {
        String::new()
    };
    println!(
        "{path}: {}: {} verified, {} unsigned, {} warning(s), {} error(s){unmatched}; {}",
        mode_name(&report.mode),
        report.verified_count,
        report.unsigned_count,
        report.warning_count,
        report.error_count,
        if report.passed { "passed" } else { "failed" }
    );
}

/// Verify the store at `path` for `check --verify-sigs` and `verify`. Usage errors are 2;
/// a store or signature file that does not read is 1.
pub(crate) fn verify_path(
    m: &ArgMatches,
    path: &str,
    keys: &MemoryKeyResolver,
    mode: &VerificationMode,
    cmd: &str,
) -> Result<(Store, VerificationReport), ExitCode> {
    let sig_file = sigs_file(m, path, cmd)?;
    let (store, _) = super::load_memory(path).map_err(|e| {
        eprintln!("smysl {cmd}: {e}");
        ExitCode::Failure
    })?;
    let sigs = read_sigs(&sig_file).map_err(|e| {
        eprintln!("smysl {cmd}: {e}");
        ExitCode::Failure
    })?;
    let report = verify_store(&store, &sigs, keys, mode);
    Ok((store, report))
}

/// `smysl keygen --agent A --out FILE`: a new signing key, written owner-only; its key file
/// entry on stdout.
pub(crate) fn cmd_keygen(m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    let Some(agent) = m.get_one::<String>("agent") else {
        eprintln!("smysl keygen: --agent is required");
        return ExitCode::Usage;
    };
    let agent: AgentId = match agent.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl keygen: --agent {agent}: {e}");
            return ExitCode::Usage;
        }
    };
    let Some(out) = m.get_one::<String>("out") else {
        eprintln!("smysl keygen: --out is required; a secret key is not written to stdout");
        return ExitCode::Usage;
    };
    let mut seed = smysl::sign::Zeroizing::new([0u8; 32]);
    // A failure here is the operating system saying it has no randomness, which is not a thing
    // to paper over with a weaker key: the process ends instead.
    getrandom::fill(&mut seed[..]).expect("the operating system's randomness");
    let key = SigningKey::from_seed(agent, *seed);
    let text = KeyFile::secret(&key);
    if let Err(e) = write_private(Path::new(out), text.as_bytes()) {
        eprintln!("smysl keygen: {out}: {e}");
        return ExitCode::Failure;
    }
    println!(
        "{}",
        KeyFile::entry(&ResolvedKey::active(key.agent().clone(), key.public_key()))
    );
    eprintln!("smysl keygen: wrote {out}; add the line above to the `keys` of .smysl/keys.hjson");
    ExitCode::Success
}

/// Create `path` for its owner alone, refusing one that exists.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(bytes)
}

/// `smysl sign -s STORE --key FILE`: sign the store's attestations by the key's agent.
pub(crate) fn cmd_sign(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match store_arg(m, global, "sign") {
        Ok(p) => p,
        Err(c) => return c,
    };
    let Some(key_path) = m.get_one::<String>("key") else {
        eprintln!("smysl sign: --key is required");
        return ExitCode::Usage;
    };
    let key = match load_signing_key(key_path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("smysl sign: {e}");
            return ExitCode::Usage;
        }
    };
    let sig_file = match sigs_file(m, &path, "sign") {
        Ok(p) => p,
        Err(c) => return c,
    };
    let (store, _) = match super::load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl sign: {e}");
            return ExitCode::Failure;
        }
    };
    match sign_into(&sig_file, &key, store.iter()) {
        Ok((new, had)) => {
            if global.get_flag("json") {
                println!(
                    "{{\"store\":{},\"signatures\":{},\"agent\":{},\"kid\":\"{}\",\"signed\":{new},\"already_signed\":{had}}}",
                    smysl::json_escape(&path),
                    smysl::json_escape(&sig_file.display().to_string()),
                    smysl::json_escape(key.agent().as_str()),
                    key.kid()
                );
            } else if !global.get_flag("quiet") {
                println!(
                    "{path}: signed {new} attestation(s) by {} into {}; {had} already signed",
                    key.agent(),
                    sig_file.display()
                );
            }
            ExitCode::Success
        }
        Err(e) => {
            eprintln!("smysl sign: {e}");
            ExitCode::Failure
        }
    }
}

/// `smysl verify -s STORE`: 0 when the report passes, 1 when it does not, 2 on a usage error
/// or a key file that is missing or malformed (§8).
pub(crate) fn cmd_verify(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match store_arg(m, global, "verify") {
        Ok(p) => p,
        Err(c) => return c,
    };
    let name = m
        .get_one::<String>("mode")
        .map(String::as_str)
        .unwrap_or("advisory");
    let mode = match mode_of(m, name) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl verify: {e}");
            return ExitCode::Usage;
        }
    };
    let keys = match load_keys(m, &path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("smysl verify: {e}");
            return ExitCode::Usage;
        }
    };
    let (store, report) = match verify_path(m, &path, &keys, &mode, "verify") {
        Ok(v) => v,
        Err(c) => return c,
    };
    print_report(
        &path,
        &store,
        &report,
        global.get_flag("json"),
        global.get_flag("quiet"),
    );
    if report.passed {
        ExitCode::Success
    } else {
        ExitCode::Failure
    }
}

/// `check --verify-sigs`: the findings in `check`'s own shapes, a diagnostic line each, and the
/// summary unless `--quiet` or `--json`.
pub(crate) fn print_check(
    path: &str,
    store: &Store,
    report: &VerificationReport,
    json: bool,
    quiet: bool,
) {
    for (_, err, r, v) in findings(store, report) {
        let severity = if err { "error" } else { "warning" };
        let message = format!(
            "{}: {}",
            r.map(describe).unwrap_or_default(),
            verdict_text(v)
        );
        if json {
            println!(
                "{{\"code\":\"sign/{}\",\"severity\":\"{severity}\",\"message\":{}}}",
                verdict_name(v),
                smysl::json_escape(&message)
            );
        } else {
            eprintln!("{path}: {severity}: sign/{}: {message}", verdict_name(v));
        }
    }
    if !json && !quiet {
        println!(
            "{path}: signatures ({}): {} verified, {} unsigned, {} warning(s), {} error(s)",
            mode_name(&report.mode),
            report.verified_count,
            report.unsigned_count,
            report.warning_count,
            report.error_count
        );
    }
}

/// `check --verify-sigs MODE`: verify `path`, print, and say whether it passed.
pub(crate) fn check_sigs(
    m: &ArgMatches,
    global: &ArgMatches,
    path: &str,
    name: &str,
) -> Result<bool, ExitCode> {
    let mode = mode_of(m, name).map_err(|e| {
        eprintln!("smysl check: --verify-sigs: {e}");
        ExitCode::Usage
    })?;
    let keys = load_keys(m, path).map_err(|e| {
        eprintln!("smysl check: --verify-sigs: {e}");
        ExitCode::Usage
    })?;
    let (store, report) = verify_path(m, path, &keys, &mode, "check")?;
    print_check(
        path,
        &store,
        &report,
        global.get_flag("json"),
        global.get_flag("quiet"),
    );
    Ok(report.passed)
}

/// The signatures beside `merge`'s inputs, read before anything is merged.
pub(crate) fn input_sigs(inputs: &[String]) -> Result<Vec<DetachedSignature>, String> {
    let mut all = Vec::new();
    for p in inputs {
        if let Some(f) = sig_path(p) {
            all.extend(read_sigs(&f)?);
        }
    }
    Ok(all)
}

/// Sign the staged attestations by `key`'s agent; the count of those left to other agents.
pub(crate) fn sign_staged(
    key: &SigningKey,
    staged: &[Record],
) -> Result<(Vec<DetachedSignature>, usize), String> {
    let mut sigs = Vec::new();
    let mut others = 0;
    for r in staged {
        let Record::Attestation(a) = r else { continue };
        if &a.agent == key.agent() {
            sigs.push(sign_record(key, r).map_err(|e| e.to_string())?);
        } else {
            others += 1;
        }
    }
    Ok((sigs, others))
}

// ---- RFC SMYSL-8: trust that reads signatures --------------------------------------------

/// A command's signature policy, resolved: the mode, the keys, and what it admits.
pub(crate) struct Trust {
    pub(crate) mode: VerificationMode,
    pub(crate) keys: MemoryKeyResolver,
    pub(crate) admission: smysl::AttestationAdmission,
}

/// `trust:` in the project's configuration (`-C`, else `.smysl/config.hjson` beside the store),
/// and the directory a relative `keys:` in it is read against. None when there is no file or
/// it has no `trust:`; a `trust:` that does not parse is an error, not an absent policy.
fn config_trust(
    global: &ArgMatches,
    store: &str,
) -> Result<Option<(smysl::sign::TrustConfig, PathBuf)>, String> {
    let named = global.try_get_one::<String>("config").ok().flatten();
    let (path, base) = match named {
        Some(c) => {
            let p = PathBuf::from(c);
            let base = p
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
            (p, base)
        }
        None => {
            let root = Path::new(store)
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
            (root.join(".smysl").join("config.hjson"), root)
        }
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && named.is_none() => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    smysl::sign::TrustConfig::parse(&text)
        .map(|t| t.map(|t| (t, base)))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The policy `--trust-sigs` names, else the configuration's `trust:`, applied to `store`;
/// None for neither, which is every result as it was. A policy that cannot be applied — a
/// missing or malformed key file, a damaged signature file — is a usage error before anything
/// is counted (RFC SMYSL-8 §5): it is not silently dropped.
pub(crate) fn trust<S: smysl::StoreRead + ?Sized>(
    m: &ArgMatches,
    global: &ArgMatches,
    path: &str,
    store: &S,
    cmd: &str,
) -> Result<Option<Trust>, ExitCode> {
    let fail = |e: String| {
        eprintln!("smysl {cmd}: {e}");
        ExitCode::Usage
    };
    let scoped = m
        .try_get_many::<String>("agent-prefix")
        .ok()
        .flatten()
        .is_some()
        || m.try_get_one::<String>("status-at-least")
            .ok()
            .flatten()
            .is_some();
    let (mode, keys) = match m.try_get_one::<String>("trust-sigs").ok().flatten() {
        Some(name) => {
            let mode = mode_of(m, name).map_err(|e| fail(format!("--trust-sigs: {e}")))?;
            (mode, load_keys(m, path).map_err(fail)?)
        }
        None => {
            if scoped {
                return Err(fail(
                    "--agent-prefix and --status-at-least scope --trust-sigs selective".into(),
                ));
            }
            let Some((cfg, base)) = config_trust(global, path).map_err(fail)? else {
                return Ok(None);
            };
            let keys = match (m.try_get_one::<String>("keys").ok().flatten(), &cfg.keys) {
                (None, Some(k)) => {
                    let p = base.join(k);
                    let text = std::fs::read_to_string(&p)
                        .map_err(|e| fail(format!("{}: {e}", p.display())))?;
                    KeyFile::parse(&text).map_err(|e| fail(format!("{}: {e}", p.display())))?
                }
                _ => load_keys(m, path).map_err(fail)?,
            };
            (cfg.mode, keys)
        }
    };
    let sigs = read_sigs(&sigs_file(m, path, cmd)?).map_err(fail)?;
    let admission = smysl::sign::admission(store, &sigs, &keys, &mode);
    Ok(Some(Trust {
        mode,
        keys,
        admission,
    }))
}

/// The `--key` files given, each loaded; a key that does not load is a usage error.
pub(crate) fn signing_keys(m: &ArgMatches, cmd: &str) -> Result<Vec<SigningKey>, ExitCode> {
    m.try_get_many::<String>("key")
        .ok()
        .flatten()
        .into_iter()
        .flatten()
        .map(|p| {
            load_signing_key(p).map_err(|e| {
                eprintln!("smysl {cmd}: {e}");
                ExitCode::Usage
            })
        })
        .collect()
}

/// The attestation an agent records on a retraction or withdrawal: `op: attested`, rung
/// `document`, at `now_ms` (RFC SMYSL-7 R1-16).
pub(crate) fn edge_attestation(subject: smysl::Uid, agent: &AgentId, now_ms: u64) -> Record {
    Record::Attestation(smysl::Attestation::new(
        subject,
        agent.clone(),
        smysl::Op::Attested,
        smysl::Rung::Document,
        smysl::Hlc::new(now_ms, 0, agent.clone()),
    ))
}

/// Under a policy, each `--as` agent the mode puts in scope must be proven by a `--key` of
/// that agent which verifies against the key file (RFC SMYSL-8 §4.2, R2-5): a key file alone is
/// no proof, since anyone can make a key for any id. The proof is the attestation `--sign`
/// would record, signed and verified now, so a revoked, out-of-bounds or over-cap key fails
/// it. An unproven agent out of scope — every agent under `advisory` — is a warning.
pub(crate) fn prove(
    trust: &Trust,
    agents: &[AgentId],
    keys: &[SigningKey],
    subject: smysl::Uid,
    subject_status: Option<Status>,
    now_ms: u64,
    cmd: &str,
) -> Result<(), ExitCode> {
    for a in agents {
        match keys.iter().find(|k| k.agent() == a) {
            Some(k) => {
                let record = edge_attestation(subject, a, now_ms);
                let sig = sign_record(k, &record).map_err(|e| {
                    eprintln!("smysl {cmd}: {e}");
                    ExitCode::Usage
                })?;
                let v = smysl::sign::verify_record(&sig, &record, &trust.keys);
                if !v.is_verified() {
                    eprintln!(
                        "smysl {cmd}: --key for {a} does not prove it: {}",
                        verdict_text(&v)
                    );
                    return Err(ExitCode::Failure);
                }
            }
            None if trust.mode.in_scope(a, subject_status) => {
                eprintln!(
                    "smysl {cmd}: --as {a} needs a --key of {a} under --trust-sigs {}",
                    mode_name(&trust.mode)
                );
                return Err(ExitCode::Failure);
            }
            None => eprintln!(
                "smysl {cmd}: warning: --as {a} is not proven by a key ({} tolerates it)",
                mode_name(&trust.mode)
            ),
        }
    }
    Ok(())
}
