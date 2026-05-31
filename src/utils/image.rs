use image::{
    DynamicImage, ImageEncoder, ImageFormat, codecs::jpeg::JpegEncoder, imageops::FilterType,
};

pub const MAX_COMPRESSED_WIDTH: u32 = 1600;
pub const JPEG_QUALITY: u8 = 80;
const JPEG_QUALITY_CANDIDATES: &[u8] = &[80, 70, 60, 50];

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
    let resized_image = resize_for_compression(image);
    let compressed_width = resized_image.width();
    let compressed_height = resized_image.height();

    for quality in JPEG_QUALITY_CANDIDATES {
        let compressed_bytes = encode_jpeg_with_quality(&resized_image, *quality)?;

        if compressed_bytes.len() < original_bytes.len() {
            return Ok(CompressedImage {
                bytes: compressed_bytes,
                width: compressed_width,
                height: compressed_height,
            });
        }
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

fn encode_jpeg_with_quality(image: &DynamicImage, quality: u8) -> anyhow::Result<Vec<u8>> {
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
    fn keeps_original_jpeg_when_reencoding_would_be_larger() {
        let image = test_image(20, 20);
        let original_bytes =
            encode_jpeg_with_quality(&image, 10).expect("image should encode as jpeg");
        let decoded = image::load_from_memory(&original_bytes).expect("jpeg should decode");

        let compressed = compress_jpeg_or_keep_original(decoded, &original_bytes)
            .expect("image should compress");

        assert_eq!(compressed.bytes, original_bytes);
        assert_eq!(compressed.width, 20);
        assert_eq!(compressed.height, 20);
    }
}
