//! The relying party's endpoints, independent of any socket.
//!
//! [`Server::handle`] maps a parsed request to a response and touches nothing
//! else, so the whole protocol is testable without a network.

use std::sync::{Mutex, PoisonError};
use aimer_time::Instant;

use ring::rand::SystemRandom;
use serde_json::{Value as Json, json};

use crate::base64url;
use crate::store::{Credential, IssueError, Purpose, Store};
use crate::verify::{self, Ceremony};

/// Longest account name accepted.
const MAX_USERNAME_CHARS: usize = 64;

/// What identifies this relying party to the world.
pub struct Config {
    /// The domain passkeys are bound to.
    pub rp_id: String,
    /// The name platforms show beside the passkey.
    pub rp_name: String,
    /// `TEAMID.bundle.id` entries for `apple-app-site-association`.
    pub apple_app_ids: Vec<String>,
    /// The Android app `assetlinks.json` vouches for, if any.
    pub android: Option<AndroidApp>,
}

/// An Android app as `assetlinks.json` names it.
pub struct AndroidApp {
    pub package: String,
    /// SHA-256 fingerprints of the signing certificate, colon-separated hex.
    pub fingerprints: Vec<String>,
}

/// A parsed HTTP request.
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

/// An HTTP response ready to serialize.
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    fn json(status: u16, value: Json) -> Self {
        Self {
            status,
            body: value.to_string().into_bytes(),
        }
    }

    fn error(status: u16, message: impl std::fmt::Display) -> Self {
        Self::json(status, json!({ "error": message.to_string() }))
    }
}

pub struct Server {
    config: Config,
    store: Mutex<Store>,
    rng: SystemRandom,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            store: Mutex::new(Store::new()),
            rng: SystemRandom::new(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn handle(&self, request: &Request) -> Response {
        self.handle_at(request, Instant::now())
    }

    /// [`Server::handle`] with the clock supplied, so expiry is testable.
    pub fn handle_at(&self, request: &Request, now: Instant) -> Response {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/health") => Response::json(
                200,
                json!({ "ok": true, "rpId": self.config.rp_id, "rpName": self.config.rp_name }),
            ),
            ("GET", "/users") => self.users(),
            ("POST", "/reset") => {
                self.store().reset();
                Response::json(200, json!({ "ok": true }))
            }
            ("GET", "/.well-known/apple-app-site-association") => self.apple_association(),
            ("GET", "/.well-known/assetlinks.json") => self.asset_links(),
            ("POST", "/register/options") => self.register_options(&request.body, now),
            ("POST", "/register/verify") => self.register_verify(&request.body, now),
            ("POST", "/login/options") => self.login_options(now),
            ("POST", "/login/verify") => self.login_verify(&request.body, now),
            (
                _,
                "/health" | "/users" | "/reset" | "/register/options" | "/register/verify"
                | "/login/options" | "/login/verify",
            ) => Response::error(405, "method not allowed"),
            _ => Response::error(404, "not found"),
        }
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        // A panic mid-request leaves the maps consistent (each operation is a
        // single insert or remove), so a poisoned lock is safe to reuse.
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn users(&self) -> Response {
        let store = self.store();
        let users: Vec<Json> = store
            .users()
            .iter()
            .map(|u| json!({ "name": u.name, "passkeys": u.credentials.len() }))
            .collect();
        Response::json(200, json!({ "users": users }))
    }

    fn apple_association(&self) -> Response {
        if self.config.apple_app_ids.is_empty() {
            return Response::error(404, "no --apple-app-id configured");
        }
        Response::json(
            200,
            json!({ "webcredentials": { "apps": self.config.apple_app_ids } }),
        )
    }

    fn asset_links(&self) -> Response {
        let Some(app) = &self.config.android else {
            return Response::error(404, "no --android-package configured");
        };
        Response::json(
            200,
            json!([{
                "relation": ["delegate_permission/common.get_login_creds"],
                "target": {
                    "namespace": "android_app",
                    "package_name": app.package,
                    "sha256_cert_fingerprints": app.fingerprints,
                },
            }]),
        )
    }

    fn register_options(&self, body: &[u8], now: Instant) -> Response {
        let json = match parse_object(body) {
            Ok(json) => json,
            Err(response) => return response,
        };
        let Some(username) = json.get("username").and_then(Json::as_str) else {
            return Response::error(400, "missing username");
        };
        let username = username.trim();
        if username.is_empty()
            || username.chars().count() > MAX_USERNAME_CHARS
            || username.chars().any(char::is_control)
        {
            return Response::error(400, "username must be 1-64 printable characters");
        }

        let mut store = self.store();
        // An existing account keeps its handle and lists the passkeys it has,
        // so the platform refuses to enroll the same authenticator twice.
        let (user_id, exclude) = match store.user_by_name(username) {
            Some(user) => (
                user.id.clone(),
                user.credentials
                    .iter()
                    .map(|c| base64url::encode(&c.id))
                    .collect(),
            ),
            None => match Store::new_user_id(&self.rng) {
                Ok(id) => (id, Vec::<String>::new()),
                Err(error) => return issue_failure(error),
            },
        };
        let purpose = Purpose::Register {
            username: username.to_owned(),
            user_id: user_id.clone(),
        };
        let challenge = match store.issue(purpose, now, &self.rng) {
            Ok(challenge) => challenge,
            Err(error) => return issue_failure(error),
        };
        Response::json(
            200,
            json!({
                "challenge": challenge,
                "rpId": self.config.rp_id,
                "rpName": self.config.rp_name,
                "userId": base64url::encode(&user_id),
                "userName": username,
                "excludeCredentials": exclude,
            }),
        )
    }

