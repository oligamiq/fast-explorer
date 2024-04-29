use std::collections::HashMap;

use global_hotkey::hotkey::{self, Code, HotKey, Modifiers};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyEventReceiver, GlobalHotKeyManager, HotKeyState,
};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

mod window;
use window::WindowWrapper;

#[derive(Default)]
struct State {
    // Use an `Option` to allow the window to not be available until the
    // application is properly running.
    window: Vec<WindowWrapper>,
    counter: i32,
    hotkey_state: Option<HotkeyStruct>,
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
    F,
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


                self.window.push(WindowWrapper::new(event_loop));
            }
        }
    }
}

impl ApplicationHandler for State {
    // This is a common indicator that you can create a window.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
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
    let hotkeys_manager = GlobalHotKeyManager::new().unwrap();

    let hotkey = HotKey::new(Some(Modifiers::SHIFT), Code::KeyD);
    let hotkey2 = HotKey::new(Some(Modifiers::SHIFT | Modifiers::ALT), Code::KeyD);
    let hotkey_meta_shift_e = HotKey::new(Some(Modifiers::META | Modifiers::SHIFT), Code::KeyE);
    let hotkey3 = HotKey::new(None, Code::KeyF);

    hotkeys_manager.register(hotkey).unwrap();
    hotkeys_manager.register(hotkey2).unwrap();
    hotkeys_manager.register(hotkey_meta_shift_e).unwrap();
    hotkeys_manager.register(hotkey3).unwrap();

    let global_hotkey_channel = GlobalHotKeyEvent::receiver();

    let mut hashmap = HashMap::default();
    hashmap.insert(HandledHotKeys::ShiftD, hotkey);
    hashmap.insert(HandledHotKeys::ShiftAltD, hotkey2);
    hashmap.insert(HandledHotKeys::MetaShitE, hotkey_meta_shift_e);
    hashmap.insert(HandledHotKeys::F, hotkey3);

    let hotkey_struct = HotkeyStruct {
        global_hotkey_channel: global_hotkey_channel.clone(),
        hotkey: hashmap,
        hotkeys_manager,
    };

    let event_loop = EventLoop::new().unwrap();
    let mut state = State {
        hotkey_state: Some(hotkey_struct),
        ..Default::default()
    };
    let _ = event_loop.run_app(&mut state);
}
