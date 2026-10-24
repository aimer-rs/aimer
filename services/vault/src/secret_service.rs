//! The Secret Service adapter for Linux (GNOME Keyring, KWallet, and any other
//! implementation of the freedesktop.org Secret Service API).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zeroize::Zeroize;

use crate::access::VaultAccess;
use crate::error::{VaultCapability, VaultError, VaultResult};
use crate::key::{check_secret, validate, validate_component};
use crate::lane::Lane;
use crate::secret::SecretBytes;
use crate::vault::{Vault, VaultFuture, ready};

const BUS_NAME: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
/// The Secret Service's "no object" value, returned where no prompt is needed.
const NO_OBJECT: &str = "/";

/// Item attributes. `SCHEMA` marks everything this crate writes, and the next
/// three identify one secret.
const SCHEMA_ATTRIBUTE: &str = "xdg:schema";
const SCHEMA: &str = "dev.aimer.vault";
const APP_ATTRIBUTE: &str = "aimer-app";
const NAMESPACE_ATTRIBUTE: &str = "aimer-namespace";
const KEY_ATTRIBUTE: &str = "aimer-key";

const LABEL_PROPERTY: &str = "org.freedesktop.Secret.Item.Label";
const ATTRIBUTES_PROPERTY: &str = "org.freedesktop.Secret.Item.Attributes";

/// A Secret Service secret: `(session, parameters, value, content_type)`, the
/// `(oayays)` structure of the specification.
type SecretStruct = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// A vault backed by the Secret Service.
///
/// Each secret is an item in the user's *default* collection (the login keyring
/// on GNOME, the default wallet on KDE), identified by attributes: this
/// crate's schema, the application id, the namespace, and the key. Two
/// applications never see each other's secrets, and clearing a namespace
/// touches only this application's.
///
/// When the keyring is locked, the service asks the user to unlock it with its
/// own dialog, and the operation waits for the answer; declining resolves to
/// [`VaultError::Denied`]. With no Secret Service on the session bus (a minimal
/// or headless system), every operation resolves to
/// [`VaultError::Unavailable`] with
/// [`VaultCapability::PlatformSecretStore`].
///
/// # Transport
///
/// Secrets cross the user's session bus using the Secret Service's `plain`
/// session algorithm, that is, unencrypted on the bus. The session bus is
/// private to the user and authenticated by user id, so the processes that can
/// observe the traffic are those that could already ask the unlocked keyring
/// for the same secrets.
///
/// Calls run on Venus's offload pool, in call order, and a Venus runtime must be
/// installed on the calling thread: an operation made without one resolves to
/// [`VaultError::Unavailable`] with
/// [`VaultCapability::Runtime`](crate::VaultCapability::Runtime).
///
/// Clones share one ordering queue and one bus connection.
#[derive(Clone)]
pub struct SecretServiceVault {
    app_id: Arc<str>,
    access: VaultAccess,
    lane: Lane,
    link: Arc<Mutex<Option<Link>>>,
}

impl SecretServiceVault {
    /// Opens the vault an application keeps its secrets in.
    ///
    /// The same `app_id` finds the same secrets on every launch, so use a stable
    /// reverse-DNS name such as `com.example.app`. Nothing is contacted until
    /// the first operation.
    ///
    /// # Errors
    ///
    /// [`VaultError::InvalidKey`] when `app_id` is not a valid name.
    pub fn for_app(app_id: &str) -> VaultResult<Self> {
        validate_component(app_id)?;
        Ok(Self {
            app_id: Arc::from(app_id),
            access: VaultAccess::default(),
            lane: Lane::default(),
            link: Arc::default(),
        })
    }

    /// Records the access level asked for. See [`VaultAccess`].
    ///
    /// The Secret Service has no accessibility classes (a keyring is locked or
    /// unlocked as a whole), so this changes nothing here; it exists so code
    /// written for every target compiles unchanged.
    #[inline]
    pub fn with_access(mut self, access: VaultAccess) -> Self {
        self.access = access;
        self
    }

    /// Returns the access level that was asked for.
    #[inline]
    pub fn access(&self) -> VaultAccess {
        self.access
    }

    /// The attributes that identify one secret.
    fn attributes(&self, namespace: &str, key: &str) -> Attributes {
        let mut attributes = self.namespace_attributes(namespace);
        attributes.insert(KEY_ATTRIBUTE.to_owned(), key.to_owned());
        attributes
    }

