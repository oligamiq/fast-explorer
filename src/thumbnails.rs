use std::path::Path;

use image::ImageReader;
use xilem::masonry::peniko::{ImageAlphaType, ImageData, ImageFormat};

const THUMBNAIL_EDGE: u32 = 192;

pub fn load(path: &Path) -> Result<ImageData, String> {
    let reader = ImageReader::open(path).map_err(|error| error.to_string())?;
    let reader = reader
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let decoded = reader.decode().map_err(|error| error.to_string())?;
    let thumbnail = decoded.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE).to_rgba8();
    let (width, height) = thumbnail.dimensions();
    Ok(ImageData {
        data: thumbnail.into_raw().into(),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_thumbnail_returns_error() {
        assert!(load(Path::new("definitely-missing-thumbnail.png")).is_err());
    }
}
