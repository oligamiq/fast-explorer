#!/usr/bin/env python3
import argparse
import ctypes
import ctypes.util
import os
import sys
import time

X11 = ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6")
XTST = ctypes.CDLL(ctypes.util.find_library("Xtst") or "libXtst.so.6")
Display = ctypes.c_void_p
Window = ctypes.c_ulong
KeySym = ctypes.c_ulong
KeyCode = ctypes.c_uint


class XClassHint(ctypes.Structure):
    _fields_ = [("res_name", ctypes.c_void_p), ("res_class", ctypes.c_void_p)]


X11.XOpenDisplay.argtypes = [ctypes.c_char_p]
X11.XOpenDisplay.restype = Display
X11.XDefaultRootWindow.argtypes = [Display]
X11.XDefaultRootWindow.restype = Window
X11.XQueryTree.argtypes = [Display, Window, ctypes.POINTER(Window), ctypes.POINTER(Window), ctypes.POINTER(ctypes.POINTER(Window)), ctypes.POINTER(ctypes.c_uint)]
X11.XQueryTree.restype = ctypes.c_int
X11.XFetchName.argtypes = [Display, Window, ctypes.POINTER(ctypes.c_char_p)]
X11.XFetchName.restype = ctypes.c_int
X11.XGetClassHint.argtypes = [Display, Window, ctypes.POINTER(XClassHint)]
X11.XGetClassHint.restype = ctypes.c_int
X11.XFree.argtypes = [ctypes.c_void_p]
X11.XStringToKeysym.argtypes = [ctypes.c_char_p]
X11.XStringToKeysym.restype = KeySym
X11.XKeysymToKeycode.argtypes = [Display, KeySym]
X11.XKeysymToKeycode.restype = ctypes.c_ubyte
X11.XRaiseWindow.argtypes = [Display, Window]
X11.XSetInputFocus.argtypes = [Display, Window, ctypes.c_int, ctypes.c_ulong]
X11.XFlush.argtypes = [Display]
X11.XTranslateCoordinates.argtypes = [Display, Window, Window, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(Window)]
XTST.XTestFakeMotionEvent.argtypes = [Display, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
XTST.XTestFakeButtonEvent.argtypes = [Display, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
XTST.XTestFakeKeyEvent.argtypes = [Display, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]


def open_display() -> Display:
    if not os.environ.get("DISPLAY"):
        raise RuntimeError("DISPLAY is not set; X11 integration testing requires an X display")
    display = X11.XOpenDisplay(None)
    if not display:
        raise RuntimeError(f"cannot open X display {os.environ.get('DISPLAY')}")
    return display


def is_fast_explorer_client(display: Display, window: int) -> bool:
    hint = XClassHint()
    if not X11.XGetClassHint(display, Window(window), ctypes.byref(hint)):
        return False
    try:
        values = []
        for pointer in (hint.res_name, hint.res_class):
            if pointer:
                values.append(ctypes.cast(pointer, ctypes.c_char_p).value.decode("utf-8", errors="replace").lower())
        return any("fast-explorer" in value for value in values)
    finally:
        if hint.res_name:
            X11.XFree(hint.res_name)
        if hint.res_class:
            X11.XFree(hint.res_class)


def top_level_windows(display: Display, title: str) -> list[int]:
    root = int(X11.XDefaultRootWindow(display))
    pending = [root]
    visited = {root}
    found = []
    while pending:
        parent = pending.pop()
        root_return = Window()
        parent_return = Window()
        children = ctypes.POINTER(Window)()
        count = ctypes.c_uint()
        if not X11.XQueryTree(display, Window(parent), ctypes.byref(root_return), ctypes.byref(parent_return), ctypes.byref(children), ctypes.byref(count)):
            continue
        try:
            for index in range(count.value):
                window = int(children[index])
                if window not in visited:
                    visited.add(window)
                    pending.append(window)
                name = ctypes.c_char_p()
                if X11.XFetchName(display, Window(window), ctypes.byref(name)) and name.value:
                    try:
                        decoded = name.value.decode("utf-8", errors="replace")
                        if title in decoded and is_fast_explorer_client(display, window):
                            found.append(window)
                    finally:
                        X11.XFree(name)
        finally:
            if children:
                X11.XFree(children)
    return found


def focus(display: Display, window: int) -> None:
    X11.XRaiseWindow(display, Window(window))
    X11.XSetInputFocus(display, Window(window), 2, 0)
    X11.XFlush(display)
    time.sleep(0.05)


def keysym_code(display: Display, name: str) -> int:
    keysym = X11.XStringToKeysym(name.encode())
    if not keysym:
        raise RuntimeError(f"unknown X11 keysym: {name}")
    code = X11.XKeysymToKeycode(display, keysym)
    if not code:
        raise RuntimeError(f"no X11 keycode for: {name}")
    return int(code)

def fake_key(display: Display, key: str, down: bool) -> None:
    XTST.XTestFakeKeyEvent(display, keysym_code(display, key), 1 if down else 0, 0)


def send_combo(display: Display, window: int, combo: str) -> None:
    focus(display, window)
    parts = combo.split("+")
    key = parts[-1]
    modifier_names = {
        "ctrl": "Control_L",
        "shift": "Shift_L",
        "alt": "Alt_L",
        "meta": "Super_L",
    }
    modifiers = [modifier_names[item.lower()] for item in parts[:-1]]
    for modifier in modifiers:
        fake_key(display, modifier, True)
    fake_key(display, key, True)
    fake_key(display, key, False)
    for modifier in reversed(modifiers):
        fake_key(display, modifier, False)
    X11.XFlush(display)
    time.sleep(0.08)


def type_text(display: Display, window: int, text: str) -> None:
    focus(display, window)
    for char in text:
        if "a" <= char <= "z" or "0" <= char <= "9":
            fake_key(display, char, True)
            fake_key(display, char, False)
        elif "A" <= char <= "Z":
            fake_key(display, "Shift_L", True)
            fake_key(display, char.lower(), True)
            fake_key(display, char.lower(), False)
            fake_key(display, "Shift_L", False)
        else:
            raise RuntimeError(f"unsupported test text character: {char!r}")
    X11.XFlush(display)
    time.sleep(0.08)

def click(display: Display, window: int, x: int, y: int, button: int) -> None:
    focus(display, window)
    root = X11.XDefaultRootWindow(display)
    root_x = ctypes.c_int()
    root_y = ctypes.c_int()
    child = Window()
    ok = X11.XTranslateCoordinates(
        display, Window(window), root, x, y,
        ctypes.byref(root_x), ctypes.byref(root_y), ctypes.byref(child),
    )
    if not ok:
        raise RuntimeError("failed to translate window coordinates")
    XTST.XTestFakeMotionEvent(display, -1, root_x.value, root_y.value, 0)
    XTST.XTestFakeButtonEvent(display, button, 1, 0)
    XTST.XTestFakeButtonEvent(display, button, 0, 0)
    X11.XFlush(display)
    time.sleep(0.12)


def wait_window(display: Display, title: str, excluded: set[int], timeout: float) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        candidates = [item for item in top_level_windows(display, title) if item not in excluded]
        if candidates:
            return candidates[-1]
        time.sleep(0.05)
    raise TimeoutError(f"no new X11 window containing title {title!r}")


def parse_window(value: str) -> int:
    return int(value, 0)


def main() -> int:
    parser = argparse.ArgumentParser(description="Minimal X11/XTest driver for FastExplorer integration tests")
    sub = parser.add_subparsers(dest="command", required=True)
    windows = sub.add_parser("windows")
    windows.add_argument("--title", default="FastExplorer")
    wait = sub.add_parser("wait-window")
    wait.add_argument("--title", default="FastExplorer")
    wait.add_argument("--exclude", default="")
    wait.add_argument("--timeout", type=float, default=15.0)
    key = sub.add_parser("key")
    key.add_argument("--window", required=True, type=parse_window)
    key.add_argument("combo")
    text = sub.add_parser("type")
    text.add_argument("--window", required=True, type=parse_window)
    text.add_argument("text")
    pointer = sub.add_parser("click")
    pointer.add_argument("--window", required=True, type=parse_window)
    pointer.add_argument("--x", type=int, required=True)
    pointer.add_argument("--y", type=int, required=True)
    pointer.add_argument("--button", choices=["primary", "secondary"], default="primary")
    args = parser.parse_args()

    try:
        display = open_display()
        if args.command == "windows":
            for window in top_level_windows(display, args.title):
                print(hex(window))
        elif args.command == "wait-window":
            excluded = {int(item, 0) for item in args.exclude.split(",") if item}
            print(hex(wait_window(display, args.title, excluded, args.timeout)))
        elif args.command == "key":
            send_combo(display, args.window, args.combo)
        elif args.command == "type":
            type_text(display, args.window, args.text)
        elif args.command == "click":
            button = 1 if args.button == "primary" else 3
            click(display, args.window, args.x, args.y, button)
        return 0
    except Exception as error:
        print(f"x11-test-driver: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
