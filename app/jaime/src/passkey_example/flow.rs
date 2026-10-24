//! The two ceremonies, from the server's challenge to its verdict.
//!
//! Each step is logged as it happens, so the page can show where a failure
//! occurred: reaching the server, the system sheet, or the server's checks.

use std::time::Duration;

use aimer::passkey::{
    Algorithm, CreateRequest, CredentialId, GetRequest, PasskeyCapability, PasskeyError,
    PlatformPasskeys, Passkeys, RelyingParty, User,
};

use super::protocol::{self, AssertionParts, RegistrationParts};

/// How long to wait for the server before giving up.
#[cfg(not(target_arch = "wasm32"))]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// What the page asks the flow to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    /// Reach the server and ask the device whether it has passkeys.
    Check,
    /// Create a passkey and register it.
    Register,
    /// Sign in with a passkey.
    SignIn,
}

/// The base URL of the passkey server, without a trailing slash.
#[derive(Debug, Eq, PartialEq)]
pub struct ServerUrl(String);

impl ServerUrl {
    /// Accepts `http://host[:port]` or `https://host[:port]`, tolerating
    /// surrounding whitespace and trailing slashes.
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim().trim_end_matches('/');
        let host = trimmed
            .strip_prefix("http://")
            .or_else(|| trimmed.strip_prefix("https://"))
            .ok_or("the server address must start with http:// or https://")?;
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err("the server address has no host".to_owned());
        }
        Ok(Self(trimmed.to_owned()))
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.0)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Runs `operation` and returns the line to show on success.
pub async fn run(
    operation: Operation,
    server: &str,
    username: &str,
    log: &dyn Fn(String),
) -> Result<String, String> {
    let server = ServerUrl::parse(server)?;
    match operation {
        Operation::Check => check(&server, log).await,
        Operation::Register => register(&server, username, log).await,
        Operation::SignIn => sign_in(&server, log).await,
    }
}

fn client() -> reqwest::Client {
    let builder = reqwest::Client::builder();
    #[cfg(not(target_arch = "wasm32"))]
    let builder = builder.timeout(REQUEST_TIMEOUT);
    builder.build().unwrap_or_default()
}

/// Sends a request and returns the body of a successful answer; any other
/// outcome becomes a message naming what went wrong.
async fn exchange(request: reqwest::RequestBuilder) -> Result<String, String> {
    let response = request
        .send()
        .await
        .map_err(|error| format!("cannot reach the server: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("the server's answer was cut off: {error}"))?;
    if status.is_success() {
        return Ok(body);
    }
    Err(match protocol::server_error(&body) {
        Some(message) => format!("the server refused it ({status}): {message}"),
        None => format!("the server answered {status}"),
    })
}

async fn post(server: &ServerUrl, path: &str, body: String) -> Result<String, String> {
    exchange(
        client()
            .post(server.endpoint(path))
            .header("content-type", "application/json")
            .body(body),
    )
    .await
}

async fn check(server: &ServerUrl, log: &dyn Fn(String)) -> Result<String, String> {
    log(format!("GET {}", server.endpoint("/health")));
    let body = exchange(client().get(server.endpoint("/health"))).await?;
    let health: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| "the server's health answer is not JSON")?;
    log(format!(
        "Server is up for relying party {}",
        health["rpId"].as_str().unwrap_or("?")
    ));

    let available = PlatformPasskeys::new()
        .is_available()
        .await
        .map_err(|error| explain(&error))?;
    if !available {
        return Err("this device cannot create or use passkeys".to_owned());
    }
    Ok("Server reachable and passkeys available on this device".to_owned())
}

async fn register(
    server: &ServerUrl,
    username: &str,
    log: &dyn Fn(String),
) -> Result<String, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("enter an account name first".to_owned());
    }

    log("Asking the server for a challenge…".to_owned());
    let body = post(server, "/register/options", protocol::register_options_request(username))
        .await?;
    let options = protocol::parse_register_options(&body)?;
    log(format!(
        "Challenge received for relying party {} (account {})",
        options.rp_id, options.user_name
    ));

    let mut request = CreateRequest::new(
        RelyingParty::new(options.rp_id.as_str(), options.rp_name.as_str()),
        User::new(options.user_id.as_slice(), options.user_name.as_str()),
        options.challenge.as_slice(),
    )
    // The server verifies ES256 only.
    .algorithms([Algorithm::Es256]);
    if !options.exclude.is_empty() {
        request = request.exclude(options.exclude.iter().cloned().map(CredentialId::new));
    }

    log("Waiting for the system passkey sheet…".to_owned());
    let registration = PlatformPasskeys::new()
        .create(request)
        .await
        .map_err(|error| explain(&error))?;

    log("Passkey created; sending it to the server to verify…".to_owned());
    let body = post(
        server,
        "/register/verify",
        protocol::register_verify_body(
            &options.challenge_text,
            &RegistrationParts {
                credential_id: registration.credential_id(),
                client_data_json: registration.client_data_json(),
                attestation_object: registration.attestation_object(),
            },
        ),
    )
    .await?;
    protocol::describe_registration(&body)
}

