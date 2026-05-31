use image::{
    DynamicImage, ImageEncoder, ImageFormat, codecs::jpeg::JpegEncoder, imageops::FilterType,
};
use std::{
    env, fs,
    io::Write,
    process::{Command, Stdio},
};
use uuid::Uuid;

pub const MAX_COMPRESSED_WIDTH: u32 = 1600;
pub const JPEG_QUALITY: u8 = 92;
const LARGE_IMAGE_SIZE_BYTES: usize = 1_000_000;
const LARGE_IMAGE_WIDTH: u32 = 2_400;
const SAFE_JPEG_QUALITY_CANDIDATES: &[u8] = &[95, 92, 90, 88, 85];
const AGGRESSIVE_JPEG_QUALITY_CANDIDATES: &[u8] = &[80];
const SAFE_MIN_ACCEPTABLE_PSNR_DB: f64 = 42.0;
const AGGRESSIVE_MIN_ACCEPTABLE_PSNR_DB: f64 = 34.0;

struct CompressionProfile {
    quality_candidates: &'static [u8],
    min_acceptable_psnr_db: f64,
    prefer_resize_over_lossless: bool,
    accept_smaller_candidate_without_psnr: bool,
}

pub struct CompressedImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn resize_for_compression(image: DynamicImage) -> DynamicImage {
    let width = image.width();
    let height = image.height();

    if width <= MAX_COMPRESSED_WIDTH {
        return image;
    }

    let resized_height = ((height as u64 * MAX_COMPRESSED_WIDTH as u64) / width as u64) as u32;

    image.resize_exact(MAX_COMPRESSED_WIDTH, resized_height, FilterType::Lanczos3)
}

pub fn compress_jpeg_or_keep_original(
    image: DynamicImage,
    original_bytes: &[u8],
) -> anyhow::Result<CompressedImage> {
    let original_width = image.width();
    let original_height = image.height();
    let profile = compression_profile(original_bytes.len(), original_width);
    let lossless_jpeg = if image::guess_format(original_bytes).ok() == Some(ImageFormat::Jpeg) {
        optimize_jpeg_losslessly(original_bytes)
    } else {
        None
    };

    if !profile.prefer_resize_over_lossless {
        if let Some(bytes) = lossless_jpeg {
            return Ok(CompressedImage {
                bytes,
                width: original_width,
                height: original_height,
            });
        }
    }

    let resized_image = resize_for_compression(image);
    let compressed_width = resized_image.width();
    let compressed_height = resized_image.height();
    let comparison_image = resized_image.to_rgb8();
    let mut best_candidate = None;

    for quality in profile.quality_candidates {
        let compressed_bytes = encode_jpeg_with_quality(&resized_image, *quality)?;

        if compressed_bytes.len() >= original_bytes.len() {
            continue;
        }

        let decoded_candidate = image::load_from_memory(&compressed_bytes)?;
        let candidate_psnr = psnr_db(&comparison_image, &decoded_candidate.to_rgb8());

        if profile.accept_smaller_candidate_without_psnr
            || candidate_psnr >= profile.min_acceptable_psnr_db
        {
            best_candidate = Some(compressed_bytes);
        }
    }

    if let Some(bytes) = best_candidate {
        return Ok(CompressedImage {
            bytes,
            width: compressed_width,
            height: compressed_height,
        });
    }

    if let Some(bytes) = lossless_jpeg {
        return Ok(CompressedImage {
            bytes,
            width: original_width,
            height: original_height,
        });
    }

    if image::guess_format(original_bytes).ok() == Some(ImageFormat::Jpeg) {
        return Ok(CompressedImage {
            bytes: original_bytes.to_vec(),
            width: original_width,
            height: original_height,
        });
    }

    let compressed_bytes = encode_jpeg_with_quality(&resized_image, JPEG_QUALITY)?;
    Ok(CompressedImage {
        bytes: compressed_bytes,
        width: compressed_width,
        height: compressed_height,
    })
}

fn compression_profile(original_size: usize, original_width: u32) -> CompressionProfile {
    if original_size >= LARGE_IMAGE_SIZE_BYTES || original_width > LARGE_IMAGE_WIDTH {
        CompressionProfile {
            quality_candidates: AGGRESSIVE_JPEG_QUALITY_CANDIDATES,
            min_acceptable_psnr_db: AGGRESSIVE_MIN_ACCEPTABLE_PSNR_DB,
            prefer_resize_over_lossless: true,
            accept_smaller_candidate_without_psnr: true,
        }
    } else {
        CompressionProfile {
            quality_candidates: SAFE_JPEG_QUALITY_CANDIDATES,
            min_acceptable_psnr_db: SAFE_MIN_ACCEPTABLE_PSNR_DB,
            prefer_resize_over_lossless: false,
            accept_smaller_candidate_without_psnr: false,
        }
    }
}

fn encode_jpeg_with_quality(image: &DynamicImage, quality: u8) -> anyhow::Result<Vec<u8>> {
    if let Some(bytes) = encode_jpeg_with_cjpeg(image, quality) {
        return Ok(bytes);
    }

    let rgb_image = image.to_rgb8();
    let mut bytes = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut bytes, quality);

    encoder.write_image(
        rgb_image.as_raw(),
        rgb_image.width(),
        rgb_image.height(),
        image::ExtendedColorType::Rgb8,
    )?;

    Ok(bytes)
}

