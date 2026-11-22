//! Push-to-talk dictation (overnight 10-11 B7): macOS's Speech framework,
//! on-device only, fed from the microphone through AVAudioEngine. The
//! webview types what comes back into the composer; nothing here sends,
//! stores or forwards text, and no audio leaves the Mac.
//!
//! - **On-device or nothing.** Every request sets
//!   `requiresOnDeviceRecognition`; a recogniser that doesn't support it is
//!   refused with a reason ([`status_from`]). There is no network fallback.
//! - **Usage strings first.** macOS ends an app that asks for speech or
//!   microphone access without `NSSpeechRecognitionUsageDescription` /
//!   `NSMicrophoneUsageDescription` in its Info.plist (a development binary
//!   has none), so nothing in the Speech framework is touched unless the
//!   running bundle declares both.
//! - **One session at a time, on the main thread.** Start and stop run there
//!   (`run_on_main_thread`), the session lives in a main-thread
//!   `thread_local`, and each session has an id the webview passes back, so
//!   a late stop never ends a newer session.
//! - **Events.** [`EVENT`] carries `{session, kind, text}`: `partial` (the
//!   whole transcript so far, replacing the previous one), `final`, `error`
//!   (the reason, in words) and `ended`.

use serde::Serialize;

pub const EVENT: &str = "dictation-event";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationStatus {
    /// `ready` | `needsPermission` (macOS hasn't asked yet) | `denied` |
    /// `unavailable`.
    pub state: String,
    /// The recogniser's language, e.g. `en-US`, when there is one.
    pub locale: Option<String>,
    /// Why it isn't ready, in words.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationEvent {
    pub session: u64,
    /// `partial` | `final` | `error` | `ended`.
    pub kind: String,
    pub text: String,
}

/// Speech recognition authorisation, as macOS reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Auth {
    NotDetermined,
    Denied,
    Restricted,
    Authorized,
}

/// What the status is computed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Facts {
    pub supported_os: bool,
    pub usage_strings: bool,
    pub auth: Auth,
    /// Microphone access (macOS 14+ reports it; older systems ask when the
    /// microphone starts, so `Authorized` there means "not known to be off").
    pub mic: Auth,
    pub recognizer: bool,
    pub on_device: bool,
    pub available: bool,
    pub locale: Option<String>,
}

pub(crate) const NO_NETWORK: &str = "Hollow never sends your voice to a network service.";

fn status(state: &str, locale: Option<String>, reason: Option<String>) -> DictationStatus {
    DictationStatus {
        state: state.into(),
        locale,
        reason,
    }
}

/// The status, from the facts. Order matters: without the usage strings
/// nothing else is read (or asked); permissions come before the recogniser.
pub(crate) fn status_from(f: &Facts) -> DictationStatus {
    if !f.supported_os {
        return status(
            "unavailable",
            None,
            Some("Dictation needs macOS's on-device speech recognition.".into()),
        );
    }
    if !f.usage_strings {
        return status(
            "unavailable",
            None,
            Some("This build can't ask for the microphone or speech recognition (it has no permission entries). Dictation works in the installed app.".into()),
        );
    }
    match f.auth {
        Auth::Denied => {
            return status(
                "denied",
                None,
                Some("Speech recognition is off for Hollow. Turn it on in System Settings → Privacy & Security → Speech Recognition.".into()),
            )
        }
        Auth::Restricted => {
            return status(
                "denied",
                None,
                Some("Speech recognition is restricted on this Mac (Screen Time or a profile).".into()),
            )
        }
        Auth::NotDetermined => return status("needsPermission", f.locale.clone(), None),
        Auth::Authorized => {}
    }
    match f.mic {
        Auth::Denied | Auth::Restricted => {
            return status(
                "denied",
                None,
                Some("Microphone access is off for Hollow. Turn it on in System Settings → Privacy & Security → Microphone.".into()),
            )
        }
        Auth::NotDetermined => return status("needsPermission", f.locale.clone(), None),
        Auth::Authorized => {}
    }
    if !f.recognizer {
        return status(
            "unavailable",
            None,
            Some("There is no speech recogniser for your language on this Mac.".into()),
        );
    }
    if !f.on_device {
        let lang = f.locale.clone().unwrap_or_else(|| "your language".into());
        return status(
            "unavailable",
            f.locale.clone(),
            Some(format!(
                "On-device recognition isn't available for {lang} on this Mac, so dictation is off. {NO_NETWORK} Turn on Dictation in System Settings → Keyboard to download it."
            )),
        );
    }
    if !f.available {
        return status(
            "unavailable",
            f.locale.clone(),
            Some("Speech recognition is unavailable right now. Try again in a moment.".into()),
        );
    }
    status("ready", f.locale.clone(), None)
}