async fn sign_in(server: &ServerUrl, log: &dyn Fn(String)) -> Result<String, String> {
    log("Asking the server for a challenge…".to_owned());
    let body = post(server, "/login/options", "{}".to_owned()).await?;
    let options = protocol::parse_login_options(&body)?;
    log(format!("Challenge received for relying party {}", options.rp_id));

    log("Waiting for the system passkey sheet…".to_owned());
    let assertion = PlatformPasskeys::new()
        .get(GetRequest::new(options.rp_id.as_str(), options.challenge.as_slice()))
        .await
        .map_err(|error| explain(&error))?;

    log("Signed; sending the proof to the server to verify…".to_owned());
    let body = post(
        server,
        "/login/verify",
        protocol::login_verify_body(
            &options.challenge_text,
            &AssertionParts {
                credential_id: assertion.credential_id(),
                client_data_json: assertion.client_data_json(),
                authenticator_data: assertion.authenticator_data(),
                signature: assertion.signature(),
                user_handle: assertion.user_handle(),
            },
        ),
    )
    .await?;
    protocol::describe_login(&body)
}

/// A passkey error in words, plus what usually fixes the configuration ones.
pub fn explain(error: &PasskeyError) -> String {
    let hint = match error {
        PasskeyError::MissingEntitlement => {
            " Apple builds need the Associated Domains entitlement with \
             webcredentials:<relying party id> and a signed binary."
        }
        PasskeyError::NotAssociated => {
            " The relying party id must be a domain whose \
             apple-app-site-association / assetlinks.json lists this app; the server \
             serves both when started with --apple-app-id / --android-package."
        }
        PasskeyError::Unavailable {
            capability: PasskeyCapability::PlatformAuthenticator,
        } => " This target has no passkey provider (Linux and the browser).",
        PasskeyError::Unavailable {
            capability: PasskeyCapability::Bridge,
        } => " On Android, add PasskeyBridge.kt to the app and call PasskeyBridge.attach(this).",
        _ => "",
    };
    format!("{error}.{hint}")
}

#[cfg(test)]
mod tests {
    use aimer::console::SyncFuture;

    use super::*;

    #[test]
    fn server_urls_are_normalised() {
        for (input, expected) in [
            ("http://127.0.0.1:8787", "http://127.0.0.1:8787"),
            ("  http://localhost:8787/  ", "http://localhost:8787"),
            ("https://example.com//", "https://example.com"),
        ] {
            assert_eq!(ServerUrl::parse(input).unwrap().as_str(), expected, "{input}");
        }
    }

    #[test]
    fn server_urls_without_a_scheme_or_host_are_refused() {
        for input in ["", "localhost:8787", "ftp://x", "http://", "https:///", "http://a b"] {
            assert!(ServerUrl::parse(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn endpoints_join_without_doubled_slashes() {
        let server = ServerUrl::parse("http://h:1/").unwrap();

        assert_eq!(server.endpoint("/health"), "http://h:1/health");
    }

    #[test]
    fn configuration_errors_say_how_to_fix_them() {
        assert!(explain(&PasskeyError::MissingEntitlement).contains("Associated Domains"));
        assert!(explain(&PasskeyError::NotAssociated).contains("--apple-app-id"));
        assert!(explain(&PasskeyError::Unavailable {
            capability: PasskeyCapability::Bridge
        })
        .contains("PasskeyBridge"));
    }

    #[test]
    fn user_driven_errors_are_plain() {
        assert_eq!(
            explain(&PasskeyError::Cancelled),
            "the passkey request was cancelled."
        );
    }

    #[test]
    fn a_bad_server_address_fails_before_any_network_use() {
        let result = run(Operation::Check, "nope", "", &|_| {}).block();

        assert!(result.unwrap_err().contains("http://"));
    }

    #[test]
    fn registering_without_a_name_fails_before_any_network_use() {
        let result = run(Operation::Register, "http://127.0.0.1:1", "  ", &|_| {}).block();

        assert_eq!(result.unwrap_err(), "enter an account name first");
    }
}
