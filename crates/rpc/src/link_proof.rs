//! How a phone and its desktop prove to each other that they hold the
//! phone-link key, without the key ever crossing the wire.
//!
//! The key reaches the phone once, on the desktop's screen (the v=2 code).
//! A dial then proves it both ways, desktop first:
//!
//! 1. The phone's handshake carries a fresh nonce ([`NONCE_HEADER`]) and no
//!    secret.
//! 2. The desktop answers the upgrade with its own fresh nonce and
//!    [`desktop_proof`]: HMAC-SHA256 under the key over both nonces. The
//!    phone checks it before it sends anything: an address that answers
//!    without the key (a wrong DNS answer, someone else on the network)
//!    learns nothing, not even a hash of the key it could replay.
//! 3. The phone's first frame is [`phone_proof`] over the same nonces
//!    (`{"linkProof": "…"}`). Until it matches, the desktop serves nothing.
//!
//! Both nonces are fresh on each dial, and the two proofs are labelled
//! apart, so no proof answers another dial or the other side's challenge.
//! What follows is plain WebSocket: the tailnet's WireGuard tunnel carries
//! it, and the desktop serves only peers inside it ([`crate::remote`]).

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The handshake header that carries a side's nonce (the phone's request,
/// the desktop's response).
pub const NONCE_HEADER: &str = "x-zend-link-nonce";
/// The response header that carries [`desktop_proof`].
pub const PROOF_HEADER: &str = "x-zend-link-proof";
/// The field of the phone's first frame that carries [`phone_proof`].
pub const PROOF_FIELD: &str = "linkProof";

/// A fresh nonce: 32 bytes from the OS CSPRNG, lowercase hex.
pub fn nonce() -> std::io::Result<String> {
    crate::ipc_auth::generate_token()
}

/// Whether `nonce` is one [`nonce`] could have made (64 hex digits).
pub fn valid_nonce(nonce: &str) -> bool {
    nonce.len() == 64 && nonce.bytes().all(|b| b.is_ascii_hexdigit())
}

fn proof(label: &str, key: &str, phone_nonce: &str, desktop_nonce: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.trim().as_bytes()).expect("HMAC takes a key of any length");
    for part in [label, phone_nonce, desktop_nonce] {
        mac.update(part.as_bytes());
        mac.update(b"\n");
    }
    crate::ipc_auth::hex(&mac.finalize().into_bytes())
}

/// What the desktop answers a phone's nonce with.
pub fn desktop_proof(key: &str, phone_nonce: &str, desktop_nonce: &str) -> String {
    proof("zend phone link v1: desktop", key, phone_nonce, desktop_nonce)
}

/// What the phone sends first, once the desktop has proved itself.
pub fn phone_proof(key: &str, phone_nonce: &str, desktop_nonce: &str) -> String {
    proof("zend phone link v1: phone", key, phone_nonce, desktop_nonce)
}

/// The phone's first frame.
pub fn phone_frame(key: &str, phone_nonce: &str, desktop_nonce: &str) -> String {
    serde_json::json!({ PROOF_FIELD: phone_proof(key, phone_nonce, desktop_nonce) }).to_string()
}

/// Whether `presented` is `expected`, in constant time.
pub fn proof_matches(expected: &str, presented: &str) -> bool {
    crate::ipc_auth::constant_time_eq(expected.as_bytes(), presented.trim().as_bytes())
}

/// Whether a phone's first frame carries `expected`.
pub fn frame_proves(frame: &str, expected: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(frame)
        .ok()
        .and_then(|v| v.get(PROOF_FIELD)?.as_str().map(|p| proof_matches(expected, p)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proof_answers_only_its_own_key_nonces_and_side() {
        let key = crate::ipc_auth::generate_token().unwrap();
        let (np, nd) = (nonce().unwrap(), nonce().unwrap());
        let desktop = desktop_proof(&key, &np, &nd);
        assert_eq!(desktop.len(), 64);
        assert!(!desktop.contains(&key));
        assert!(proof_matches(&desktop, &desktop_proof(&key, &np, &nd)));
        let other_key = crate::ipc_auth::generate_token().unwrap();
        for wrong in [
            desktop_proof(&other_key, &np, &nd),
            desktop_proof(&key, &nonce().unwrap(), &nd),
            desktop_proof(&key, &np, &nonce().unwrap()),
            desktop_proof(&key, &nd, &np),
            phone_proof(&key, &np, &nd),
            String::new(),
        ] {
            assert!(!proof_matches(&desktop, &wrong), "{wrong}");
        }
        let frame = phone_frame(&key, &np, &nd);
        assert!(frame_proves(&frame, &phone_proof(&key, &np, &nd)));
        assert!(!frame_proves(&frame, &desktop));
        assert!(!frame_proves("not json", &desktop));
        assert!(valid_nonce(&np) && !valid_nonce("abc") && !valid_nonce(&format!("{}g", &np[..63])));
    }
}
