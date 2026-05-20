//! Pak signing and verification.
//!
//! A shipped pak — especially a Live Ops update pak (spec IV.8) — is signed so
//! the runtime can prove it came from the publisher before mounting it. The
//! signature covers the pak's deterministic [`Pak::to_bytes`] form.
//!
//! Cryptography is *not* owned (ADR-025): signing uses the audited
//! `ed25519-dalek` crate. This module is only the thin pak-shaped wrapper
//! around it.

use crate::pak::Pak;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// The number of bytes in an Ed25519 secret seed, public key, and signature.
pub const SEED_LEN: usize = 32;
/// The length of an encoded [`VerifyingKey`].
pub const PUBLIC_KEY_LEN: usize = 32;
/// The length of an encoded [`Signature`].
pub const SIGNATURE_LEN: usize = 64;

/// A publisher's Ed25519 signing key pair.
pub struct PakSigner {
    key: SigningKey,
}

impl PakSigner {
    /// Builds a signer from a 32-byte secret seed.
    ///
    /// The seed is the long-lived publisher secret; keep it out of the repo
    /// and out of shipped builds.
    pub fn from_seed(seed: &[u8; SEED_LEN]) -> Self {
        Self {
            key: SigningKey::from_bytes(seed),
        }
    }

    /// The public verifying key, for embedding in the runtime.
    pub fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        self.key.verifying_key().to_bytes()
    }

    /// Signs `pak`, returning the detached 64-byte signature over its
    /// deterministic serialized form.
    pub fn sign(&self, pak: &Pak) -> [u8; SIGNATURE_LEN] {
        self.key.sign(&pak.to_bytes()).to_bytes()
    }
}

/// Verifies that `signature` over `pak` was produced by the holder of the
/// secret matching `public_key`.
///
/// Returns `false` on any malformed input as well as on a genuine mismatch —
/// an unmountable pak and a forged pak are both simply "do not mount".
pub fn verify(
    pak: &Pak,
    signature: &[u8; SIGNATURE_LEN],
    public_key: &[u8; PUBLIC_KEY_LEN],
) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let sig = Signature::from_bytes(signature);
    key.verify(&pak.to_bytes(), &sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pak(payload: &[u8]) -> Pak {
        let mut b = Pak::builder();
        b.add("data.bin", payload.to_vec());
        b.build()
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let signer = PakSigner::from_seed(&[7u8; SEED_LEN]);
        let pak = sample_pak(b"live-ops-update");
        let sig = signer.sign(&pak);
        assert!(verify(&pak, &sig, &signer.public_key()));
    }

    #[test]
    fn a_tampered_pak_fails_verification() {
        let signer = PakSigner::from_seed(&[7u8; SEED_LEN]);
        let sig = signer.sign(&sample_pak(b"original"));
        // Same signature, different pak contents.
        assert!(!verify(
            &sample_pak(b"tampered"),
            &sig,
            &signer.public_key()
        ));
    }

    #[test]
    fn a_different_key_fails_verification() {
        let publisher = PakSigner::from_seed(&[1u8; SEED_LEN]);
        let attacker = PakSigner::from_seed(&[2u8; SEED_LEN]);
        let pak = sample_pak(b"payload");
        let sig = attacker.sign(&pak);
        assert!(!verify(&pak, &sig, &publisher.public_key()));
    }

    #[test]
    fn a_malformed_public_key_is_rejected_not_panicked() {
        let signer = PakSigner::from_seed(&[7u8; SEED_LEN]);
        let pak = sample_pak(b"payload");
        let sig = signer.sign(&pak);
        // An all-zero point is not a valid Ed25519 public key.
        assert!(!verify(&pak, &sig, &[0u8; PUBLIC_KEY_LEN]));
    }
}
