use image::{
    DynamicImage, ImageEncoder, ImageFormat, codecs::jpeg::JpegEncoder, imageops::FilterType,
};
use std::{
    env, fs,
    io::Write,
    process::{Command, Stdio},
};
use uuid::Uuid;
use webp::Encoder as LossyWebPEncoder;

pub const MAX_COMPRESSED_WIDTH: u32 = 1600;
const LARGE_IMAGE_SIZE_BYTES: usize = 1_000_000;
const LARGE_IMAGE_WIDTH: u32 = 2_400;
const SAFE_JPEG_QUALITY_CANDIDATES: &[u8] = &[95, 92, 90, 88, 85, 82, 78, 74, 70, 64, 58, 52, 46];
const AGGRESSIVE_JPEG_QUALITY_CANDIDATES: &[u8] = &[82, 78, 74, 70, 66, 62, 58, 52, 46, 40, 34];
const WEBP_QUALITY_CANDIDATES: &[u8] = &[88, 82, 76, 70, 64, 58, 52, 46, 40, 34, 28];
const SAFE_MIN_ACCEPTABLE_PSNR_DB: f64 = 42.0;
const AGGRESSIVE_MIN_ACCEPTABLE_PSNR_DB: f64 = 34.0;
const MIN_ACCEPTABLE_FALLBACK_PSNR_DB: f64 = 30.0;

struct CompressionProfile {
    name: &'static str,
    quality_candidates: &'static [u8],
    min_acceptable_psnr_db: f64,
    prefer_resize_over_lossless: bool,
    accept_smaller_candidate_without_psnr: bool,
}

pub struct CompressedImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub extension: &'static str,
    pub format: &'static str,
}

struct EncodedCandidate {
    bytes: Vec<u8>,
    extension: &'static str,
    format: &'static str,
    quality: Option<u8>,
    psnr_db: Option<f64>,
    encoder: &'static str,
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
    tracing::info!(
        stage = "compress",
        profile = profile.name,
        original_size_bytes = original_bytes.len(),
        original_width,
        original_height,
        max_compressed_width = MAX_COMPRESSED_WIDTH,
        "Compression profile selected"
    );

    let lossless_jpeg = if image::guess_format(original_bytes).ok() == Some(ImageFormat::Jpeg) {
        tracing::info!(
            stage = "metadata_clean",
            method = "jpegtran",
            "Trying lossless JPEG optimization and metadata stripping"
        );
        optimize_jpeg_losslessly(original_bytes)
    } else {
        tracing::info!(
            stage = "metadata_clean",
            method = "reencode",
            "Source is not JPEG, metadata will be stripped by clean JPEG re-encode"
        );
        None
    };

    if !profile.prefer_resize_over_lossless {
        if let Some(bytes) = lossless_jpeg {
            tracing::info!(
                stage = "metadata_clean",
                method = "jpegtran",
                compressed_size_bytes = bytes.len(),
                "Metadata stripped with lossless JPEG optimization"
            );
            return Ok(CompressedImage {
                bytes,
                width: original_width,
                height: original_height,
                extension: "jpg",
                format: "jpeg",
            });
        }
    }

    let resized_image = resize_for_compression(image);
    let compressed_width = resized_image.width();
    let compressed_height = resized_image.height();
    if compressed_width != original_width || compressed_height != original_height {
        tracing::info!(
            stage = "resize",
            original_width,
            original_height,
            compressed_width,
            compressed_height,
            "Image resized before compression"
        );
    } else {
        tracing::info!(
            stage = "resize",
            width = compressed_width,
            height = compressed_height,
            "Image kept at original dimensions"
        );
    }

    let comparison_image = resized_image.to_rgb8();
    let mut best_candidate: Option<EncodedCandidate> = None;
    let mut smallest_clean_candidate: Option<EncodedCandidate> = None;