    /// The attributes shared by every secret of one namespace.
    fn namespace_attributes(&self, namespace: &str) -> Attributes {
        HashMap::from([
            (SCHEMA_ATTRIBUTE.to_owned(), SCHEMA.to_owned()),
            (APP_ATTRIBUTE.to_owned(), self.app_id.to_string()),
            (NAMESPACE_ATTRIBUTE.to_owned(), namespace.to_owned()),
        ])
    }
}

type Attributes = HashMap<String, String>;

impl Vault for SecretServiceVault {
    fn read(&self, namespace: &str, key: &str) -> VaultFuture<Option<SecretBytes>> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let (attributes, link) = (self.attributes(namespace, key), Arc::clone(&self.link));
        self.lane
            .submit(move || with_link(&link, |link| read_item(link, &attributes)))
    }

    fn write(&self, namespace: &str, key: &str, secret: &[u8]) -> VaultFuture<()> {
        if let Err(error) = validate(namespace, key).and_then(|()| check_secret(secret.len())) {
            return ready(Err(error));
        }
        let label = format!("Aimer: {}/{namespace}/{key}", self.app_id);
        let (attributes, link) = (self.attributes(namespace, key), Arc::clone(&self.link));
        // The worker needs an owned copy; it is wiped when the job finishes.
        let secret = SecretBytes::from_slice(secret);
        self.lane.submit(move || {
            with_link(&link, |link| {
                write_item(link, &label, &attributes, secret.expose())
            })
        })
    }

    fn remove(&self, namespace: &str, key: &str) -> VaultFuture<bool> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let (attributes, link) = (self.attributes(namespace, key), Arc::clone(&self.link));
        self.lane.submit(move || {
            with_link(&link, |link| {
                delete_matching(link, &attributes).map(|deleted| deleted > 0)
            })
        })
    }

    fn clear(&self, namespace: &str) -> VaultFuture<()> {
        if let Err(error) = validate_component(namespace) {
            return ready(Err(error));
        }
        let (attributes, link) = (self.namespace_attributes(namespace), Arc::clone(&self.link));
        self.lane.submit(move || {
            with_link(&link, |link| delete_matching(link, &attributes).map(drop))
        })
    }
}

/// A connection to the session bus and the Secret Service session opened on it.
struct Link {
    connection: Connection,
    session: OwnedObjectPath,
}

/// Why a step failed: either for good, or because the cached link went stale
/// (the service restarted or the bus dropped us) and one retry on a fresh link
/// may succeed.
enum Failure {
    Stale,
    Vault(VaultError),
}

impl From<VaultError> for Failure {
    fn from(error: VaultError) -> Self {
        Self::Vault(error)
    }
}

impl From<zbus::Error> for Failure {
    fn from(error: zbus::Error) -> Self {
        classify(&error)
    }
}

type Step<T> = Result<T, Failure>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A job that panicked left the link in whatever state it had; it is either
    // usable or fails with `Stale` and is replaced.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `step` on the cached link, opening one first if needed, and once more on
/// a fresh link if the cached one turned out to be stale.
fn with_link<T>(link: &Mutex<Option<Link>>, step: impl Fn(&Link) -> Step<T>) -> VaultResult<T> {
    let mut cached = lock(link);
    for attempt in 0..2 {
        let current = match cached.as_ref() {
            Some(current) => current,
            None => cached.insert(connect().map_err(into_vault_error)?),
        };
        match step(current) {
            Ok(value) => return Ok(value),
            Err(Failure::Stale) if attempt == 0 => *cached = None,
            Err(failure) => return Err(into_vault_error(failure)),
        }
    }
    unreachable!("the second attempt always returns")
}

fn into_vault_error(failure: Failure) -> VaultError {
    match failure {
        Failure::Stale => VaultError::Failed,
        Failure::Vault(error) => error,
    }
}

fn connect() -> Step<Link> {
    // No session bus at all (no `DBUS_SESSION_BUS_ADDRESS`, a service container)
    // means no secret store, not a failure worth retrying.
    let connection = Connection::session().map_err(|_| unavailable())?;
    let session = {
        let service = service_proxy(&connection)?;
        let (_output, session): (OwnedValue, OwnedObjectPath) =
            service.call("OpenSession", &("plain", Value::from("")))?;
        session
    };
    Ok(Link {
        connection,
        session,
    })
}

