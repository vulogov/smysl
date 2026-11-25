//! Keys (RFC SMYSL-7 §5): the signing key, its id, and how a verifier finds a public key.

use std::collections::BTreeMap;
use std::fmt;

use smysl_core::{AgentId, Status};

/// A key's id: the first 16 bytes of `BLAKE3-256(public key)`, the COSE `kid` (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(pub [u8; 16]);

impl KeyId {
    /// The id of an Ed25519 public key.
    pub fn of(public_key: &[u8; 32]) -> KeyId {
        let h = smysl_core::hash_bytes(public_key);
        let mut id = [0u8; 16];
        id.copy_from_slice(&h[..16]);
        KeyId(id)
    }
}

impl fmt::Display for KeyId {
    /// Lowercase hex, 32 digits.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// An Ed25519 signing key bound to the agent it signs as.
///
/// The secret half is wiped when the key is dropped, and `Debug` shows only the agent and the
/// key id. There is no generator here: the crate draws no randomness (§2, G2). The caller draws
/// thirty-two bytes, as `smysl keygen` does with `getrandom`, and passes them to
/// [`SigningKey::from_seed`].
pub struct SigningKey {
    inner: ed25519_dalek::SigningKey,
    agent: AgentId,
}

impl SigningKey {
    /// The key whose RFC 8032 secret is `seed`, signing as `agent`.
    pub fn from_seed(agent: AgentId, seed: [u8; 32]) -> SigningKey {
        let mut seed = zeroize::Zeroizing::new(seed);
        let inner = ed25519_dalek::SigningKey::from_bytes(&seed);
        zeroize::Zeroize::zeroize(&mut *seed);
        SigningKey { inner, agent }
    }

    /// The agent this key signs as.
    pub fn agent(&self) -> &AgentId {
        &self.agent
    }

    /// The 32-byte public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.inner.verifying_key().to_bytes()
    }

    /// The key's id.
    pub fn kid(&self) -> KeyId {
        KeyId::of(&self.public_key())
    }

    /// The secret, for writing a key file. Wiped when the returned value is dropped.
    pub fn seed(&self) -> zeroize::Zeroizing<[u8; 32]> {
        zeroize::Zeroizing::new(self.inner.to_bytes())
    }

    pub(crate) fn sign(&self, msg: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        self.inner.sign(msg).to_bytes()
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("agent", &self.agent)
            .field("kid", &self.kid())
            .finish_non_exhaustive()
    }
}

/// Where a public key stands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyStatus {
    /// Valid.
    Active,
    /// Signatures on attestations whose clock reads `after_ms` or later are refused; every
    /// signature when `None`, which is how a key revoked for compromise is revoked (§9).
    Revoked {
        after_ms: Option<u64>,
        reason: Option<String>,
    },
    /// Valid for attestations whose clock reads within `[not_before_ms, not_after_ms]`.
    Bounded {
        not_before_ms: u64,
        not_after_ms: u64,
    },
}

/// A public key, as a resolver found it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedKey {
    pub kid: KeyId,
    pub agent: AgentId,
    pub public_key: [u8; 32],
    pub status: KeyStatus,
    /// The highest status this key may vouch for, read against the unit an attestation
    /// names (R1-9).
    pub max_status: Option<Status>,
}

impl ResolvedKey {
    /// An active key with no cap; its id is computed from `public_key`.
    pub fn active(agent: AgentId, public_key: [u8; 32]) -> ResolvedKey {
        ResolvedKey {
            kid: KeyId::of(&public_key),
            agent,
            public_key,
            status: KeyStatus::Active,
            max_status: None,
        }
    }

    pub fn with_status(mut self, status: KeyStatus) -> ResolvedKey {
        self.status = status;
        self
    }

    pub fn with_max_status(mut self, max: Status) -> ResolvedKey {
        self.max_status = Some(max);
        self
    }
}

/// Why a key was not found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyError {
    /// No key with this id for this agent.
    NotFound { agent: AgentId, kid: KeyId },
    /// The key exists, and belongs to another agent.
    OtherAgent {
        agent: AgentId,
        kid: KeyId,
        bound_to: AgentId,
    },
    /// The resolver itself failed.
    Resolver(String),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::NotFound { agent, kid } => write!(f, "no key {kid} for {agent}"),
            KeyError::OtherAgent {
                agent,
                kid,
                bound_to,
            } => write!(f, "key {kid} belongs to {bound_to}, not {agent}"),
            KeyError::Resolver(e) => write!(f, "key resolver: {e}"),
        }
    }
}

impl std::error::Error for KeyError {}

/// Where a verifier finds public keys. Synchronous and pure: an implementation may wrap a map,
/// a parsed key file or a cache, and does its I/O before it is asked.
pub trait KeyResolver {
    fn resolve(&self, agent: &AgentId, kid: &KeyId) -> Result<ResolvedKey, KeyError>;
}

/// Keys held in memory, by agent and key id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryKeyResolver {
    keys: BTreeMap<(AgentId, KeyId), ResolvedKey>,
}

impl MemoryKeyResolver {
    pub fn new() -> MemoryKeyResolver {
        MemoryKeyResolver::default()
    }

    /// Add a key; a later key with the same agent and id replaces an earlier one.
    pub fn insert(&mut self, key: ResolvedKey) {
        self.keys.insert((key.agent.clone(), key.kid), key);
    }

    pub fn with(mut self, key: ResolvedKey) -> MemoryKeyResolver {
        self.insert(key);
        self
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The keys, in agent and id order.
    pub fn keys(&self) -> impl Iterator<Item = &ResolvedKey> {
        self.keys.values()
    }

    /// Set the status of the key with id `kid`, whichever agent holds it; false when none does
    /// (RFC SMYSL-8 §6, R1-9).
    pub fn set_status(&mut self, kid: &KeyId, status: KeyStatus) -> bool {
        match self.keys.values_mut().find(|k| &k.kid == kid) {
            Some(k) => {
                k.status = status;
                true
            }
            None => false,
        }
    }
}

impl KeyResolver for MemoryKeyResolver {
    fn resolve(&self, agent: &AgentId, kid: &KeyId) -> Result<ResolvedKey, KeyError> {
        if let Some(k) = self.keys.get(&(agent.clone(), *kid)) {
            return Ok(k.clone());
        }
        match self.keys.iter().find(|((_, id), _)| id == kid) {
            Some(((owner, _), _)) => Err(KeyError::OtherAgent {
                agent: agent.clone(),
                kid: *kid,
                bound_to: owner.clone(),
            }),
            None => Err(KeyError::NotFound {
                agent: agent.clone(),
                kid: *kid,
            }),
        }
    }
}
