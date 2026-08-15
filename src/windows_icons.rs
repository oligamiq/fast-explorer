use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, SelectObject,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows_sys::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
use xilem::masonry::peniko::{ImageAlphaType, ImageData, ImageFormat};

use crate::app::EntryKind;

const ICON_SIZE: u32 = 20;
static ICON_CACHE: OnceLock<Mutex<HashMap<String, Option<ImageData>>>> = OnceLock::new();
pub fn shell_icon(path: &Path, display_name: &str, kind: EntryKind) -> Option<ImageData> {
    let (key, query, attributes, use_attributes) = shell_query(path, display_name, kind)?;
    let cache = ICON_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock()
        && let Some(cached) = cache.get(&key)
    {
        return cached.clone();
    }

    let loaded = unsafe { load_shell_icon(&query, attributes, use_attributes) };
    if let Ok(mut cache) = cache.lock() {
        cache.insert(key, loaded.clone());
    }
    loaded
}

fn shell_query(
    path: &Path,
    display_name: &str,
    kind: EntryKind,
) -> Option<(String, String, u32, bool)> {
    if kind == EntryKind::Directory {
        return Some((
            "folder".to_owned(),
            "folder".to_owned(),
            FILE_ATTRIBUTE_DIRECTORY,
            true,
        ));
    }
    if kind != EntryKind::File {
        return None;
    }
    let extension = Path::new(display_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let use_real_path = path.is_absolute() && matches!(extension.as_str(), "exe" | "lnk" | "ico");
    if use_real_path {
        return Some((
            format!("path:{}", path.to_string_lossy().to_ascii_lowercase()),
            path.to_string_lossy().into_owned(),
            FILE_ATTRIBUTE_NORMAL,
            false,
        ));
    }

    let query = if extension.is_empty() {
        "file".to_owned()
    } else {
        format!("file.{extension}")
    };
    Some((
        format!("ext:{extension}"),
        query,
        FILE_ATTRIBUTE_NORMAL,
        true,
    ))
}
unsafe fn load_shell_icon(query: &str, attributes: u32, use_attributes: bool) -> Option<ImageData> {
    let wide = std::ffi::OsStr::new(query)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut info = SHFILEINFOW::default();
    let mut flags = SHGFI_ICON | SHGFI_SMALLICON;
    if use_attributes {
        flags |= SHGFI_USEFILEATTRIBUTES;
    }
    // SAFETY: `wide` is NUL-terminated and `info` points to writable storage.
    let result = unsafe {
        SHGetFileInfoW(
            wide.as_ptr(),
            attributes,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if result == 0 || info.hIcon.is_null() {
        return None;
    }
    // SAFETY: SHGetFileInfoW returned an owned HICON that must be destroyed after conversion.
    let image = unsafe { icon_to_image(info.hIcon) };
    unsafe { DestroyIcon(info.hIcon) };
    image
}
unsafe fn icon_to_image(icon: HICON) -> Option<ImageData> {
    // SAFETY: a memory DC has no external lifetime dependency.
    let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
    if dc.is_null() {
        return None;
    }
    let mut bitmap_info = BITMAPINFO::default();
    bitmap_info.bmiHeader.biSize = std::mem::size_of_val(&bitmap_info.bmiHeader) as u32;
    bitmap_info.bmiHeader.biWidth = ICON_SIZE as i32;
    bitmap_info.bmiHeader.biHeight = -(ICON_SIZE as i32);
    bitmap_info.bmiHeader.biPlanes = 1;
    bitmap_info.bmiHeader.biBitCount = 32;
    bitmap_info.bmiHeader.biCompression = BI_RGB;

    let mut pixels = std::ptr::null_mut();
    // SAFETY: bitmap_info describes a valid top-down 32-bit DIB section.
    let bitmap = unsafe {
        CreateDIBSection(
            dc,
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut pixels,
            std::ptr::null_mut(),
            0,
        )
    };
    if bitmap.is_null() || pixels.is_null() {
        if !bitmap.is_null() {
            unsafe { DeleteObject(bitmap) };
        }
        unsafe { DeleteDC(dc) };
        return None;
    }
    // SAFETY: the DIB section and DC are valid until cleanup below.
    let previous = unsafe { SelectObject(dc, bitmap) };
    let drawn = unsafe {
        DrawIconEx(
            dc,
            0,
            0,
            icon,
            ICON_SIZE as i32,
            ICON_SIZE as i32,
            0,
            std::ptr::null_mut(),
            DI_NORMAL,
        )
    } != 0;
    let byte_len = ICON_SIZE as usize * ICON_SIZE as usize * 4;
    let mut data = drawn
        .then(|| unsafe { std::slice::from_raw_parts(pixels.cast::<u8>(), byte_len).to_vec() });
    if !previous.is_null() {
        unsafe { SelectObject(dc, previous) };
    }
    unsafe {
        DeleteObject(bitmap);
        DeleteDC(dc);
    }

    let bytes = data.as_mut()?;
    if bytes.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        for pixel in bytes.chunks_exact_mut(4) {
            if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
                pixel[3] = 255;
            }
        }
    }
    Some(ImageData {
        data: std::mem::take(bytes).into(),
        format: ImageFormat::Bgra8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: ICON_SIZE,
        height: ICON_SIZE,
    })
}