    for quality in profile.quality_candidates {
        let compressed_candidate = encode_jpeg_with_quality(&resized_image, *quality)?;
        tracing::info!(
            stage = "compress",
            quality,
            format = compressed_candidate.format,
            encoder = compressed_candidate.encoder,
            candidate_size_bytes = compressed_candidate.bytes.len(),
            "Clean JPEG candidate encoded"
        );

        let decoded_candidate = image::load_from_memory(&compressed_candidate.bytes)?;
        let candidate_psnr = psnr_db(&comparison_image, &decoded_candidate.to_rgb8());
        let compressed_candidate = EncodedCandidate {
            psnr_db: Some(candidate_psnr),
            ..compressed_candidate
        };
        tracing::info!(
            stage = "quality_check",
            quality,
            candidate_psnr_db = candidate_psnr,
            min_acceptable_psnr_db = profile.min_acceptable_psnr_db,
            "Candidate visual quality measured"
        );

        if compressed_candidate.bytes.len() < original_bytes.len()
            && (profile.accept_smaller_candidate_without_psnr
                || candidate_psnr >= profile.min_acceptable_psnr_db)
        {
            tracing::info!(
                stage = "compress",
                quality,
                format = compressed_candidate.format,
                encoder = compressed_candidate.encoder,
                candidate_size_bytes = compressed_candidate.bytes.len(),
                "Candidate accepted"
            );
            remember_smallest_clean_candidate(
                &mut best_candidate,
                compressed_candidate.clone_candidate(),
            );
        }

        remember_smallest_clean_candidate(&mut smallest_clean_candidate, compressed_candidate);
    }

    for quality in WEBP_QUALITY_CANDIDATES {
        if let Some(webp_candidate) = encode_webp_with_quality(&resized_image, *quality) {
            tracing::info!(
                stage = "webp",
                quality,
                encoder = webp_candidate.encoder,
                candidate_size_bytes = webp_candidate.bytes.len(),
                "Clean WebP candidate encoded"
            );
            let decoded_candidate = image::load_from_memory(&webp_candidate.bytes)?;
            let candidate_psnr = psnr_db(&comparison_image, &decoded_candidate.to_rgb8());
            let webp_candidate = EncodedCandidate {
                psnr_db: Some(candidate_psnr),
                ..webp_candidate
            };
            tracing::info!(
                stage = "quality_check",
                format = "webp",
                quality,
                candidate_psnr_db = candidate_psnr,
                min_acceptable_psnr_db = profile.min_acceptable_psnr_db,
                "WebP visual quality measured"
            );

            if webp_candidate.bytes.len() < original_bytes.len()
                && (profile.accept_smaller_candidate_without_psnr
                    || candidate_psnr >= profile.min_acceptable_psnr_db)
            {
                tracing::info!(
                    stage = "webp",
                    quality,
                    encoder = webp_candidate.encoder,
                    candidate_size_bytes = webp_candidate.bytes.len(),
                    "WebP candidate accepted"
                );
                remember_smallest_clean_candidate(
                    &mut best_candidate,
                    webp_candidate.clone_candidate(),
                );
            }

            remember_smallest_clean_candidate(&mut smallest_clean_candidate, webp_candidate);
        } else {
            tracing::info!(
                stage = "webp",
                quality,
                "WebP encoder failed; skipping this WebP candidate"
            );
        }
    }

    if best_candidate.is_none() {
        for quality in [40, 34, 28, 24, 20, 16] {
            let compressed_candidate = encode_jpeg_with_quality(&resized_image, quality)?;
            let decoded_candidate = image::load_from_memory(&compressed_candidate.bytes)?;
            let candidate_psnr = psnr_db(&comparison_image, &decoded_candidate.to_rgb8());
            let compressed_candidate = EncodedCandidate {
                psnr_db: Some(candidate_psnr),
                ..compressed_candidate
            };
            remember_smallest_clean_candidate(
                &mut smallest_clean_candidate,
                compressed_candidate.clone_candidate(),
            );

            if compressed_candidate.bytes.len() < original_bytes.len()
                && candidate_psnr >= MIN_ACCEPTABLE_FALLBACK_PSNR_DB
            {
                tracing::warn!(
                    stage = "compress",
                    format = compressed_candidate.format,
                    quality,
                    encoder = compressed_candidate.encoder,
                    candidate_size_bytes = compressed_candidate.bytes.len(),
                    candidate_psnr_db = candidate_psnr,
                    min_acceptable_psnr_db = MIN_ACCEPTABLE_FALLBACK_PSNR_DB,
                    "Emergency JPEG compression accepted to keep output smaller than source"
                );
                remember_smallest_clean_candidate(&mut best_candidate, compressed_candidate);
            }
        }
    }

