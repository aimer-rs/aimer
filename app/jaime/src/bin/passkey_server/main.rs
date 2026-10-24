//! A local relying party for trying `aimer_passkey` end to end.
//!
//! It issues challenges, then verifies the registrations and assertions the
//! Jaime "Passkeys" page sends back: client data, relying party hash, user
//! presence, and the ES256 signature. Accounts live in memory.
//!
//! ```bash
//! cargo run -p jaime --features passkey-server --bin passkey_server
//! cargo run -p jaime --features passkey-server --bin passkey_server -- \
//!     --rp-id example.com --apple-app-id TEAMID.com.example.jaime
//! ```
//!
//! The server speaks plain HTTP and has no authentication of its own: bind it
//! to loopback unless a device on your network has to reach it.

#![allow(dead_code)]

mod app;
#[path = "../../passkey_example/base64url.rs"]
mod base64url;
mod cbor;
mod http;
mod store;
mod verify;

#[cfg(test)]
mod authenticator;
#[cfg(test)]
mod tests;

use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Arc;

use app::{AndroidApp, Config, Server};

const USAGE: &str = "\
usage: passkey_server [options]

  --bind ADDR                 listen address (default 127.0.0.1:8787)
  --rp-id DOMAIN              relying party id passkeys are bound to (default localhost)
  --rp-name NAME              name shown beside the passkey (default Jaime)
  --apple-app-id TEAM.BUNDLE  serve it in apple-app-site-association (repeatable)
  --android-package PACKAGE   serve it in assetlinks.json
  --android-fingerprint SHA256  signing certificate fingerprint (repeatable)
";

struct Options {
    bind: String,
    config: Config,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut bind = "127.0.0.1:8787".to_owned();
    let mut rp_id = "localhost".to_owned();
    let mut rp_name = "Jaime".to_owned();
    let mut apple_app_ids = Vec::new();
    let mut package = None;
    let mut fingerprints = Vec::new();

    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = |what: &str| {
            args.next()
                .ok_or_else(|| format!("{flag} needs {what}"))
        };
        match flag.as_str() {
            "--bind" => bind = value("an address")?,
            "--rp-id" => rp_id = value("a domain")?,
            "--rp-name" => rp_name = value("a name")?,
            "--apple-app-id" => apple_app_ids.push(value("TEAMID.bundle.id")?),
            "--android-package" => package = Some(value("a package name")?),
            "--android-fingerprint" => fingerprints.push(value("a fingerprint")?),
            other => return Err(format!("unknown option {other}")),
        }
    }
    if rp_id.is_empty()
        || !rp_id.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
    {
        return Err(format!("{rp_id:?} is not a lowercase domain name"));
    }
    if package.is_none() && !fingerprints.is_empty() {
        return Err("--android-fingerprint needs --android-package".to_owned());
    }
    Ok(Options {
        bind,
        config: Config {
            rp_id,
            rp_name,
            apple_app_ids,
            android: package.map(|package| AndroidApp {
                package,
                fingerprints,
            }),
        },
    })
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let listener = match TcpListener::bind(&options.bind) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("cannot listen on {}: {error}", options.bind);
            return ExitCode::FAILURE;
        }
    };
    println!(
        "passkey server for {:?} listening on http://{}",
        options.config.rp_id,
        listener
            .local_addr()
            .map_or(options.bind.clone(), |addr| addr.to_string())
    );
    http::serve(listener, Arc::new(Server::new(options.config)));
    ExitCode::SUCCESS
}
