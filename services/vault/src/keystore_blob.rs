//! The bytes written to disk for one secret.
// Only the Android adapter uses this, but it is plain logic, so it is compiled and
// tested on every target.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]


use crate::access::VaultAccess;

/// Bumped if the layout ever changes, so an old file is recognised, not misread.
const VERSION: u8 = 1;

/// One encrypted secret: which key sealed it, the nonce, and the GCM output
/// (ciphertext followed by its authentication tag).
///
/// Layout: `version, access, iv_len, iv.., ciphertext..`.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Blob {
    pub(crate) access: VaultAccess,
    pub(crate) iv: Vec<u8>,
    pub(crate) ciphertext: Vec<u8>,
}

impl Blob {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(3 + self.iv.len() + self.ciphertext.len());
        bytes.push(VERSION);
        bytes.push(match self.access {
            VaultAccess::WhenUnlocked => 0,
            VaultAccess::AfterFirstUnlock => 1,
        });
        // GCM nonces are 12 bytes; the length byte only has to hold what the
        // Keystore produced.
        bytes.push(self.iv.len() as u8);
        bytes.extend_from_slice(&self.iv);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// `None` for anything that is not a well-formed blob of this version.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let (&version, rest) = bytes.split_first()?;
        if version != VERSION {
            return None;
        }
        let (&access, rest) = rest.split_first()?;
        let access = match access {
            0 => VaultAccess::WhenUnlocked,
            1 => VaultAccess::AfterFirstUnlock,
            _ => return None,
        };
        let (&iv_len, rest) = rest.split_first()?;
        let iv_len = usize::from(iv_len);
        if iv_len == 0 || rest.len() <= iv_len {
            return None;
        }
        let (iv, ciphertext) = rest.split_at(iv_len);
        Some(Self {
            access,
            iv: iv.to_vec(),
            ciphertext: ciphertext.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob() -> Blob {
        Blob {
            access: VaultAccess::AfterFirstUnlock,
            iv: vec![7; 12],
            ciphertext: vec![9; 20],
        }
    }

    #[test]
    fn a_blob_round_trips() {
        assert_eq!(Blob::decode(&blob().encode()), Some(blob()));
        let unlocked = Blob {
            access: VaultAccess::WhenUnlocked,
            ..blob()
        };
        assert_eq!(Blob::decode(&unlocked.encode()), Some(unlocked));
    }

    #[test]
    fn malformed_blobs_are_rejected() {
        let good = blob().encode();
        assert_eq!(Blob::decode(&[]), None);
        assert_eq!(Blob::decode(&good[..2]), None);
        // Truncated inside the iv, and with nothing after the iv.
        assert_eq!(Blob::decode(&good[..8]), None);
        assert_eq!(Blob::decode(&good[..15]), None);

        let mut wrong_version = good.clone();
        wrong_version[0] = 2;
        assert_eq!(Blob::decode(&wrong_version), None);

        let mut wrong_access = good.clone();
        wrong_access[1] = 9;
        assert_eq!(Blob::decode(&wrong_access), None);

        let mut no_iv = good;
        no_iv[2] = 0;
        assert_eq!(Blob::decode(&no_iv), None);
    }
}