fn unavailable() -> Failure {
    Failure::Vault(VaultError::Unavailable {
        capability: VaultCapability::PlatformSecretStore,
    })
}

fn proxy<'a>(link: &'a Connection, path: &'a str, interface: &'a str) -> Step<Proxy<'a>> {
    Ok(Proxy::new(link, BUS_NAME, path, interface)?)
}

fn service_proxy(connection: &Connection) -> Step<Proxy<'_>> {
    proxy(connection, SERVICE_PATH, SERVICE_INTERFACE)
}

fn read_item(link: &Link, attributes: &Attributes) -> Step<Option<SecretBytes>> {
    let items = search(link, attributes)?;
    let Some(item) = items.first() else {
        return Ok(None);
    };
    let item = proxy(&link.connection, item.as_str(), ITEM_INTERFACE)?;
    let (_session, _parameters, value, _content_type): SecretStruct =
        item.call("GetSecret", &(&link.session,))?;
    Ok(Some(SecretBytes::new(value)))
}

fn write_item(
    link: &Link,
    label: &str,
    attributes: &Attributes,
    secret: &[u8],
) -> Step<()> {
    let collection_path = default_collection(link)?;
    unlock(link, std::slice::from_ref(&collection_path))?;

    let properties: HashMap<&str, Value<'_>> = HashMap::from([
        (LABEL_PROPERTY, Value::from(label)),
        (ATTRIBUTES_PROPERTY, Value::new(attributes.clone())),
    ]);
    let mut secret: SecretStruct = (
        link.session.clone(),
        Vec::new(),
        secret.to_vec(),
        "application/octet-stream".to_owned(),
    );
    let collection = proxy(&link.connection, collection_path.as_str(), COLLECTION_INTERFACE)?;
    // `replace = true` makes an item with the same attributes be overwritten
    // rather than duplicated, which is what `write` promises.
    let created: Result<(OwnedObjectPath, OwnedObjectPath), zbus::Error> =
        collection.call("CreateItem", &(properties, &secret, true));
    // Wipe the copy that was handed to the bus, whether or not the call worked.
    secret.2.zeroize();
    let (_item, prompt) = match created {
        Ok(created) => created,
        // The alias names a collection that is not there: a keyring that was
        // never created, so there is nowhere to keep the secret.
        Err(zbus::Error::MethodError(name, ..)) if is_missing_object(name.as_str()) => {
            return Err(unavailable());
        }
        Err(error) => return Err(error.into()),
    };
    run_prompt(link, &prompt)
}

/// The collection the `default` alias names, where new secrets are kept.
fn default_collection(link: &Link) -> Step<OwnedObjectPath> {
    let service = service_proxy(&link.connection)?;
    let collection: OwnedObjectPath = service.call("ReadAlias", &("default",))?;
    if collection.as_str() == NO_OBJECT {
        // No default keyring has been set up.
        return Err(unavailable());
    }
    Ok(collection)
}

/// Whether a bus error name says the object called does not exist.
fn is_missing_object(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.UnknownMethod"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.Secret.Error.NoSuchObject"
    )
}

/// Deletes every item matching `attributes` and returns how many there were.
fn delete_matching(link: &Link, attributes: &Attributes) -> Step<usize> {
    let items = search(link, attributes)?;
    for item in &items {
        let proxy = proxy(&link.connection, item.as_str(), ITEM_INTERFACE)?;
        let prompt: OwnedObjectPath = proxy.call("Delete", &())?;
        run_prompt(link, &prompt)?;
    }
    Ok(items.len())
}

/// The items matching `attributes`, unlocked.
fn search(link: &Link, attributes: &Attributes) -> Step<Vec<OwnedObjectPath>> {
    let service = service_proxy(&link.connection)?;
    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
        service.call("SearchItems", &(attributes,))?;
    unlock(link, &locked)?;
    Ok(unlocked.into_iter().chain(locked).collect())
}

