//! The server's memory: outstanding challenges and registered accounts.
//!
//! Everything is in process memory and gone on restart, which is what a test
//! server wants. A real relying party keeps the same facts in a database.

use std::collections::HashMap;
use aimer_time::{Duration, Instant};

use ring::rand::{SecureRandom, SystemRandom};

/// How long an issued challenge stays valid. Matches the five minutes the
/// client side allows a ceremony.
const CHALLENGE_TTL: Duration = Duration::from_secs(300);
/// Most challenges outstanding at once. Bounds memory against a client that
/// asks for options and never finishes.
const MAX_PENDING: usize = 1024;
const CHALLENGE_BYTES: usize = 32;
const USER_ID_BYTES: usize = 16;

/// One passkey of an account.
pub struct Credential {
    pub id: Vec<u8>,
    /// Uncompressed SEC1 P-256 point.
    pub public_key: Vec<u8>,
    pub sign_count: u32,
}

/// An account and its passkeys.
pub struct User {
    /// The opaque handle the platform stores beside the passkey.
    pub id: Vec<u8>,
    pub name: String,
    pub credentials: Vec<Credential>,
}

/// What a challenge was issued for, so one ceremony's challenge cannot finish
/// the other.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Purpose {
    Register { username: String, user_id: Vec<u8> },
    Login,
}

struct Pending {
    purpose: Purpose,
    issued: Instant,
}

/// Why a challenge could not be issued.
#[derive(Debug, Eq, PartialEq)]
pub enum IssueError {
    /// Too many unfinished ceremonies.
    Busy,
    /// The system random source failed.
    Random,
}

#[derive(Default)]
pub struct Store {
    pending: HashMap<String, Pending>,
    users: Vec<User>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issues a fresh single-use challenge for `purpose` and returns it as
    /// base64url.
    pub fn issue(
        &mut self,
        purpose: Purpose,
        now: Instant,
        rng: &SystemRandom,
    ) -> Result<String, IssueError> {
        self.pending
            .retain(|_, p| now.duration_since(p.issued) < CHALLENGE_TTL);
        if self.pending.len() >= MAX_PENDING {
            return Err(IssueError::Busy);
        }
        let mut bytes = [0u8; CHALLENGE_BYTES];
        rng.fill(&mut bytes).map_err(|_| IssueError::Random)?;
        let challenge = crate::base64url::encode(&bytes);
        self.pending.insert(
            challenge.clone(),
            Pending {
                purpose,
                issued: now,
            },
        );
        Ok(challenge)
    }

    /// Consumes `challenge`. A challenge is spent whether or not the ceremony
    /// that presents it then succeeds, so a failed attempt cannot be replayed
    /// with tweaked fields.
    pub fn take(&mut self, challenge: &str, now: Instant) -> Option<Purpose> {
        let pending = self.pending.remove(challenge)?;
        (now.duration_since(pending.issued) < CHALLENGE_TTL).then_some(pending.purpose)
    }

    /// A fresh random user handle.
    pub fn new_user_id(rng: &SystemRandom) -> Result<Vec<u8>, IssueError> {
        let mut id = vec![0u8; USER_ID_BYTES];
        rng.fill(&mut id).map_err(|_| IssueError::Random)?;
        Ok(id)
    }

    pub fn user_by_name(&self, name: &str) -> Option<&User> {
        self.users.iter().find(|u| u.name == name)
    }

    pub fn users(&self) -> &[User] {
        &self.users
    }

    /// The account owning `credential_id` and the credential itself.
    pub fn find_credential(&self, credential_id: &[u8]) -> Option<(&User, &Credential)> {
        self.users.iter().find_map(|user| {
            user.credentials
                .iter()
                .find(|c| c.id == credential_id)
                .map(|credential| (user, credential))
        })
    }

    /// Records the counter an assertion proved.
    pub fn set_sign_count(&mut self, credential_id: &[u8], sign_count: u32) {
        if let Some(credential) = self
            .users
            .iter_mut()
            .flat_map(|u| u.credentials.iter_mut())
            .find(|c| c.id == credential_id)
        {
            credential.sign_count = sign_count;
        }
    }

