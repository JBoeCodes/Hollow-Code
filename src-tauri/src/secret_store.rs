//! Every Hollow secret in ONE Keychain item, read once per process.
//!
//! **Why one item.** Hollow is signed with a self-signed identity ("Hollow
//! Dev Signing"), which has no Team ID. macOS therefore stamps each Keychain
//! item Hollow writes with a partition list of `cdhash:<build hash>` entries,
//! and every rebuild has a new hash. The first read of an item by a new
//! build asks for the login-keychain password, once PER ITEM. With a
//! Keychain item per Claude account, plus Jev's and Microsoft's, that was
//! three to five password prompts after every install (the stable signing
//! identity satisfies the item's application list, but not its partition
//! list). One item means at most one prompt per build, and none once "Always
//! Allow" has added that build.
//!
//! **Layout.** Service [`SERVICE`], account [`ACCOUNT`], value a JSON object
//! `{"v":1,"secrets":{"<key>":"<value>"}}`. Keys: `claude:<account id>` (a
//! Claude setup-token), `jev` (Jev's settings JSON) and `apns` (the phone
//! API's push key: Key ID, team and the `.p8` text, `phone_access::push`),
//! `tracker:<provider>` (the task tracker's API key, `tracker::settings`). [`RETIRED_KEYS`]
//! (`m365`, the Microsoft tokens, retired with Outlook on 2026-09-26) are
//! dropped from memory when the bundle loads, so the next save removes them
//! without an extra Keychain write.
//!
//! **Migration.** A key missing from the bundle is looked up once in its old
//! per-item location ([`legacy_location`]); a hit is copied into the bundle
//! and, only after the bundle is saved, the old item is deleted. So the first
//! launch of this build may still prompt once per old item, and never again.
//!
//! **Failures.** A read the user denies (or a locked keychain) is remembered
//! for [`RETRY_AFTER`], so a stream of callers can't stack prompts; writes
//! never replace a bundle that could not be read (that would drop every other
//! secret). Values never reach logs or errors.
//!
//! **`hollow serve` (M1 PR 3).** A headless Mac has no login Keychain to
//! write (the mini's runner user has no default keychain at all), so serve
//! installs [`SecretFile`] instead ([`use_file`]): the same bundle as the
//! text of `<app data>/secrets.json`, mode 0600, written whole through a
//! temp file and a rename, every read and change under a `flock` on
//! `secrets.lock` and re-read from disk, so `hollow serve secret-set` and a
//! running serve never lose each other's keys. A file that is not the
//! current user's or that group or others can read is refused, not used.
//! Threat model: anyone who can read the serving user's files (that user,
//! and root, so any admin with sudo on that Mac) can read the secrets; the
//! file is never in the vault, a log or argv. The app keeps the Keychain.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

pub const SERVICE: &str = "com.hollow.secrets";
pub const ACCOUNT: &str = "bundle";
const SCHEMA: u32 = 1;
/// How long a failed (denied, locked) read is remembered before asking again.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);

pub const JEV: &str = "jev";
/// Keys of retired integrations, removed from the bundle on its next save.
const RETIRED_KEYS: &[&str] = &["m365"];

pub fn claude_key(id: &str) -> String {
    format!("claude:{id}")
}

/// Where a key lived before the bundle.
fn legacy_location(key: &str) -> Option<(&'static str, String)> {
    if let Some(id) = key.strip_prefix("claude:") {
        return Some(("com.hollow.claude-accounts", id.to_string()));
    }
    match key {
        JEV => Some(("com.hollow.jev", "typesafe".into())),
        _ => None,
    }
}

/// A backend failure. Never carries a secret.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

