#[cfg(not(target_os = "android"))]
use std::sync::Mutex;

#[cfg(not(target_os = "android"))]
static CLIPBOARD: std::sync::OnceLock<Mutex<Option<arboard::Clipboard>>> =
    std::sync::OnceLock::new();

#[cfg(not(target_os = "android"))]
fn with_clipboard<T>(
    operation: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, String> {
    let cell = CLIPBOARD.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .map_err(|_| "System clipboard lock is poisoned".to_owned())?;
    if guard.is_none() {
        *guard = Some(
            arboard::Clipboard::new()
                .map_err(|error| format!("Cannot open system clipboard: {error}"))?,
        );
    }
    operation(guard.as_mut().expect("clipboard initialized"))
        .map_err(|error| format!("System clipboard error: {error}"))
}

#[cfg(not(target_os = "android"))]
pub(crate) fn set_text(_app: Option<&()>, text: String) -> Result<(), String> {
    with_clipboard(|clipboard| clipboard.set_text(text))
}

#[cfg(not(target_os = "android"))]
pub(crate) fn get_text(_app: Option<&()>) -> Result<String, String> {
    with_clipboard(arboard::Clipboard::get_text)
}

#[cfg(target_os = "android")]
pub(crate) fn set_text(
    app: Option<&winit::platform::android::activity::AndroidApp>,
    text: String,
) -> Result<(), String> {
    let app = app.ok_or_else(|| "Android activity is unavailable".to_owned())?;
    crate::android_platform::set_clipboard_text(app, &text)
}

#[cfg(target_os = "android")]
pub(crate) fn get_text(
    app: Option<&winit::platform::android::activity::AndroidApp>,
) -> Result<String, String> {
    let app = app.ok_or_else(|| "Android activity is unavailable".to_owned())?;
    crate::android_platform::clipboard_text(app)
}
