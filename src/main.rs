use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use tray_icon::{menu::Menu, TrayIconBuilder};
use winit::event_loop::{ControlFlow, EventLoopBuilder};

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/icon.png");

    let event_loop = EventLoopBuilder::new().build().unwrap();

    let hotkeys_manager = GlobalHotKeyManager::new().unwrap();

    let hotkey = HotKey::new(Some(Modifiers::SHIFT), Code::KeyD);
    let hotkey2 = HotKey::new(Some(Modifiers::SHIFT | Modifiers::ALT), Code::KeyD);
    let hotkey3 = HotKey::new(None, Code::KeyF);

    hotkeys_manager.register(hotkey).unwrap();
    hotkeys_manager.register(hotkey2).unwrap();
    hotkeys_manager.register(hotkey3).unwrap();

    let global_hotkey_channel = GlobalHotKeyEvent::receiver();

    event_loop
        .run(move |n_event, event_loop| {
            event_loop.set_control_flow(ControlFlow::Poll);

            if let Ok(event) = global_hotkey_channel.try_recv() {
                println!("{event:?}");
                println!("n_event: {:?}", n_event);

                if hotkey2.id() == event.id && event.state == HotKeyState::Released {
                    hotkeys_manager.unregister(hotkey2).unwrap();
                }
            }
        })
        .unwrap();

    // let tray_menu = Menu::new();
    // let tray_icon = TrayIconBuilder::new()
    //     .with_menu(Box::new(tray_menu))
    //     .with_tooltip("system-tray - tray icon library!")
    //     .with_icon(icon)
    //     .build()
    //     .unwrap();
}