    fn register_verify(&self, body: &[u8], now: Instant) -> Response {
        let json = match parse_object(body) {
            Ok(json) => json,
            Err(response) => return response,
        };
        let fields = (
            text(&json, "challenge"),
            bytes(&json, "credentialId"),
            bytes(&json, "clientDataJSON"),
            bytes(&json, "attestationObject"),
        );
        let (Some(challenge), Some(credential_id), Some(client_data), Some(attestation)) = fields
        else {
            return Response::error(400, "missing or malformed field");
        };

        let mut store = self.store();
        let Some(Purpose::Register { username, user_id }) = store.take(challenge, now) else {
            return Response::error(400, "unknown, expired or reused challenge");
        };
        let rp_id = &self.config.rp_id;
        let checked = verify::check_client_data(&client_data, Ceremony::Create, challenge, rp_id)
            .and_then(|()| verify::verify_registration(&attestation, &credential_id, rp_id));
        let new = match checked {
            Ok(new) => new,
            Err(error) => return Response::error(400, error),
        };
        let credential = Credential {
            id: credential_id,
            public_key: new.public_key,
            sign_count: new.sign_count,
        };
        let id_text = base64url::encode(&credential.id);
        match store.add_credential(&username, user_id, credential) {
            Some(passkeys) => Response::json(
                200,
                json!({
                    "ok": true,
                    "username": username,
                    "credentialId": id_text,
                    "passkeys": passkeys,
                }),
            ),
            None => Response::error(409, "that passkey is already registered"),
        }
    }

    fn login_options(&self, now: Instant) -> Response {
        match self.store().issue(Purpose::Login, now, &self.rng) {
            // No allow list: passkeys are discoverable, so the platform lists
            // the ones it holds for this relying party.
            Ok(challenge) => Response::json(
                200,
                json!({
                    "challenge": challenge,
                    "rpId": self.config.rp_id,
                    "allowCredentials": [],
                }),
            ),
            Err(error) => issue_failure(error),
        }
    }

    fn login_verify(&self, body: &[u8], now: Instant) -> Response {
        let json = match parse_object(body) {
            Ok(json) => json,
            Err(response) => return response,
        };
        let fields = (
            text(&json, "challenge"),
            bytes(&json, "credentialId"),
            bytes(&json, "clientDataJSON"),
            bytes(&json, "authenticatorData"),
            bytes(&json, "signature"),
        );
        let (
            Some(challenge),
            Some(credential_id),
            Some(client_data),
            Some(authenticator_data),
            Some(signature),
        ) = fields
        else {
            return Response::error(400, "missing or malformed field");
        };
        // The user handle is only present when the platform returns one.
        let user_handle = match json.get("userHandle") {
            None | Some(Json::Null) => None,
            Some(_) => match bytes(&json, "userHandle") {
                Some(handle) => Some(handle),
                None => return Response::error(400, "malformed userHandle"),
            },
        };

        let mut store = self.store();
        if !matches!(store.take(challenge, now), Some(Purpose::Login)) {
            return Response::error(400, "unknown, expired or reused challenge");
        }
        let rp_id = &self.config.rp_id;
        if let Err(error) = verify::check_client_data(&client_data, Ceremony::Get, challenge, rp_id)
        {
            return Response::error(400, error);
        }
        let Some((user, credential)) = store.find_credential(&credential_id) else {
            return Response::error(401, "unknown passkey");
        };
        if user_handle.is_some_and(|handle| handle != user.id) {
            return Response::error(401, "the user handle does not belong to that passkey");
        }
        let username = user.name.clone();
        let sign_count = match verify::verify_assertion(
            &authenticator_data,
            &client_data,
            &signature,
            &credential.public_key,
            credential.sign_count,
            rp_id,
        ) {
            Ok(count) => count,
            Err(error) => return Response::error(401, error),
        };
        store.set_sign_count(&credential_id, sign_count);
        Response::json(
            200,
            json!({ "ok": true, "username": username, "signCount": sign_count }),
        )
    }
}

fn parse_object(body: &[u8]) -> Result<Json, Response> {
    match serde_json::from_slice::<Json>(body) {
        Ok(json) if json.is_object() => Ok(json),
        _ => Err(Response::error(400, "the body must be a JSON object")),
    }
}

fn text<'a>(json: &'a Json, key: &str) -> Option<&'a str> {
    json.get(key).and_then(Json::as_str)
}

/// A base64url field, decoded.
fn bytes(json: &Json, key: &str) -> Option<Vec<u8>> {
    base64url::decode(text(json, key)?)
}

fn issue_failure(error: IssueError) -> Response {
    match error {
        IssueError::Busy => Response::error(503, "too many unfinished ceremonies"),
        IssueError::Random => Response::error(500, "no randomness available"),
    }
}
