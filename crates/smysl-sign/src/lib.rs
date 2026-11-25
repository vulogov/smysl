//! Detached attestation signatures (RFC SMYSL-7).
//!
//! An `AgentId` is a string its writer chose. This crate lets an agent sign what it attested
//! with Ed25519 (RFC 8032), in a detached `COSE_Sign1` envelope (RFC 9052) kept beside the
//! store, so a verifier can tell an attestation its agent made from one that only names it.
//!
//! - The signed bytes are the record's canonical CBOR, `smysl_core::to_cbor`, inside RFC 9052's
//!   `Sig_structure` with the external AAD `smysl.sign/1`. No uid and no record changes, and a
//!   reader that never heard of signatures reads a signed store as it always did.
//! - A signature says who attested a record. It never raises a status.
//! - Pure: no file, clock, environment or randomness. A key is made from thirty-two bytes the
//!   caller drew ([`SigningKey::from_seed`]), a key file is parsed from its text
//!   ([`KeyFile::parse`]), and the CLI reads key and signature files.
//! - Three modes ([`VerificationMode`]): `Advisory` warns about what is unsigned, `Selective`
//!   requires signatures from some agents or on some statuses, `Strict` requires them all. A
//!   signature that is present and wrong fails every mode.
//! - Trust that reads signatures (RFC SMYSL-8): [`admission`] turns a mode into the
//!   `smysl_graph::AttestationAdmission` that salience's corroboration and origin authority
//!   count by. Exactly what `verify_store` fails under a mode is what it excludes.
//!
//! ```
//! use smysl_core::{AgentId, Attestation, Hlc, Op, Record, Rung, Uid};
//! use smysl_sign::{sign_record, verify_record, MemoryKeyResolver, RecordVerdict, ResolvedKey,
//!                  SigningKey};
//!
//! let alice: AgentId = "human:alice".parse().unwrap();
//! let key = SigningKey::from_seed(alice.clone(), [7; 32]);
//! let a = Attestation::new(Uid::from_bytes([1; 32]), alice.clone(), Op::Authored,
//!                          Rung::Document, Hlc::zero(alice.clone()));
//! let record = Record::Attestation(a);
//! let sig = sign_record(&key, &record).unwrap();
//!
//! let keys = MemoryKeyResolver::new().with(ResolvedKey::active(alice, key.public_key()));
//! assert!(matches!(verify_record(&sig, &record, &keys), RecordVerdict::Verified { .. }));
//! ```

#![forbid(unsafe_code)]

/// A value wiped when dropped: what [`SigningKey::seed`] and [`KeyFile::secret`] return.
pub use zeroize::Zeroizing;

mod cose;
mod key;
mod keyfile;
mod trust;
mod verify;

pub use cose::{
    read_signatures, sig_structure, DetachedSignature, SignError, TargetKind, ALG_EDDSA,
    COSE_SIGN1_TAG, EXTERNAL_AAD,
};
pub use key::{
    KeyError, KeyId, KeyResolver, KeyStatus, MemoryKeyResolver, ResolvedKey, SigningKey,
};
pub use keyfile::{parse_public_key, KeyFile, KeyFileError, ED25519_SPKI_PREFIX};
pub use trust::TrustConfig;
pub use verify::{
    admission, sign_record, verified_attesters, verify_record, verify_store, RecordVerdict,
    VerificationMode, VerificationReport,
};