/// The raw credential store: the macOS Keychain in the app, a 0600 file in
/// `hollow serve` ([`SecretFile`]), memory in tests.
trait Backend: Send {
    fn read(&mut self, service: &str, account: &str) -> Result<Option<String>, StoreError>;
    fn write(&mut self, service: &str, account: &str, value: &str) -> Result<(), StoreError>;
    fn remove(&mut self, service: &str, account: &str) -> Result<(), StoreError>;
    /// Another process may change the bundle under this one (a file), so
    /// every read and change reads it again, holding [`Backend::exclusive`].
    fn shared(&self) -> bool {
        false
    }
    /// Held across a read-change-write, when [`Backend::shared`].
    fn exclusive(&mut self) -> Result<Option<crate::flock::HeldLock>, StoreError> {
        Ok(None)
    }
}

/// The file serve keeps its secrets in, in the app data folder.
pub(crate) const FILE_NAME: &str = "secrets.json";
/// The lock beside it.
const LOCK_NAME: &str = "secrets.lock";
/// How long a change waits for another process's.
const LOCK_WAIT: Duration = Duration::from_secs(10);

/// The bundle as a private file (`hollow serve`): see the module docs. It
/// holds one item, the bundle of the service it was made for; any other
/// item reads as absent (so there is no legacy import and `read_item` finds
/// nothing).
pub(crate) struct SecretFile {
    path: std::path::PathBuf,
    service: String,
}

impl SecretFile {
    pub(crate) fn new(path: std::path::PathBuf, service: String) -> Self {
        Self { path, service }
    }

    fn ours(&self, service: &str, account: &str) -> bool {
        service == self.service && account == ACCOUNT
    }

    fn named(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| FILE_NAME.to_string())
    }
}

impl Backend for SecretFile {
    fn read(&mut self, service: &str, account: &str) -> Result<Option<String>, StoreError> {
        use std::os::unix::fs::MetadataExt;
        if !self.ours(service, account) {
            return Ok(None);
        }
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => {
                return Err(StoreError(format!(
                    "Could not read Hollow's saved credentials ({}).",
                    self.named()
                )))
            }
        };
        let meta = file
            .metadata()
            .map_err(|_| StoreError(format!("Could not read {}.", self.named())))?;
        // SAFETY: geteuid has no preconditions.
        let me = unsafe { libc::geteuid() };
        if meta.uid() != me || meta.mode() & 0o077 != 0 {
            return Err(StoreError(format!(
                "{} must belong to this user and be readable by nobody else (chmod 600); it was not used.",
                self.named()
            )));
        }
        let mut text = String::new();
        std::io::Read::read_to_string(&mut &file, &mut text)
            .map_err(|_| StoreError(format!("Could not read {}.", self.named())))?;
        Ok(Some(text))
    }
    fn write(&mut self, service: &str, account: &str, value: &str) -> Result<(), StoreError> {
        if !self.ours(service, account) {
            return Err(StoreError(
                "Only Hollow's own credentials are kept in this file.".into(),
            ));
        }
        crate::vault::atomic_write_private(&self.path, value.as_bytes())
            .map_err(|_| StoreError(format!("Could not save {}.", self.named())))
    }
    fn remove(&mut self, service: &str, account: &str) -> Result<(), StoreError> {
        if !self.ours(service, account) {
            return Ok(());
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(StoreError(format!("Could not remove {}.", self.named()))),
        }
    }
    fn shared(&self) -> bool {
        true
    }
    fn exclusive(&mut self) -> Result<Option<crate::flock::HeldLock>, StoreError> {
        use std::os::unix::fs::OpenOptionsExt;
        let lock = self.path.with_file_name(LOCK_NAME);
        if let Some(dir) = lock.parent() {
            // The first secret on a fresh data folder.
            std::fs::create_dir_all(dir)
                .map_err(|_| StoreError("Could not create Hollow's app data folder.".into()))?;
        }
        let busy = || StoreError("Hollow's saved credentials are busy; try again.".into());
        let started = Instant::now();
        loop {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(&lock)
                .map_err(|_| StoreError(format!("Could not open {LOCK_NAME}.")))?;
            match crate::flock::HeldLock::try_take(file) {
                Ok(held) => return Ok(Some(held)),
                Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < LOCK_WAIT => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return Err(busy()),
            }
        }
    }
}