    if best_candidate.is_none() {
        for quality in [30, 24, 18, 12] {
            if let Some(webp_candidate) = encode_webp_with_quality(&resized_image, quality) {
                let decoded_candidate = image::load_from_memory(&webp_candidate.bytes)?;
                let candidate_psnr = psnr_db(&comparison_image, &decoded_candidate.to_rgb8());
                let webp_candidate = EncodedCandidate {
                    psnr_db: Some(candidate_psnr),
                    ..webp_candidate
                };
                remember_smallest_clean_candidate(
                    &mut smallest_clean_candidate,
                    webp_candidate.clone_candidate(),
                );

                if webp_candidate.bytes.len() < original_bytes.len()
                    && candidate_psnr >= MIN_ACCEPTABLE_FALLBACK_PSNR_DB
                {
                    tracing::warn!(
                        stage = "webp",
                        quality,
                        encoder = webp_candidate.encoder,
                        candidate_size_bytes = webp_candidate.bytes.len(),
                        candidate_psnr_db = candidate_psnr,
                        min_acceptable_psnr_db = MIN_ACCEPTABLE_FALLBACK_PSNR_DB,
                        "Emergency WebP compression accepted to keep output smaller than source"
                    );
                    remember_smallest_clean_candidate(&mut best_candidate, webp_candidate);
                }
            }
        }
    }

    if best_candidate.is_none() {
        if let Some(candidate) = smallest_clean_candidate
            .as_ref()
            .filter(|candidate| candidate.bytes.len() < original_bytes.len())
            .filter(|candidate| candidate.psnr_db.unwrap_or(0.0) >= MIN_ACCEPTABLE_FALLBACK_PSNR_DB)
        {
            tracing::warn!(
                stage = "compress",
                format = candidate.format,
                quality = candidate.quality.unwrap_or_default(),
                candidate_psnr_db = candidate.psnr_db.unwrap_or_default(),
                min_acceptable_psnr_db = MIN_ACCEPTABLE_FALLBACK_PSNR_DB,
                "Using strongest clean candidate to guarantee the output is smaller than source"
            );
            best_candidate = Some(candidate.clone_candidate());
        }
    }

    if let Some(candidate) = best_candidate {
        tracing::info!(
            stage = "metadata_clean",
            method = "clean_reencode",
            format = candidate.format,
            encoder = candidate.encoder,
            compressed_size_bytes = candidate.bytes.len(),
            "Metadata stripped by best clean candidate"
        );
        return Ok(CompressedImage {
            bytes: candidate.bytes,
            width: compressed_width,
            height: compressed_height,
            extension: candidate.extension,
            format: candidate.format,
        });
    }

    if let Some(bytes) = lossless_jpeg.filter(|bytes| bytes.len() <= original_bytes.len()) {
        tracing::info!(
            stage = "metadata_clean",
            method = "jpegtran",
            compressed_size_bytes = bytes.len(),
            "No better candidate selected; using metadata-stripped lossless JPEG"
        );
        return Ok(CompressedImage {
            bytes,
            width: original_width,
            height: original_height,
            extension: "jpg",
            format: "jpeg",
        });
    }