/// Unlocks `objects`, prompting the user if the service needs to.
fn unlock(link: &Link, objects: &[OwnedObjectPath]) -> Step<()> {
    if objects.is_empty() {
        return Ok(());
    }
    let service = service_proxy(&link.connection)?;
    let (_unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        service.call("Unlock", &(objects.to_vec(),))?;
    run_prompt(link, &prompt)
}

/// Shows the service's prompt, if there is one, and waits for the user's answer.
fn run_prompt(link: &Link, prompt: &OwnedObjectPath) -> Step<()> {
    if prompt.as_str() == NO_OBJECT {
        return Ok(());
    }
    let prompt = proxy(&link.connection, prompt.as_str(), PROMPT_INTERFACE)?;
    // Subscribe before asking, or a fast answer could be missed.
    let mut completed = prompt.receive_signal("Completed")?;
    prompt.call::<_, _, ()>("Prompt", &("",))?;
    let message = completed.next().ok_or(Failure::Stale)?;
    let (dismissed, _result): (bool, OwnedValue) = message
        .body()
        .deserialize()
        .map_err(|_| Failure::Vault(VaultError::Failed))?;
    if dismissed {
        Err(VaultError::Denied.into())
    } else {
        Ok(())
    }
}

/// Translates a bus error into the vault's vocabulary.
fn classify(error: &zbus::Error) -> Failure {
    match error {
        zbus::Error::MethodError(name, ..) => classify_name(name.as_str()),
        zbus::Error::FDO(error) => match &**error {
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_) => {
                unavailable()
            }
            zbus::fdo::Error::AccessDenied(_) => VaultError::Denied.into(),
            _ => VaultError::Failed.into(),
        },
        // The bus went away under a cached link.
        zbus::Error::InputOutput(_) => Failure::Stale,
        _ => VaultError::Failed.into(),
    }
}

fn classify_name(name: &str) -> Failure {
    match name {
        "org.freedesktop.DBus.Error.ServiceUnknown"
        | "org.freedesktop.DBus.Error.NameHasNoOwner"
        | "org.freedesktop.DBus.Error.UnknownObject" => unavailable(),
        "org.freedesktop.DBus.Error.AccessDenied" => VaultError::Denied.into(),
        "org.freedesktop.Secret.Error.IsLocked" => VaultError::Locked.into(),
        // Our session is gone (the service restarted): open a new one.
        "org.freedesktop.Secret.Error.NoSession" => Failure::Stale,
        _ => VaultError::Failed.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> SecretServiceVault {
        SecretServiceVault::for_app("com.example.app").unwrap()
    }

    #[test]
    fn a_secret_is_identified_by_schema_application_namespace_and_key() {
        let attributes = vault().attributes("tokens", "api");
        assert_eq!(attributes.len(), 4);
        assert_eq!(attributes[SCHEMA_ATTRIBUTE], SCHEMA);
        assert_eq!(attributes[APP_ATTRIBUTE], "com.example.app");
        assert_eq!(attributes[NAMESPACE_ATTRIBUTE], "tokens");
        assert_eq!(attributes[KEY_ATTRIBUTE], "api");
    }

    #[test]
    fn a_namespace_search_is_a_subset_of_a_secrets_attributes() {
        let vault = vault();
        let namespace = vault.namespace_attributes("tokens");
        let secret = vault.attributes("tokens", "api");
        assert!(!namespace.contains_key(KEY_ATTRIBUTE));
        assert!(namespace.iter().all(|(name, value)| secret[name] == *value));
    }

    #[test]
    fn bus_error_names_map_to_typed_errors() {
        let kind = |name| match classify_name(name) {
            Failure::Stale => None,
            Failure::Vault(error) => Some(error),
        };
        assert_eq!(kind("org.freedesktop.Secret.Error.IsLocked"), Some(VaultError::Locked));
        assert_eq!(kind("org.freedesktop.DBus.Error.AccessDenied"), Some(VaultError::Denied));
        assert_eq!(
            kind("org.freedesktop.DBus.Error.ServiceUnknown"),
            Some(VaultError::Unavailable {
                capability: VaultCapability::PlatformSecretStore,
            })
        );
        assert_eq!(kind("org.freedesktop.Secret.Error.NoSession"), None);
        assert_eq!(kind("org.example.Anything"), Some(VaultError::Failed));
    }

    #[test]
    fn a_missing_collection_is_recognised_by_its_error_name() {
        assert!(is_missing_object("org.freedesktop.DBus.Error.UnknownMethod"));
        assert!(is_missing_object("org.freedesktop.Secret.Error.NoSuchObject"));
        assert!(!is_missing_object("org.freedesktop.Secret.Error.IsLocked"));
    }

    #[test]
    fn an_invalid_application_id_is_rejected() {
        assert!(matches!(
            SecretServiceVault::for_app("a/b"),
            Err(VaultError::InvalidKey)
        ));
    }
}
