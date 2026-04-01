use crate::error::ImageError;
use std::io::Cursor;
use std::path::{Path, PathBuf};

pub fn resize_image(bytes: &[u8], max_dimension: u32) -> Result<Vec<u8>, ImageError> {
    // Decode the image
    let img = image::load_from_memory(bytes)
        .map_err(|e| ImageError::ResizeFailed(format!("Failed to decode image: {}", e)))?;
    
    let (width, height) = (img.width(), img.height());
    
    // No upscaling: if both dimensions are within limit, return original bytes
    if width <= max_dimension && height <= max_dimension {
        return Ok(bytes.to_vec());
    }
    
    // Calculate new dimensions preserving aspect ratio
    let (new_width, new_height) = if width > height {
        // Landscape or square: width is longest edge
        let ratio = max_dimension as f32 / width as f32;
        (max_dimension, (height as f32 * ratio).round() as u32)
    } else {
        // Portrait: height is longest edge
        let ratio = max_dimension as f32 / height as f32;
        ((width as f32 * ratio).round() as u32, max_dimension)
    };
    
    // Resize with Lanczos3 filter
    let resized = image::imageops::resize(
        &img,
        new_width,
        new_height,
        image::imageops::FilterType::Lanczos3,
    );
    
    // Convert to RGB8 if needed (JPEG doesn't support RGBA)
    let rgb_image = image::DynamicImage::ImageRgba8(resized).to_rgb8();
    
    // Encode to JPEG at 85% quality
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 85);
    image::DynamicImage::ImageRgb8(rgb_image)
        .write_with_encoder(encoder)
        .map_err(|e| ImageError::ResizeFailed(format!("Failed to encode JPEG: {}", e)))?;
    
    Ok(buffer)
}

pub fn encode_base64(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    
    let encoded = STANDARD.encode(bytes);
    format!("data:image/jpeg;base64,{}", encoded)
}

pub fn sanitize_slug(raw: &str) -> String {
    // Convert to lowercase and replace non-alphanumeric with hyphens in a single pass,
    // collapsing consecutive hyphens as we go (O(n) instead of O(n²))
    let mut slug = String::with_capacity(raw.len());
    let mut last_was_hyphen = true; // Start true to skip leading hyphens

    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            // Only add hyphen if previous char wasn't already a hyphen
            slug.push('-');
            last_was_hyphen = true;
        }
        // else: skip consecutive non-alphanumeric chars
    }

    // Trim trailing hyphen
    if slug.ends_with('-') {
        slug.pop();
    }

    // Truncate to 50 chars
    if slug.len() > 50 {
        slug.truncate(50);
        // Re-trim trailing hyphen in case truncation created one
        if slug.ends_with('-') {
            slug.pop();
        }
    }

    slug
}

pub fn generate_filename(date: &str, slug: &str) -> String {
    let safe_date = date.replace(['/', '\\'], "-");
    let sanitized = sanitize_slug(slug);
    let uuid_suffix = &uuid::Uuid::new_v4().to_string()[..4];
    format!("{}-{}-{}.jpg", safe_date, sanitized, uuid_suffix)
}