    /// Adds `credential` to the account named `username`, creating the account
    /// with `user_id` if it does not exist yet. Returns how many passkeys the
    /// account has now, or `None` if the credential id is already registered.
    pub fn add_credential(
        &mut self,
        username: &str,
        user_id: Vec<u8>,
        credential: Credential,
    ) -> Option<usize> {
        if self.find_credential(&credential.id).is_some() {
            return None;
        }
        let index = match self.users.iter().position(|u| u.name == username) {
            Some(index) => index,
            None => {
                self.users.push(User {
                    id: user_id,
                    name: username.to_owned(),
                    credentials: Vec::new(),
                });
                self.users.len() - 1
            }
        };
        let user = &mut self.users[index];
        user.credentials.push(credential);
        Some(user.credentials.len())
    }

    /// Forgets every account and challenge.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.users.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential(id: u8) -> Credential {
        Credential {
            id: vec![id; 16],
            public_key: vec![4; 65],
            sign_count: 0,
        }
    }

    #[test]
    fn a_challenge_is_single_use() {
        let (mut store, rng, now) = (Store::new(), SystemRandom::new(), Instant::now());
        let challenge = store.issue(Purpose::Login, now, &rng).unwrap();

        assert_eq!(store.take(&challenge, now), Some(Purpose::Login));
        assert_eq!(store.take(&challenge, now), None);
    }

    #[test]
    fn challenges_are_random_and_unknown_ones_are_refused() {
        let (mut store, rng, now) = (Store::new(), SystemRandom::new(), Instant::now());
        let a = store.issue(Purpose::Login, now, &rng).unwrap();
        let b = store.issue(Purpose::Login, now, &rng).unwrap();

        assert_ne!(a, b);
        assert_eq!(crate::base64url::decode(&a).unwrap().len(), CHALLENGE_BYTES);
        assert_eq!(store.take("bm90LWlzc3VlZA", now), None);
    }

    #[test]
    fn an_expired_challenge_is_refused_and_pruned() {
        let (mut store, rng, now) = (Store::new(), SystemRandom::new(), Instant::now());
        let challenge = store.issue(Purpose::Login, now, &rng).unwrap();
        let later = now + CHALLENGE_TTL;

        assert_eq!(store.take(&challenge, later), None);
        let fresh = store.issue(Purpose::Login, later, &rng).unwrap();
        assert!(store.take(&fresh, later).is_some());
    }

    #[test]
    fn unfinished_ceremonies_are_capped() {
        let (mut store, rng, now) = (Store::new(), SystemRandom::new(), Instant::now());
        for _ in 0..MAX_PENDING {
            store.issue(Purpose::Login, now, &rng).unwrap();
        }

        assert_eq!(store.issue(Purpose::Login, now, &rng), Err(IssueError::Busy));
        // Expiry frees the slots again.
        assert!(store
            .issue(Purpose::Login, now + CHALLENGE_TTL, &rng)
            .is_ok());
    }

    #[test]
    fn credentials_attach_to_one_account_and_ids_stay_unique() {
        let mut store = Store::new();

        assert_eq!(store.add_credential("alice", vec![1; 16], credential(1)), Some(1));
        assert_eq!(store.add_credential("alice", vec![9; 16], credential(2)), Some(2));
        assert_eq!(store.add_credential("bob", vec![2; 16], credential(1)), None);
        assert_eq!(store.users().len(), 1);
        // The account keeps the handle it was created with.
        assert_eq!(store.user_by_name("alice").unwrap().id, vec![1; 16]);
        let (user, found) = store.find_credential(&[2; 16]).unwrap();
        assert_eq!((user.name.as_str(), found.sign_count), ("alice", 0));
    }

    #[test]
    fn the_sign_count_is_updated_in_place() {
        let mut store = Store::new();
        store.add_credential("alice", vec![1; 16], credential(1));

        store.set_sign_count(&[1; 16], 7);

        assert_eq!(store.find_credential(&[1; 16]).unwrap().1.sign_count, 7);
    }
}
