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
    pub system_inner_frame_left: i32,
    pub system_inner_frame_right: i32,
    pub system_inner_frame_top: i32,
    pub system_inner_frame_bottom: i32,
    pub overlay_caption_frame_width: i32,
    pub control_box_setting: ControlBoxSetting,
}

impl Default for WindowSetting {
    fn default() -> Self {
        Self {
            left_frame_width: 8,
            right_frame_width: 8,
            top_frame_height: 8,
            bottom_frame_height: 8,
            system_inner_frame_left: 2,
            system_inner_frame_right: 2,
            system_inner_frame_top: 2,
            system_inner_frame_bottom: 2,
            overlay_caption_frame_width: 2,
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