#[cfg_attr(test, allow(dead_code))]
struct Keychain;

impl Backend for Keychain {
    fn read(&mut self, service: &str, account: &str) -> Result<Option<String>, StoreError> {
        let entry = keyring::Entry::new(service, account)
            .map_err(|_| StoreError("Could not open Hollow's credential store.".into()))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(StoreError(
                "Could not read Hollow's saved credentials from the Keychain.".into(),
            )),
        }
    }
    fn write(&mut self, service: &str, account: &str, value: &str) -> Result<(), StoreError> {
        keyring::Entry::new(service, account)
            .and_then(|entry| entry.set_password(value))
            .map_err(|_| StoreError("Could not save Hollow's credentials in the Keychain.".into()))
    }
    fn remove(&mut self, service: &str, account: &str) -> Result<(), StoreError> {
        let entry = keyring::Entry::new(service, account)
            .map_err(|_| StoreError("Could not open Hollow's credential store.".into()))?;
        match entry.delete_password() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(StoreError(
                "Could not remove a credential from the Keychain.".into(),
            )),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Bundle {
    v: u32,
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

struct Store {
    backend: Box<dyn Backend>,
    /// [`SERVICE`] for the everyday app; a candidate build's own service
    /// (`instance::Instance::secret_service`).
    service: String,
    /// Whether pre-bundle items may be imported. They are the everyday app's.
    legacy: bool,
    /// The bundle's contents once read.
    secrets: Option<BTreeMap<String, String>>,
    /// When the bundle last failed to read.
    failed_at: Option<Instant>,
    /// When each legacy item last failed to read.
    legacy_failed: HashMap<String, Instant>,
    /// Keys whose legacy item is known to be gone (moved or never there).
    legacy_gone: HashSet<String>,
}

impl Store {
    fn new(backend: Box<dyn Backend>) -> Self {
        Self::for_instance(backend, crate::instance::current())
    }

    fn for_instance(backend: Box<dyn Backend>, instance: &crate::instance::Instance) -> Self {
        // Legacy items are Keychain items: a file has none.
        let legacy = instance.imports_legacy_secrets() && !backend.shared();
        Store {
            backend,
            service: instance.secret_service(),
            legacy,
            secrets: None,
            failed_at: None,
            legacy_failed: HashMap::new(),
            legacy_gone: HashSet::new(),
        }
    }

    fn loaded(&mut self) -> Result<&mut BTreeMap<String, String>, StoreError> {
        if self.secrets.is_none() {
            if self.failed_at.is_some_and(|at| at.elapsed() < RETRY_AFTER) {
                return Err(StoreError(
                    "Hollow's saved credentials are unavailable (Keychain access was not granted). Try again in a few minutes."
                        .into(),
                ));
            }
            let map = match self.backend.read(&self.service, ACCOUNT) {
                Ok(None) => BTreeMap::new(),
                Ok(Some(raw)) => match serde_json::from_str::<Bundle>(&raw) {
                    Ok(bundle) if bundle.v == SCHEMA => bundle.secrets,
                    _ => {
                        self.failed_at = Some(Instant::now());
                        return Err(StoreError(
                            "Hollow's saved credentials could not be read (unrecognized format)."
                                .into(),
                        ));
                    }
                },
                Err(error) => {
                    self.failed_at = Some(Instant::now());
                    return Err(error);
                }
            };
            self.failed_at = None;
            let mut map = map;
            for key in RETIRED_KEYS {
                map.remove(*key);
            }
            self.secrets = Some(map);
        }
        Ok(self.secrets.as_mut().expect("loaded"))
    }

    fn persist(&mut self) -> Result<(), StoreError> {
        let secrets = self.secrets.clone().unwrap_or_default();
        let raw = serde_json::to_string(&Bundle { v: SCHEMA, secrets })
            .map_err(|_| StoreError("Could not save Hollow's credentials.".into()))?;
        self.backend.write(&self.service, ACCOUNT, &raw)
    }

    /// Forget the copy in memory when another process may have changed it.
    /// A file prompts nobody, so a failed read is not remembered either:
    /// the next use reads it again (a `secret-set` or a `chmod` fixes it at
    /// once). The Keychain keeps [`RETRY_AFTER`].
    fn reread(&mut self) {
        if self.backend.shared() {
            self.secrets = None;
            self.failed_at = None;
        }
    }

    fn get(&mut self, key: &str) -> Result<Option<String>, StoreError> {
        self.reread();
        if let Some(value) = self.loaded()?.get(key) {
            return Ok(Some(value.clone()));
        }
        let Some((service, account)) = legacy_location(key).filter(|_| self.legacy) else {
            return Ok(None);
        };
        if self.legacy_gone.contains(key) {
            return Ok(None);
        }
        if self
            .legacy_failed
            .get(key)
            .is_some_and(|at| at.elapsed() < RETRY_AFTER)
        {
            return Ok(None);
        }
        let value = match self.backend.read(service, &account) {
            Ok(Some(value)) => value,
            Ok(None) => {
                self.legacy_gone.insert(key.to_string());
                return Ok(None);
            }
            Err(error) => {
                self.legacy_failed.insert(key.to_string(), Instant::now());
                return Err(error);
            }
        };
        self.loaded()?.insert(key.to_string(), value.clone());
        match self.persist() {
            // The old item goes only once its copy is saved.
            Ok(()) => match self.backend.remove(service, &account) {
                Ok(()) => {
                    self.legacy_gone.insert(key.to_string());
                }
                Err(error) => eprintln!("[secret-store] migrated {key}; old item kept: {error}"),
            },
            Err(error) => eprintln!("[secret-store] could not migrate {key}: {error}"),
        }
        Ok(Some(value))
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        let _held = self.backend.exclusive()?;
        self.reread();
        let previous = self.loaded()?.insert(key.to_string(), value.to_string());
        if let Err(error) = self.persist() {
            let map = self.secrets.as_mut().expect("loaded");
            match previous {
                Some(old) => map.insert(key.to_string(), old),
                None => map.remove(key),
            };
            return Err(error);
        }
        self.clear_legacy(key);
        Ok(())
    }

    fn delete(&mut self, key: &str) -> Result<(), StoreError> {
        let _held = self.backend.exclusive()?;
        self.reread();
        let previous = self.loaded()?.remove(key);
        if previous.is_some() {
            if let Err(error) = self.persist() {
                if let Some(old) = previous {
                    self.secrets
                        .as_mut()
                        .expect("loaded")
                        .insert(key.to_string(), old);
                }
                return Err(error);
            }
        }
        // Always try: an older build may have written the old item since.
        self.legacy_gone.remove(key);
        self.clear_legacy(key);
        Ok(())
    }

    /// The keys saved, never their values.
    fn keys(&mut self) -> Result<Vec<String>, StoreError> {
        self.reread();
        Ok(self.loaded()?.keys().cloned().collect())
    }

    /// Drop a stale pre-bundle copy so it can never be migrated back.
    fn clear_legacy(&mut self, key: &str) {
        if self.legacy_gone.contains(key) || !self.legacy {
            return;
        }
        if let Some((service, account)) = legacy_location(key) {
            if self.backend.remove(service, &account).is_ok() {
                self.legacy_gone.insert(key.to_string());
            }
        }
    }
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn store() -> MutexGuard<'static, Option<Store>> {
    let mut guard = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    if guard.is_none() {
        #[cfg(not(test))]
        let backend: Box<dyn Backend> = Box::new(Keychain);
        // Tests never touch the real Keychain.
        #[cfg(test)]
        let backend: Box<dyn Backend> = Box::new(tests::Memory::default());
        *guard = Some(Store::new(backend));
    }
    guard
}

/// `hollow serve`: keep the bundle in the private file at `path`
/// ([`SecretFile`]) for the rest of the process, instead of the Keychain.
/// Called before anything reads a secret.
pub(crate) fn use_file(path: std::path::PathBuf) {
    let service = crate::instance::current().secret_service();
    let mut guard = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = Some(Store::new(Box::new(SecretFile::new(path, service))));
}

/// The names of the keys saved (`hollow serve secret-list`), never values.
pub(crate) fn keys() -> Result<Vec<String>, StoreError> {
    store().as_mut().expect("store").keys()
}

/// The value saved under `key`, or None.
///
/// 1Password (`onepassword`, off by default) is asked first, and only for a
/// key the person gave an `op://` reference while the switch is on. The
/// Keychain stays the fallback: when the reference can't be read, the
/// Keychain's value for the same key is used if there is one; if there is
/// none, the caller gets 1Password's reason (fixed words), never `Ok(None)`,
/// which would read as "not set".
pub fn get(key: &str) -> Result<Option<String>, StoreError> {
    match crate::onepassword::lookup(key) {
        None => store().as_mut().expect("store").get(key),
        Some(Ok(value)) => Ok(Some(value)),
        Some(Err(why)) => match store().as_mut().expect("store").get(key) {
            Ok(Some(value)) => Ok(Some(value)),
            _ => Err(StoreError(why.message().to_string())),
        },
    }
}

/// Whether a value is saved under `key`, for a status line. A key read
/// through 1Password counts as saved without running `op`, so showing
/// Settings never raises an unlock prompt.
pub fn present(key: &str) -> Result<bool, StoreError> {
    if crate::onepassword::has_reference(key) {
        return Ok(true);
    }
    get(key).map(|v| v.is_some_and(|v| !v.is_empty()))
}

pub fn set(key: &str, value: &str) -> Result<(), StoreError> {
    store().as_mut().expect("store").set(key, value)
}

pub fn delete(key: &str) -> Result<(), StoreError> {
    store().as_mut().expect("store").delete(key)
}

/// Read another Keychain item once, outside the bundle and uncached (the
/// task tracker's "Use the key saved in Keychain", `tracker::settings`). The
/// caller copies what it needs into the bundle, so later builds never prompt
/// for this item again. Memory in tests, like the bundle.
pub fn read_item(service: &str, account: &str) -> Result<Option<String>, StoreError> {
    store()
        .as_mut()
        .expect("store")
        .backend
        .read(service, account)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    #[derive(Default, Clone)]
    pub(crate) struct Memory {
        items: Arc<StdMutex<BTreeMap<(String, String), String>>>,
        reads: Arc<StdMutex<Vec<(String, String)>>>,
        fail_reads: Arc<StdMutex<bool>>,
        fail_writes: Arc<StdMutex<bool>>,
    }

    impl Backend for Memory {
        fn read(&mut self, service: &str, account: &str) -> Result<Option<String>, StoreError> {
            self.reads
                .lock()
                .unwrap()
                .push((service.into(), account.into()));
            if *self.fail_reads.lock().unwrap() {
                return Err(StoreError("denied".into()));
            }
            Ok(self
                .items
                .lock()
                .unwrap()
                .get(&(service.into(), account.into()))
                .cloned())
        }
        fn write(&mut self, service: &str, account: &str, value: &str) -> Result<(), StoreError> {
            if *self.fail_writes.lock().unwrap() {
                return Err(StoreError("write refused".into()));
            }
            self.items
                .lock()
                .unwrap()
                .insert((service.into(), account.into()), value.into());
            Ok(())
        }
        fn remove(&mut self, service: &str, account: &str) -> Result<(), StoreError> {
            self.items
                .lock()
                .unwrap()
                .remove(&(service.to_string(), account.to_string()));
            Ok(())
        }
    }

    /// Put an item in the test store's memory backend, for [`read_item`].
    pub(crate) fn seed_item(service: &str, account: &str, value: &str) {
        store()
            .as_mut()
            .expect("store")
            .backend
            .write(service, account, value)
            .unwrap();
    }

    impl Memory {
        fn put(&self, service: &str, account: &str, value: &str) {
            self.items
                .lock()
                .unwrap()
                .insert((service.into(), account.into()), value.into());
        }
        fn has(&self, service: &str, account: &str) -> bool {
            self.items
                .lock()
                .unwrap()
                .contains_key(&(service.to_string(), account.to_string()))
        }
        fn bundle_reads(&self) -> usize {
            self.reads
                .lock()
                .unwrap()
                .iter()
                .filter(|(s, _)| s == SERVICE)
                .count()
        }
        fn raw(&self, service: &str, account: &str) -> Option<String> {
            self.items
                .lock()
                .unwrap()
                .get(&(service.to_string(), account.to_string()))
                .cloned()
        }
    }

    fn fresh() -> (Store, Memory) {
        let memory = Memory::default();
        (Store::new(Box::new(memory.clone())), memory)
    }

    #[test]
    fn a_candidate_build_keeps_its_own_item_and_never_reads_the_everyday_ones() {
        let memory = Memory::default();
        memory.put(
            SERVICE,
            ACCOUNT,
            "{\"v\":1,\"secrets\":{\"jev\":\"everyday\"}}",
        );
        memory.put("com.hollow.claude-accounts", "a", "everyday-legacy");
        let candidate = crate::instance::Instance::from_identifier("com.hollow.app.candidate");
        let mut store = Store::for_instance(Box::new(memory.clone()), &candidate);

        assert_eq!(
            store.get(JEV).unwrap(),
            None,
            "the everyday bundle is not the candidate's"
        );
        assert_eq!(
            store.get(&claude_key("a")).unwrap(),
            None,
            "no legacy import"
        );
        store.set(JEV, "candidate").unwrap();
        store.delete(&claude_key("a")).unwrap();

        let read: Vec<String> = memory
            .reads
            .lock()
            .unwrap()
            .iter()
            .map(|(s, _)| s.clone())
            .collect();
        assert!(
            read.iter().all(|s| s == "com.hollow.secrets.candidate"),
            "{read:?}"
        );
        assert!(memory
            .raw("com.hollow.secrets.candidate", ACCOUNT)
            .unwrap()
            .contains("candidate"));
        assert!(
            memory.raw(SERVICE, ACCOUNT).unwrap().contains("everyday"),
            "everyday bundle untouched"
        );
        assert!(
            memory.has("com.hollow.claude-accounts", "a"),
            "legacy item untouched"
        );
    }

    #[test]
    fn every_secret_shares_one_item_read_once() {
        let (mut store, memory) = fresh();
        store.set(&claude_key("a"), "tok-a").unwrap();
        store.set(&claude_key("b"), "tok-b").unwrap();
        store.set(JEV, "{\"enabled\":true}").unwrap();

        // A new process: one Keychain read serves every key.
        let mut next = Store::new(Box::new(memory.clone()));
        assert_eq!(
            next.get(&claude_key("a")).unwrap().as_deref(),
            Some("tok-a")
        );
        assert_eq!(
            next.get(&claude_key("b")).unwrap().as_deref(),
            Some("tok-b")
        );
        assert_eq!(
            next.get(JEV).unwrap().as_deref(),
            Some("{\"enabled\":true}")
        );
        assert_eq!(
            memory.bundle_reads(),
            2,
            "one read per process, not per key"
        );
        let raw = memory.raw(SERVICE, ACCOUNT).unwrap();
        assert!(raw.contains("\"v\":1") && raw.contains("claude:a"));
    }

    #[test]
    fn legacy_items_are_moved_into_the_bundle_then_deleted() {
        let (mut store, memory) = fresh();
        memory.put("com.hollow.claude-accounts", "acct-1", "legacy-token");
        memory.put("com.hollow.jev", "typesafe", "{\"enabled\":false}");

        assert_eq!(
            store.get(&claude_key("acct-1")).unwrap().as_deref(),
            Some("legacy-token")
        );
        assert_eq!(
            store.get(JEV).unwrap().as_deref(),
            Some("{\"enabled\":false}")
        );
        assert!(!memory.has("com.hollow.claude-accounts", "acct-1"));
        assert!(!memory.has("com.hollow.jev", "typesafe"));

        let mut next = Store::new(Box::new(memory.clone()));
        assert_eq!(
            next.get(&claude_key("acct-1")).unwrap().as_deref(),
            Some("legacy-token")
        );
    }

    #[test]
    fn an_absent_legacy_item_is_looked_up_once() {
        let (mut store, memory) = fresh();
        for _ in 0..3 {
            assert_eq!(store.get(JEV).unwrap(), None);
        }
        let legacy = memory
            .reads
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| s == "com.hollow.jev")
            .count();
        assert_eq!(legacy, 1);
        assert_eq!(memory.bundle_reads(), 1);
    }

    #[test]
    fn a_failed_bundle_save_keeps_the_legacy_item() {
        let (mut store, memory) = fresh();
        memory.put("com.hollow.jev", "typesafe", "legacy");
        *memory.fail_writes.lock().unwrap() = true;
        assert_eq!(store.get(JEV).unwrap().as_deref(), Some("legacy"));
        assert!(memory.has("com.hollow.jev", "typesafe"));
    }

    #[test]
    fn a_denied_read_is_not_retried_at_once_and_never_overwritten() {
        let (mut store, memory) = fresh();
        memory.put(SERVICE, ACCOUNT, "{\"v\":1,\"secrets\":{\"jev\":\"keep\"}}");
        *memory.fail_reads.lock().unwrap() = true;
        assert!(store.get(JEV).is_err());
        assert!(store.get(JEV).is_err());
        assert!(store.set(&claude_key("x"), "new").is_err());
        assert_eq!(memory.bundle_reads(), 1, "a denial must not stack prompts");
        assert!(memory.raw(SERVICE, ACCOUNT).unwrap().contains("keep"));
    }

    #[test]
    fn retired_keys_are_dropped_on_the_next_save() {
        let (mut store, memory) = fresh();
        memory.put(
            SERVICE,
            ACCOUNT,
            "{\"v\":1,\"secrets\":{\"jev\":\"keep\",\"m365\":\"old-token\"}}",
        );
        assert_eq!(
            store.get("m365").unwrap(),
            None,
            "a retired key is never handed out"
        );
        assert!(
            memory.raw(SERVICE, ACCOUNT).unwrap().contains("m365"),
            "no write just for reading"
        );
        store.set(JEV, "new").unwrap();
        let raw = memory.raw(SERVICE, ACCOUNT).unwrap();
        assert!(!raw.contains("m365") && raw.contains("new"));
    }

    #[test]
    fn an_unrecognized_bundle_is_never_replaced() {
        let (mut store, memory) = fresh();
        memory.put(SERVICE, ACCOUNT, "not json");
        assert!(store.set(JEV, "x").is_err());
        assert_eq!(memory.raw(SERVICE, ACCOUNT).as_deref(), Some("not json"));
    }

    #[test]
    fn delete_removes_the_key_and_any_legacy_copy() {
        let (mut store, memory) = fresh();
        store.set(&claude_key("a"), "tok").unwrap();
        memory.put("com.hollow.claude-accounts", "a", "stale");
        store.delete(&claude_key("a")).unwrap();
        assert!(!memory.has("com.hollow.claude-accounts", "a"));
        assert_eq!(store.get(&claude_key("a")).unwrap(), None);
    }

    #[test]
    fn a_failed_write_leaves_the_cache_as_it_was() {
        let (mut store, memory) = fresh();
        store.set(JEV, "one").unwrap();
        *memory.fail_writes.lock().unwrap() = true;
        assert!(store.set(JEV, "two").is_err());
        assert!(store.delete(JEV).is_err());
        assert_eq!(store.get(JEV).unwrap().as_deref(), Some("one"));
    }

    // ── The file backend (`hollow serve`) ────────────────────────────────

    fn file_store(dir: &std::path::Path) -> Store {
        Store::new(Box::new(SecretFile::new(
            dir.join(FILE_NAME),
            crate::instance::current().secret_service(),
        )))
    }

    fn mode_of(path: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_file_keeps_the_bundle_owner_only_across_processes() {
        let root = tempfile::tempdir().unwrap();
        // A data folder that doesn't exist yet is made by the first save.
        let dir = root.path().join("fresh-data");
        let dir = dir.as_path();
        let mut store = file_store(dir);
        assert_eq!(store.get(JEV).unwrap(), None, "no file reads as empty");
        store.set(&claude_key("a"), "tok-a").unwrap();
        store.set("runner.github.token", "ghp-x").unwrap();
        let path = dir.join(FILE_NAME);
        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(mode_of(&dir.join(LOCK_NAME)), 0o600);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"v\":1") && raw.contains("claude:a"));

        let mut next = file_store(dir);
        assert_eq!(
            next.get(&claude_key("a")).unwrap().as_deref(),
            Some("tok-a")
        );
        assert_eq!(
            next.keys().unwrap(),
            vec!["claude:a".to_string(), "runner.github.token".to_string()]
        );
        next.delete(&claude_key("a")).unwrap();
        // The first store sees the other's change: it reads the file again.
        assert_eq!(store.get(&claude_key("a")).unwrap(), None);
        assert_eq!(
            store.get("runner.github.token").unwrap().as_deref(),
            Some("ghp-x")
        );
        // Other items (legacy, `read_item`) are absent, never written.
        let mut backend = SecretFile::new(path.clone(), SERVICE.into());
        assert_eq!(backend.read("com.hollow.jev", "typesafe").unwrap(), None);
        assert!(backend.write("com.hollow.jev", "typesafe", "x").is_err());
    }

    #[test]
    fn a_damaged_or_open_file_is_never_used_or_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "not json").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut store = file_store(dir.path());
        assert!(store.get(JEV).is_err());
        assert!(store.set(JEV, "x").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");

        // Readable by others: refused, and left as it is.
        let open = tempfile::tempdir().unwrap();
        let mut first = file_store(open.path());
        first.set(JEV, "keep").unwrap();
        let path = open.path().join(FILE_NAME);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut store = file_store(open.path());
        let error = store.get(JEV).unwrap_err();
        assert!(error.0.contains("chmod 600"), "{error}");
        assert!(!error.0.contains("keep"), "no value in an error");
        assert!(store.set(JEV, "new").is_err());
        assert!(std::fs::read_to_string(&path).unwrap().contains("keep"));
        assert_eq!(mode_of(&path), 0o644);
    }

    /// A file fixed by a chmod (or a `secret-set`) is used at the next read,
    /// not after the Keychain's five minutes, and its error never names the
    /// Keychain.
    #[test]
    fn a_fixed_file_is_read_again_at_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        file_store(dir.path()).set(JEV, "keep").unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut store = file_store(dir.path());
        for _ in 0..2 {
            let error = store.get(JEV).unwrap_err();
            assert!(!error.0.contains("Keychain"), "{error}");
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(store.get(JEV).unwrap().as_deref(), Some("keep"));
    }

    #[test]
    fn writers_in_two_stores_never_lose_each_others_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let writers: Vec<_> = (0..4)
            .map(|w| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut store = file_store(&path);
                    for i in 0..15 {
                        store.set(&format!("k{w}-{i}"), "v").unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(file_store(&path).keys().unwrap().len(), 60);
    }
    /// A store over [`SecretFile`] at `path`, for tests outside this module
    /// (`serve_control`'s `secret-set` path) that must not touch the global one.
    pub(crate) struct TestFile(Store);

    impl TestFile {
        pub(crate) fn at(path: std::path::PathBuf) -> Self {
            Self(Store::new(Box::new(SecretFile::new(
                path,
                crate::instance::current().secret_service(),
            ))))
        }
        pub(crate) fn set(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
            self.0.set(key, value)
        }
        pub(crate) fn get(&mut self, key: &str) -> Result<Option<String>, StoreError> {
            self.0.get(key)
        }
    }
}