fn optimize_jpeg_losslessly(original_bytes: &[u8]) -> Option<Vec<u8>> {
    let file_id = Uuid::new_v4();
    let input_path = env::temp_dir().join(format!("imgx-{file_id}-input.jpg"));
    let output_path = env::temp_dir().join(format!("imgx-{file_id}-output.jpg"));

    let result = (|| {
        fs::write(&input_path, original_bytes).ok()?;

        let status = Command::new("jpegtran")
            .arg("-copy")
            .arg("none")
            .arg("-optimize")
            .arg("-progressive")
            .arg("-outfile")
            .arg(&output_path)
            .arg(&input_path)
            .status()
            .ok()?;

        if !status.success() {
            return None;
        }

        let bytes = fs::read(&output_path).ok()?;
        (bytes.len() <= original_bytes.len()).then_some(bytes)
    })();

    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);

    result
}

fn encode_jpeg_with_cjpeg(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let rgb_image = image.to_rgb8();
    let mut child = Command::new("cjpeg")
        .arg("-quality")
        .arg(quality.to_string())
        .arg("-optimize")
        .arg("-progressive")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    {
        let stdin = child.stdin.as_mut()?;
        writeln!(stdin, "P6").ok()?;
        writeln!(stdin, "{} {}", rgb_image.width(), rgb_image.height()).ok()?;
        writeln!(stdin, "255").ok()?;
        stdin.write_all(rgb_image.as_raw()).ok()?;
    }

    let output = child.wait_with_output().ok()?;

    if output.status.success() {
        Some(output.stdout)
    } else {
        None
    }
}

fn psnr_db(original: &image::RgbImage, compressed: &image::RgbImage) -> f64 {
    if original.dimensions() != compressed.dimensions() {
        return 0.0;
    }

    let squared_error_sum = original
        .as_raw()
        .iter()
        .zip(compressed.as_raw())
        .map(|(original, compressed)| {
            let diff = f64::from(*original) - f64::from(*compressed);
            diff * diff
        })
        .sum::<f64>();

    if squared_error_sum == 0.0 {
        return f64::INFINITY;
    }

    let mean_squared_error = squared_error_sum / original.as_raw().len() as f64;

    20.0 * (255.0 / mean_squared_error.sqrt()).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn test_image(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_pixel(width, height, Rgb([0, 0, 0])))
    }

    #[test]
    fn does_not_upscale_images_at_or_below_max_width() {
        let image = resize_for_compression(test_image(800, 600));

        assert_eq!(image.width(), 800);
        assert_eq!(image.height(), 600);
    }

    #[test]
    fn resizes_large_images_preserving_aspect_ratio() {
        let image = resize_for_compression(test_image(4000, 3000));

        assert_eq!(image.width(), 1600);
        assert_eq!(image.height(), 1200);
    }

    #[test]
    fn encodes_resized_image_as_jpeg() {
        let image = test_image(10, 10);
        let bytes =
            encode_jpeg_with_quality(&image, JPEG_QUALITY).expect("image should encode as jpeg");

        assert!(bytes.starts_with(&[0xFF, 0xD8]));
        assert!(bytes.ends_with(&[0xFF, 0xD9]));
    }

    #[test]
    fn does_not_make_jpeg_larger_or_visually_worse() {
        let image = test_image(20, 20);
        let original_bytes =
            encode_jpeg_with_quality(&image, 10).expect("image should encode as jpeg");
        let decoded = image::load_from_memory(&original_bytes).expect("jpeg should decode");

        let compressed = compress_jpeg_or_keep_original(decoded, &original_bytes)
            .expect("image should compress");

        let compressed_decoded =
            image::load_from_memory(&compressed.bytes).expect("compressed jpeg should decode");

        assert!(compressed.bytes.len() <= original_bytes.len());
        assert_eq!(compressed_decoded.to_rgb8(), image.to_rgb8());
        assert_eq!(compressed.width, 20);
        assert_eq!(compressed.height, 20);
    }

    #[test]
    fn accepts_only_candidates_with_enough_visual_quality() {
        let image = test_image(20, 20);
        let original = image.to_rgb8();
        let encoded =
            encode_jpeg_with_quality(&image, JPEG_QUALITY).expect("image should encode as jpeg");
        let decoded = image::load_from_memory(&encoded).expect("jpeg should decode");

        assert!(psnr_db(&original, &decoded.to_rgb8()) >= SAFE_MIN_ACCEPTABLE_PSNR_DB);
    }

    #[test]
    fn uses_aggressive_profile_for_large_images() {
        let profile = compression_profile(LARGE_IMAGE_SIZE_BYTES, 800);

        assert_eq!(
            profile.quality_candidates,
            AGGRESSIVE_JPEG_QUALITY_CANDIDATES
        );
        assert!(profile.prefer_resize_over_lossless);
        assert!(profile.accept_smaller_candidate_without_psnr);
    }

    #[test]
    fn uses_safe_profile_for_small_images() {
        let profile = compression_profile(200_000, 800);

        assert_eq!(profile.quality_candidates, SAFE_JPEG_QUALITY_CANDIDATES);
        assert!(!profile.prefer_resize_over_lossless);
        assert!(!profile.accept_smaller_candidate_without_psnr);
    }
}