pub async fn save_image(
    bytes: &[u8],
    daily_note_dir: &Path,
    assets_folder: &str,
    filename: &str,
) -> Result<PathBuf, ImageError> {
    // Build asset directory path
    let assets_dir = daily_note_dir.join(assets_folder);
    
    // Create directory if it doesn't exist
    tokio::fs::create_dir_all(&assets_dir)
        .await
        .map_err(|e| ImageError::SaveFailed(format!("Failed to create assets directory: {}", e)))?;
    
    // Build full file path
    let full_path = assets_dir.join(filename);
    
    // Write image bytes to file
    tokio::fs::write(&full_path, bytes)
        .await
        .map_err(|e| ImageError::SaveFailed(format!("Failed to write image file: {}", e)))?;
    
    Ok(full_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use std::io::Cursor;

    /// Helper: Create a solid-color RGB8 image and encode as JPEG bytes
    fn create_test_image(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(width, height, |_, _| {
            Rgb(color)
        });
        
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        
        img.write_to(&mut cursor, image::ImageFormat::Jpeg)
            .expect("Failed to encode test image");
        
        buffer
    }

    #[test]
    fn test_resize_large_image() {
        // Create 3000x2000 image (aspect 1.5)
        let bytes = create_test_image(3000, 2000, [255, 0, 0]);
        
        let resized = resize_image(&bytes, 1280).expect("Resize failed");
        
        // Decode to verify dimensions
        let img = image::load_from_memory(&resized).expect("Failed to decode resized image");
        
        // Longest edge should be 1280, aspect preserved
        // 3000/2000 = 1.5 → 1280x853 (1280/853 ≈ 1.5)
        assert_eq!(img.width(), 1280);
        assert_eq!(img.height(), 853);
    }

    #[test]
    fn test_resize_small_image_unchanged() {
        // Create 800x600 image
        let bytes = create_test_image(800, 600, [0, 255, 0]);
        
        let result = resize_image(&bytes, 1280).expect("Resize failed");
        
        // Decode to verify no upscaling
        let img = image::load_from_memory(&result).expect("Failed to decode");
        
        assert_eq!(img.width(), 800);
        assert_eq!(img.height(), 600);
    }

    #[test]
    fn test_resize_tall_image() {
        // Create 1000x3000 image (taller than wide, aspect 0.333)
        let bytes = create_test_image(1000, 3000, [0, 0, 255]);
        
        let resized = resize_image(&bytes, 1280).expect("Resize failed");
        
        let img = image::load_from_memory(&resized).expect("Failed to decode resized image");
        
        // Longest edge (height) should be 1280
        // 1000/3000 = 0.333 → 427x1280 (427/1280 ≈ 0.333)
        assert_eq!(img.width(), 427);
        assert_eq!(img.height(), 1280);
    }

    #[test]
    fn test_encode_jpeg() {
        let bytes = create_test_image(100, 100, [128, 128, 128]);
        
        let resized = resize_image(&bytes, 1280).expect("Resize failed");
        
        // Verify JPEG magic bytes (FF D8)
        assert_eq!(resized[0], 0xFF);
        assert_eq!(resized[1], 0xD8);
    }

    #[test]
    fn test_encode_base64() {
        let test_bytes = vec![1, 2, 3, 4];
        
        let encoded = encode_base64(&test_bytes);
        
        // Should start with data:image/jpeg;base64,
        assert!(encoded.starts_with("data:image/jpeg;base64,"));
        
        // Extract base64 part and verify it's valid
        let base64_part = encoded.strip_prefix("data:image/jpeg;base64,").unwrap();
        assert!(!base64_part.is_empty());
    }

    #[tokio::test]
    async fn test_save_to_assets_dir() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let daily_note_dir = temp_dir.path();
        
        let test_bytes = create_test_image(100, 100, [255, 255, 0]);
        
        let result = save_image(&test_bytes, daily_note_dir, "assets", "test.jpg")
            .await
            .expect("Save failed");
        
        // Verify file exists
        assert!(result.exists());
        
        // Verify can read back
        let read_bytes = tokio::fs::read(&result).await.expect("Failed to read saved file");
        assert!(!read_bytes.is_empty());
    }

    #[tokio::test]
    async fn test_create_assets_dir() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let daily_note_dir = temp_dir.path();
        
        let test_bytes = create_test_image(50, 50, [200, 100, 50]);
        
        // Assets dir doesn't exist yet
        let assets_path = daily_note_dir.join("assets");
        assert!(!assets_path.exists());
        
        let result = save_image(&test_bytes, daily_note_dir, "assets", "auto-create.jpg")
            .await
            .expect("Save failed");
        
        // Verify dir was created
        assert!(assets_path.exists());
        assert!(result.exists());
    }

    #[test]
    fn test_sanitize_slug() {
        // Test with special chars, spaces, umlauts
        let raw = "Schöner Sonnenuntergang am Meer!!";
        
        let sanitized = sanitize_slug(raw);
        
        // Should be lowercase, alphanumeric + hyphens only
        assert!(sanitized.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        
        // No leading/trailing hyphens
        assert!(!sanitized.starts_with('-'));
        assert!(!sanitized.ends_with('-'));
        
        // Max 50 chars
        assert!(sanitized.len() <= 50);
        
        // Should not have consecutive hyphens
        assert!(!sanitized.contains("--"));
    }

    #[test]
    fn test_generate_filename() {
        let date = "2026-03-24";
        let slug = "sunset-at-beach";
        
        let filename = generate_filename(date, slug);
        
        // Should match pattern: {date}-{slug}-{uuid4}.jpg
        assert!(filename.starts_with("2026-03-24-"));
        assert!(filename.contains("sunset-at-beach"));
        assert!(filename.ends_with(".jpg"));
        
        // UUID suffix should be 4 hex chars
        let parts: Vec<&str> = filename.rsplitn(2, '-').collect();
        let uuid_part = parts[0].strip_suffix(".jpg").unwrap();
        assert_eq!(uuid_part.len(), 4);
        assert!(uuid_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_filename_with_slashes_in_date() {
        let filename = generate_filename("2026/03/24", "sunset");
        assert!(!filename.contains('/'), "filename should not contain forward slashes");
        assert!(!filename.contains('\\'), "filename should not contain backslashes");
        assert!(filename.starts_with("2026-03-24-"), "slashes should be replaced with dashes");
    }
}
