use std::collections::HashMap;
use std::ptr::null_mut;

use global_hotkey::hotkey::{self, Code, HotKey, Modifiers};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyEventReceiver, GlobalHotKeyManager, HotKeyState,
};
use parking_lot::RwLock;
use setting::window::control_box::CaptionDirection;
use setting::SettingContext;
use windows::Win32::Graphics::Dwm::{self, DwmDefWindowProc};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    PAINTSTRUCT, RGBQUAD,
};
use windows_sys::Win32::UI::Controls::OpenThemeData;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

mod window;
use window::WindowWrapper;

mod setting;

#[derive(Default)]
struct State {
    // Use an `Option` to allow the window to not be available until the
    // application is properly running.
    window: Vec<WindowWrapper>,
    counter: i32,
    hotkey_state: Option<HotkeyStruct>,
    setting: SettingContext,
}

struct HotkeyStruct {
    global_hotkey_channel: GlobalHotKeyEventReceiver,
    hotkeys_manager: GlobalHotKeyManager,
    hotkey: HashMap<HandledHotKeys, HotKey>,
}

#[derive(PartialEq, Eq, Hash)]
enum HandledHotKeys {
    ShiftD,
    ShiftAltD,
    MetaShitE,
    // F,
}

impl State {
    fn hotkey(&mut self, event_loop: &ActiveEventLoop) {
        let hotkey_state = self.hotkey_state.as_ref().unwrap();
        if let Ok(event) = hotkey_state.global_hotkey_channel.try_recv() {
            println!("{event:?}");

            let hotkey2 = hotkey_state.hotkey.get(&HandledHotKeys::ShiftAltD).unwrap();
            if hotkey2.id() == event.id && event.state == HotKeyState::Released {
                hotkey_state.hotkeys_manager.unregister(*hotkey2).unwrap();
            }

            let hotkey = hotkey_state.hotkey.get(&HandledHotKeys::MetaShitE).unwrap();
            if hotkey.id() == event.id && event.state == HotKeyState::Pressed {
                self.window
                    .push(WindowWrapper::new(event_loop, self.setting.clone()));
            }
        }
    }
}

impl ApplicationHandler for State {
    // This is a common indicator that you can create a window.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.window
            .push(WindowWrapper::new(event_loop, self.setting.clone()));

        // println!("Application resumed");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // println!("window event: {event:?}")

        // `unwrap` is fine, the window will always be available when
        // receiving a window event.
        // let window = self.window.as_ref().unwrap();
        // Handle window event.

        // match &event {
        //     WindowEvent::Destroyed => {
        //         self.window.remove();
        //         return;
        //     }
        //     _ => {}
        // }

        if let Some(window_index) = &self.window.iter().position(|w| w.window.id() == window_id) {
            let window = &self.window[*window_index];
            match event {
                WindowEvent::Destroyed => {
                    println!("Window destroyed");
                    return;
                }
                WindowEvent::Focused(focus) => {
                    println!("Window focused: {focus}");
                }
                _ => {}
            }
            if window.check_dwm_is_composition() {
                // println!("DWM is enabled");

                match &event {
                    WindowEvent::RedrawRequested => {
                        // println!("Redraw requested");
                        //
                        window.paint();
                    }
                    WindowEvent::CloseRequested => {
                        println!("Close requested");

                        self.window.remove(*window_index);
                    }
                    _ => {}
                }

                // let mut f_call_dwp = true;
                // f_call_dwp = !unsafe { DwmDefWindowProc(hwnd, 0, ).into() } ;

                // match event {
                //     WindowEvent::Destroyed
                // }
            }
        }
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        // println!("device event: {event:?}");

        // Handle window event.

        self.hotkey(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // println!("Application about to wait");

        // if let Some(window) = self.window.as_ref() {
        //     window.request_redraw();
        //     self.counter += 1;
        // }

        // Pollから復帰するとき
        self.hotkey(event_loop);
    }
}

fn main() {
    let hotkeys_manager: GlobalHotKeyManager = GlobalHotKeyManager::new().unwrap();

    let hotkey = HotKey::new(Some(Modifiers::SHIFT), Code::KeyD);
    let hotkey2 = HotKey::new(Some(Modifiers::SHIFT | Modifiers::ALT), Code::KeyD);
    let hotkey_meta_shift_e = HotKey::new(Some(Modifiers::META | Modifiers::SHIFT), Code::KeyE);
    // let hotkey3 = HotKey::new(None, Code::KeyF);

    hotkeys_manager.register(hotkey).unwrap();
    hotkeys_manager.register(hotkey2).unwrap();
    hotkeys_manager.register(hotkey_meta_shift_e).unwrap();
    // hotkeys_manager.register(hotkey3).unwrap();

    let global_hotkey_channel = GlobalHotKeyEvent::receiver();

    let mut hashmap = HashMap::default();
    hashmap.insert(HandledHotKeys::ShiftD, hotkey);
    hashmap.insert(HandledHotKeys::ShiftAltD, hotkey2);
    hashmap.insert(HandledHotKeys::MetaShitE, hotkey_meta_shift_e);
    // hashmap.insert(HandledHotKeys::F, hotkey3);

    let hotkey_struct = HotkeyStruct {
        global_hotkey_channel: global_hotkey_channel.clone(),
        hotkey: hashmap,
        hotkeys_manager,
    };

    let event_loop = EventLoop::new().unwrap();
    let mut state = State {
        hotkey_state: Some(hotkey_struct),
        setting: SettingContext::new(setting::Settings {
            window_setting: crate::setting::window::WindowSetting {
                control_box_setting: crate::setting::window::control_box::ControlBoxSetting {
                    caption_wide: 30,
                    caption_direction: CaptionDirection::Left,
                    box_width: 30,
                    box_height: 40,
                    maximize_button: true,
                    minimize_button: true,
                    close_button: true,
                    position_x:
                        crate::setting::window::control_box::ControlBoxPositionAxis::Center {
                            margin: 0,
                        },
                    position_y: crate::setting::window::control_box::ControlBoxPositionAxis::Last,
                },
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };

    let _ = event_loop.run_app(&mut state);
}