    tracing::warn!(
        stage = "compress",
        original_size_bytes = original_bytes.len(),
        smallest_clean_size_bytes = smallest_clean_candidate
            .as_ref()
            .map(|candidate| candidate.bytes.len())
            .unwrap_or(0),
        "Source is already smaller than every clean candidate; keeping original bytes to avoid file growth"
    );
    let (extension, format) = source_format_for_response(original_bytes);
    return Ok(CompressedImage {
        bytes: original_bytes.to_vec(),
        width: original_width,
        height: original_height,
        extension,
        format,
    });
}

fn compression_profile(original_size: usize, original_width: u32) -> CompressionProfile {
    if original_size >= LARGE_IMAGE_SIZE_BYTES || original_width > LARGE_IMAGE_WIDTH {
        CompressionProfile {
            name: "aggressive",
            quality_candidates: AGGRESSIVE_JPEG_QUALITY_CANDIDATES,
            min_acceptable_psnr_db: AGGRESSIVE_MIN_ACCEPTABLE_PSNR_DB,
            prefer_resize_over_lossless: true,
            accept_smaller_candidate_without_psnr: true,
        }
    } else {
        CompressionProfile {
            name: "safe",
            quality_candidates: SAFE_JPEG_QUALITY_CANDIDATES,
            min_acceptable_psnr_db: SAFE_MIN_ACCEPTABLE_PSNR_DB,
            prefer_resize_over_lossless: false,
            accept_smaller_candidate_without_psnr: false,
        }
    }
}

fn encode_jpeg_with_quality(image: &DynamicImage, quality: u8) -> anyhow::Result<EncodedCandidate> {
    if let Some(bytes) = encode_jpeg_with_cjpeg(image, quality) {
        tracing::info!(
            stage = "encode",
            encoder = "cjpeg",
            quality,
            size_bytes = bytes.len(),
            "JPEG encoded with external cjpeg"
        );
        return Ok(EncodedCandidate::new(
            bytes,
            "jpg",
            "jpeg",
            Some(quality),
            "cjpeg",
        ));
    }

    tracing::info!(
        stage = "encode",
        encoder = "rust_image",
        quality,
        "cjpeg unavailable; using Rust JPEG encoder"
    );
    let rgb_image = image.to_rgb8();
    let mut bytes = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut bytes, quality);

    encoder.write_image(
        rgb_image.as_raw(),
        rgb_image.width(),
        rgb_image.height(),
        image::ExtendedColorType::Rgb8,
    )?;

    Ok(EncodedCandidate::new(
        bytes,
        "jpg",
        "jpeg",
        Some(quality),
        "rust_image",
    ))
}

fn encode_webp_with_quality(image: &DynamicImage, quality: u8) -> Option<EncodedCandidate> {
    if let Some(bytes) = encode_webp_with_cwebp(image, quality) {
        return Some(EncodedCandidate::new(
            bytes,
            "webp",
            "webp",
            Some(quality),
            "cwebp",
        ));
    }

    let rgb_image = image.to_rgb8();
    let encoder =
        LossyWebPEncoder::from_rgb(rgb_image.as_raw(), rgb_image.width(), rgb_image.height());
    let bytes = encoder.encode(f32::from(quality)).to_vec();
    Some(EncodedCandidate::new(
        bytes,
        "webp",
        "webp",
        Some(quality),
        "libwebp",
    ))
}

fn source_format_for_response(original_bytes: &[u8]) -> (&'static str, &'static str) {
    match image::guess_format(original_bytes).ok() {
        Some(ImageFormat::Png) => ("png", "png"),
        Some(ImageFormat::WebP) => ("webp", "webp"),
        Some(ImageFormat::Gif) => ("gif", "gif"),
        Some(ImageFormat::Jpeg) => ("jpg", "jpeg"),
        _ => ("jpg", "jpeg"),
    }
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
            tracing::warn!(
                stage = "metadata_clean",
                method = "jpegtran",
                "jpegtran failed; falling back to clean re-encode"
            );
            return None;
        }

        let bytes = fs::read(&output_path).ok()?;
        if bytes.len() <= original_bytes.len() {
            Some(bytes)
        } else {
            tracing::info!(
                stage = "metadata_clean",
                method = "jpegtran",
                optimized_size_bytes = bytes.len(),
                original_size_bytes = original_bytes.len(),
                "jpegtran output was larger; falling back to clean re-encode"
            );
            None
        }
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