/// A recognition error's words for the person (the framework's own
/// description, or a plain line for the codes it leaves blank).
pub(crate) fn error_text(description: &str, code: isize) -> String {
    let d = description.trim();
    if !d.is_empty() && !d.starts_with("The operation couldn") {
        return d.to_string();
    }
    match code {
        // kAFAssistantErrorDomain 1110: no speech detected.
        1110 => "No speech was heard.".into(),
        // 1700: the on-device model is missing or not ready.
        1700 => format!("The on-device speech model isn't ready. {NO_NETWORK}"),
        _ => format!("Dictation stopped (error {code})."),
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::NSObjectProtocol;
    use objc2::runtime::{AnyClass, Bool};
    use objc2::{sel, AllocAnyThread};
    use objc2_avf_audio::{
        AVAudioApplication, AVAudioApplicationRecordPermission, AVAudioEngine, AVAudioPCMBuffer,
        AVAudioTime,
    };
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_speech::{
        SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionResult, SFSpeechRecognitionTask,
        SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus,
    };
    use std::cell::RefCell;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tauri::{AppHandle, Emitter};

    struct Session {
        id: u64,
        engine: Retained<AVAudioEngine>,
        request: Retained<SFSpeechAudioBufferRecognitionRequest>,
        task: Retained<SFSpeechRecognitionTask>,
        _recognizer: Retained<SFSpeechRecognizer>,
        // The blocks the framework holds stay alive with the session.
        _tap: RcBlock<dyn Fn(NonNull<AVAudioPCMBuffer>, NonNull<AVAudioTime>)>,
        _handler: RcBlock<dyn Fn(*mut SFSpeechRecognitionResult, *mut NSError)>,
    }

    thread_local! {
        /// The live session (main thread only).
        static LIVE: RefCell<Option<Session>> = const { RefCell::new(None) };
        /// The last stopped session, kept until its final result lands or
        /// the next one starts, so the framework's task isn't freed mid-way.
        static FINISHING: RefCell<Option<Session>> = const { RefCell::new(None) };
    }
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    pub(super) fn usage_strings() -> bool {
        let bundle = NSBundle::mainBundle();
        [
            "NSSpeechRecognitionUsageDescription",
            "NSMicrophoneUsageDescription",
        ]
        .iter()
        .all(|k| {
            bundle
                .objectForInfoDictionaryKey(&NSString::from_str(k))
                .is_some()
        })
    }

    fn auth_now() -> Auth {
        let s = unsafe { SFSpeechRecognizer::authorizationStatus() };
        if s == SFSpeechRecognizerAuthorizationStatus::Authorized {
            Auth::Authorized
        } else if s == SFSpeechRecognizerAuthorizationStatus::Denied {
            Auth::Denied
        } else if s == SFSpeechRecognizerAuthorizationStatus::Restricted {
            Auth::Restricted
        } else {
            Auth::NotDetermined
        }
    }

    /// Whether this macOS has AVAudioApplication (14+), which reports and
    /// asks for microphone access.
    fn has_audio_application() -> bool {
        AnyClass::get(c"AVAudioApplication").is_some()
    }

    fn mic_now() -> Auth {
        if !has_audio_application() {
            return Auth::Authorized;
        }
        let p = unsafe { AVAudioApplication::sharedInstance().recordPermission() };
        if p == AVAudioApplicationRecordPermission::Granted {
            Auth::Authorized
        } else if p == AVAudioApplicationRecordPermission::Denied {
            Auth::Denied
        } else {
            Auth::NotDetermined
        }
    }

    fn recognizer() -> Option<Retained<SFSpeechRecognizer>> {
        unsafe { SFSpeechRecognizer::init(SFSpeechRecognizer::alloc()) }
    }

    pub(super) fn facts() -> Facts {
        let mut f = Facts {
            supported_os: true,
            usage_strings: usage_strings(),
            auth: Auth::NotDetermined,
            mic: Auth::NotDetermined,
            recognizer: false,
            on_device: false,
            available: false,
            locale: None,
        };
        if !f.usage_strings {
            return f;
        }
        f.auth = auth_now();
        f.mic = mic_now();
        if let Some(r) = recognizer() {
            f.recognizer = true;
            unsafe {
                f.on_device = r.supportsOnDeviceRecognition();
                f.available = r.isAvailable();
                f.locale = Some(r.locale().localeIdentifier().to_string().replace('_', "-"));
            }
        }
        f
    }

    /// Ask macOS for speech recognition, then the microphone (each once;
    /// later calls return at once). Blocks the calling (non-main) thread
    /// until macOS answers, at most a minute each.
    pub(super) fn authorize() -> Result<(), String> {
        if !usage_strings() {
            return Ok(());
        }
        let wait = std::time::Duration::from_secs(60);
        if auth_now() == Auth::NotDetermined {
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let block = RcBlock::new(move |_s: SFSpeechRecognizerAuthorizationStatus| {
                let _ = tx.send(());
            });
            unsafe { SFSpeechRecognizer::requestAuthorization(&block) };
            rx.recv_timeout(wait)
                .map_err(|_| "macOS didn't answer the speech recognition prompt.".to_string())?;
        }
        if has_audio_application() && mic_now() == Auth::NotDetermined {
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let block = RcBlock::new(move |_granted: Bool| {
                let _ = tx.send(());
            });
            unsafe { AVAudioApplication::requestRecordPermissionWithCompletionHandler(&block) };
            rx.recv_timeout(wait)
                .map_err(|_| "macOS didn't answer the microphone prompt.".to_string())?;
        }
        Ok(())
    }

    /// The recogniser ended on its own (a final result or an error; main
    /// queue): stop the microphone if it is still live and free the session,
    /// so nothing keeps listening after the webview hears `ended`.
    fn ended(id: u64) {
        LIVE.with(|l| {
            let mut live = l.borrow_mut();
            if live.as_ref().is_some_and(|s| s.id == id) {
                if let Some(s) = live.take() {
                    halt(&s);
                }
            }
        });
        FINISHING.with(|f| {
            let mut fin = f.borrow_mut();
            if fin.as_ref().is_some_and(|s| s.id == id) {
                fin.take();
            }
        });
    }

    fn emit(app: &AppHandle, session: u64, kind: &str, text: String) {
        let _ = app.emit(
            EVENT,
            DictationEvent {
                session,
                kind: kind.into(),
                text,
            },
        );
    }

    fn halt(s: &Session) {
        unsafe {
            s.engine.stop();
            s.engine.inputNode().removeTapOnBus(0);
            s.request.endAudio();
        }
    }

    /// Start a session (main thread). Refuses unless the status is ready.
    pub(super) fn start(app: AppHandle) -> Result<u64, String> {
        let st = status_from(&facts());
        if st.state != "ready" {
            return Err(st.reason.unwrap_or_else(|| "Dictation isn't ready.".into()));
        }
        // One at a time: a session still running is stopped first.
        LIVE.with(|l| {
            if let Some(old) = l.borrow_mut().take() {
                halt(&old);
                unsafe { old.task.cancel() };
            }
        });
        FINISHING.with(|f| f.borrow_mut().take());
        let rec =
            recognizer().ok_or("There is no speech recogniser for your language on this Mac.")?;
        if !unsafe { rec.supportsOnDeviceRecognition() } {
            return Err(format!(
                "On-device recognition isn't available on this Mac. {NO_NETWORK}"
            ));
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let request = unsafe { SFSpeechAudioBufferRecognitionRequest::new() };
        unsafe {
            request.setRequiresOnDeviceRecognition(true);
            request.setShouldReportPartialResults(true);
            // macOS 13+: punctuation; older systems lack the setter.
            if request.respondsToSelector(sel!(setAddsPunctuation:)) {
                request.setAddsPunctuation(true);
            }
        }
        let engine = unsafe { AVAudioEngine::new() };
        let input = unsafe { engine.inputNode() };
        let format = unsafe { input.outputFormatForBus(0) };
        if unsafe { format.channelCount() } == 0 || unsafe { format.sampleRate() } <= 0.0 {
            return Err("No microphone input is available.".into());
        }
        let feed = request.clone();
        let tap = RcBlock::new(
            move |buffer: NonNull<AVAudioPCMBuffer>, _when: NonNull<AVAudioTime>| unsafe {
                feed.appendAudioPCMBuffer(buffer.as_ref());
            },
        );
        unsafe {
            input.installTapOnBus_bufferSize_format_block(
                0,
                1024,
                Some(&format),
                RcBlock::as_ptr(&tap),
            );
            engine.prepare();
        }
        if let Err(e) = unsafe { engine.startAndReturnError() } {
            unsafe { input.removeTapOnBus(0) };
            return Err(format!(
                "Couldn't start the microphone: {}",
                e.localizedDescription()
            ));
        }
        let events = app.clone();
        let handler = RcBlock::new(
            move |result: *mut SFSpeechRecognitionResult, error: *mut NSError| {
                if let Some(r) = unsafe { result.as_ref() } {
                    let text = unsafe { r.bestTranscription().formattedString() }.to_string();
                    let last = unsafe { r.isFinal() };
                    emit(&events, id, if last { "final" } else { "partial" }, text);
                    if last {
                        ended(id);
                        emit(&events, id, "ended", String::new());
                    }
                } else if let Some(e) = unsafe { error.as_ref() } {
                    emit(
                        &events,
                        id,
                        "error",
                        error_text(&e.localizedDescription().to_string(), e.code()),
                    );
                    ended(id);
                    emit(&events, id, "ended", String::new());
                }
            },
        );
        let task = unsafe { rec.recognitionTaskWithRequest_resultHandler(&request, &handler) };
        LIVE.with(|l| {
            *l.borrow_mut() = Some(Session {
                id,
                engine,
                request,
                task,
                _recognizer: rec,
                _tap: tap,
                _handler: handler,
            })
        });
        Ok(id)
    }

    /// Stop session `id` (main thread): the microphone stops and the
    /// recogniser finishes what it heard (its final result follows). A stop
    /// for any other session does nothing.
    pub(super) fn stop(id: u64) {
        LIVE.with(|l| {
            let mut live = l.borrow_mut();
            if live.as_ref().is_some_and(|s| s.id == id) {
                let s = live.take().expect("checked");
                halt(&s);
                unsafe { s.task.finish() };
                FINISHING.with(|f| *f.borrow_mut() = Some(s));
            }
        });
    }
}

#[cfg(target_os = "macos")]
fn on_main<T: Send + 'static>(
    app: &tauri::AppHandle,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(std::time::Duration::from_secs(15))
        .map_err(|_| "The app didn't answer in time.".to_string())
}

#[cfg(target_os = "macos")]
fn current(app: &tauri::AppHandle) -> Result<DictationStatus, String> {
    on_main(app, || status_from(&native::facts()))
}

#[cfg(not(target_os = "macos"))]
fn current(_app: &tauri::AppHandle) -> Result<DictationStatus, String> {
    Ok(status_from(&Facts {
        supported_os: false,
        usage_strings: false,
        auth: Auth::NotDetermined,
        mic: Auth::NotDetermined,
        recognizer: false,
        on_device: false,
        available: false,
        locale: None,
    }))
}

#[tauri::command]
pub async fn dictation_status(app: tauri::AppHandle) -> Result<DictationStatus, String> {
    ready_status(app).await
}

/// Whether on-device dictation is ready (the body of `dictation_status`, and
/// the COO's `system.dictation_status`). A read: it never asks macOS for
/// access or opens the microphone.
pub(crate) async fn ready_status(app: tauri::AppHandle) -> Result<DictationStatus, String> {
    tauri::async_runtime::spawn_blocking(move || current(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// Ask macOS for speech recognition access (its own prompt), then the status.
#[tauri::command]
pub async fn dictation_authorize(app: tauri::AppHandle) -> Result<DictationStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        native::authorize()?;
        current(&app)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Start listening; returns the session id its events carry.
#[tauri::command]
pub async fn dictation_start(app: tauri::AppHandle) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::Arc;
            // A start that comes too late (the main thread was busy past the
            // wait) is skipped, so no microphone opens for a session nobody
            // holds.
            let events = app.clone();
            let abandoned = Arc::new(AtomicBool::new(false));
            let flag = abandoned.clone();
            let out = on_main(&app, move || {
                if flag.load(Ordering::SeqCst) {
                    return Err("Dictation took too long to start.".to_string());
                }
                native::start(events)
            });
            if out.is_err() {
                abandoned.store(true, Ordering::SeqCst);
            }
            out?
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = app;
            Err("Dictation needs macOS's on-device speech recognition.".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stop session `session` (a stale id does nothing).
#[tauri::command]
pub async fn dictation_stop(app: tauri::AppHandle, session: u64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        {
            on_main(&app, move || native::stop(session))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app, session);
            Ok(())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The System Settings pane a refusal names (P1-55): only these two, so the
/// command can't be used to open any other address.
fn privacy_url(pane: &str) -> Result<&'static str, String> {
    match pane {
        "microphone" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        }
        "speech" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition")
        }
        other => Err(format!("Unknown privacy pane: {other}")),
    }
}

/// "Open System Settings" on a dictation refusal: opens Privacy & Security →
/// Microphone or Speech Recognition for the person. Opens nothing else.
#[tauri::command]
pub fn open_dictation_settings(app: tauri::AppHandle, pane: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(privacy_url(&pane)?, None::<String>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_url_opens_only_the_two_dictation_panes() {
        assert!(privacy_url("microphone")
            .unwrap()
            .ends_with("?Privacy_Microphone"));
        assert!(privacy_url("speech")
            .unwrap()
            .ends_with("?Privacy_SpeechRecognition"));
        assert!(privacy_url("https://example.com").is_err());
        assert!(privacy_url("calendars").is_err());
    }

    fn ready() -> Facts {
        Facts {
            supported_os: true,
            usage_strings: true,
            auth: Auth::Authorized,
            mic: Auth::Authorized,
            recognizer: true,
            on_device: true,
            available: true,
            locale: Some("en-US".into()),
        }
    }

    #[test]
    fn ready_only_with_everything_on_device() {
        assert_eq!(status_from(&ready()).state, "ready");
    }

    #[test]
    fn never_ready_without_on_device_recognition_and_says_no_network() {
        let s = status_from(&Facts {
            on_device: false,
            ..ready()
        });
        assert_eq!(s.state, "unavailable");
        let reason = s.reason.unwrap();
        assert!(
            reason.contains("en-US") && reason.contains(NO_NETWORK),
            "{reason}"
        );
    }

    #[test]
    fn usage_strings_are_checked_before_anything_else() {
        let s = status_from(&Facts {
            usage_strings: false,
            auth: Auth::Denied,
            ..ready()
        });
        assert_eq!(s.state, "unavailable");
        assert!(s.reason.unwrap().contains("permission entries"));
    }

    #[test]
    fn authorisation_states() {
        assert_eq!(
            status_from(&Facts {
                auth: Auth::NotDetermined,
                ..ready()
            })
            .state,
            "needsPermission"
        );
        let denied = status_from(&Facts {
            auth: Auth::Denied,
            ..ready()
        });
        assert_eq!(denied.state, "denied");
        assert!(denied.reason.unwrap().contains("Privacy & Security"));
        assert_eq!(
            status_from(&Facts {
                auth: Auth::Restricted,
                ..ready()
            })
            .state,
            "denied"
        );
    }

    #[test]
    fn microphone_access_counts_too() {
        let denied = status_from(&Facts {
            mic: Auth::Denied,
            ..ready()
        });
        assert_eq!(denied.state, "denied");
        assert!(denied.reason.unwrap().contains("Microphone"));
        assert_eq!(
            status_from(&Facts {
                mic: Auth::NotDetermined,
                ..ready()
            })
            .state,
            "needsPermission"
        );
    }

    #[test]
    fn no_recogniser_unavailable_service_and_other_systems() {
        assert_eq!(
            status_from(&Facts {
                recognizer: false,
                ..ready()
            })
            .state,
            "unavailable"
        );
        assert_eq!(
            status_from(&Facts {
                available: false,
                ..ready()
            })
            .state,
            "unavailable"
        );
        assert_eq!(
            status_from(&Facts {
                supported_os: false,
                ..ready()
            })
            .state,
            "unavailable"
        );
    }

    #[test]
    fn error_words() {
        assert_eq!(
            error_text("Recognition request was canceled", 216),
            "Recognition request was canceled"
        );
        assert_eq!(error_text("", 1110), "No speech was heard.");
        assert!(error_text("The operation couldn’t be completed.", 1700).contains(NO_NETWORK));
        assert_eq!(error_text("", 42), "Dictation stopped (error 42).");
    }
}
