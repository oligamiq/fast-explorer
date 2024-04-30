pub mod window;

use std::sync::Arc;

use parking_lot::{lock_api::RwLockReadGuard, RawRwLock, RwLock};
use window::WindowSetting;

#[derive(Default, Clone)]
pub struct SettingContext(Arc<RwLock<Settings>>);

impl SettingContext {
    #[inline]
    pub fn read(&self) -> RwLockReadGuard<RawRwLock, Settings> {
        self.0.read()
    }
}

#[derive(Default)]
pub struct Settings {
    window_setting: WindowSetting,
}

impl Settings {
    #[inline]
    pub fn window_setting<'a>(&'a self) -> &'a WindowSetting {
        &self.window_setting
    }
}
