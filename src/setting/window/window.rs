use std::ops::Deref;

use super::control_box::ControlBoxSetting;

/// これは余白
/// この範囲がダブルクリックで拡大や、ドラッグで移動できる範囲
#[derive(Debug, Clone, Copy)]
pub struct WindowSetting {
    pub left_frame_width: i32,
    pub right_frame_width: i32,
    pub top_frame_height: i32,
    pub bottom_frame_height: i32,
    pub control_box_setting: ControlBoxSetting,
}

impl Default for WindowSetting {
    fn default() -> Self {
        Self {
            left_frame_width: 5,
            right_frame_width: 5,
            top_frame_height: 5,
            bottom_frame_height: 5,
            control_box_setting: ControlBoxSetting::default(),
        }
    }
}

impl Deref for WindowSetting {
    type Target = WindowSetting;

    fn deref(&self) -> &Self::Target {
        self
    }
}

pub struct PinnedWindowSetting {
    setting: WindowSetting,
    _pin: std::marker::PhantomPinned,
}

impl PinnedWindowSetting {
    pub fn new(setting: WindowSetting) -> Self {
        Self {
            setting,
            _pin: std::marker::PhantomPinned,
        }
    }

    #[inline]
    pub fn setting(&self) -> &WindowSetting {
        &self.setting
    }
}

impl PinnedWindowSetting {
    #[inline]
    pub fn pointer(&self) -> &Self {
        self
    }
}