fn encode_webp_with_cwebp(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let file_id = Uuid::new_v4();
    let input_path = env::temp_dir().join(format!("imgx-{file_id}-input.png"));
    let output_path = env::temp_dir().join(format!("imgx-{file_id}-output.webp"));

    let result = (|| {
        image.save(&input_path).ok()?;

        let status = Command::new("cwebp")
            .arg("-quiet")
            .arg("-q")
            .arg(quality.to_string())
            .arg("-metadata")
            .arg("none")
            .arg(&input_path)
            .arg("-o")
            .arg(&output_path)
            .status()
            .ok()?;

        if !status.success() {
            return None;
        }

        fs::read(&output_path).ok()
    })();

    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);

    result
}

impl EncodedCandidate {
    fn new(
        bytes: Vec<u8>,
        extension: &'static str,
        format: &'static str,
        quality: Option<u8>,
        encoder: &'static str,
    ) -> Self {
        Self {
            bytes,
            extension,
            format,
            quality,
            psnr_db: None,
            encoder,
        }
    }

    fn clone_candidate(&self) -> Self {
        Self {
            bytes: self.bytes.clone(),
            extension: self.extension,
            format: self.format,
            quality: self.quality,
            psnr_db: self.psnr_db,
            encoder: self.encoder,
        }
    }
}

fn remember_smallest_clean_candidate(
    best: &mut Option<EncodedCandidate>,
    candidate: EncodedCandidate,
) {
    if best
        .as_ref()
        .map(|best| candidate.bytes.len() < best.bytes.len())
        .unwrap_or(true)
    {
        *best = Some(candidate);
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
        let candidate = encode_jpeg_with_quality(&image, 92).expect("image should encode as jpeg");

        assert!(candidate.bytes.starts_with(&[0xFF, 0xD8]));
        assert!(candidate.bytes.ends_with(&[0xFF, 0xD9]));
        assert_eq!(candidate.extension, "jpg");
        assert_eq!(candidate.format, "jpeg");
    }

    #[test]
    fn returns_clean_jpeg_when_no_smaller_candidate_exists() {
        let image = test_image(20, 20);
        let original_candidate =
            encode_jpeg_with_quality(&image, 10).expect("image should encode as jpeg");
        let decoded =
            image::load_from_memory(&original_candidate.bytes).expect("jpeg should decode");

        let compressed = compress_jpeg_or_keep_original(decoded, &original_candidate.bytes)
            .expect("image should compress");

        let compressed_decoded =
            image::load_from_memory(&compressed.bytes).expect("compressed jpeg should decode");

        assert!(compressed.bytes.len() <= original_candidate.bytes.len());
        assert!(matches!(compressed.extension, "jpg" | "webp"));
        assert_eq!(compressed_decoded.to_rgb8(), image.to_rgb8());
        assert_eq!(compressed.width, 20);
        assert_eq!(compressed.height, 20);
    }

    #[test]
    fn encodes_webp_candidates_with_lossy_encoder() {
        let image = test_image(32, 32);
        let candidate = encode_webp_with_quality(&image, 70).expect("image should encode as webp");

        assert!(candidate.bytes.starts_with(b"RIFF"));
        assert_eq!(&candidate.bytes[8..12], b"WEBP");
        assert_eq!(candidate.extension, "webp");
        assert_eq!(candidate.format, "webp");
        assert_eq!(candidate.quality, Some(70));
    }

    #[test]
    fn accepts_only_candidates_with_enough_visual_quality() {
        let image = test_image(20, 20);
        let original = image.to_rgb8();
        let encoded = encode_jpeg_with_quality(&image, 92).expect("image should encode as jpeg");
        let decoded = image::load_from_memory(&encoded.bytes).expect("jpeg should decode");

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
