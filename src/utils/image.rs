use image::{DynamicImage, ImageEncoder, codecs::jpeg::JpegEncoder, imageops::FilterType};

pub const MAX_COMPRESSED_WIDTH: u32 = 1600;
pub const JPEG_QUALITY: u8 = 80;

pub fn resize_for_compression(image: DynamicImage) -> DynamicImage {
    let width = image.width();
    let height = image.height();

    if width <= MAX_COMPRESSED_WIDTH {
        return image;
    }

    let resized_height = ((height as u64 * MAX_COMPRESSED_WIDTH as u64) / width as u64) as u32;

    image.resize_exact(MAX_COMPRESSED_WIDTH, resized_height, FilterType::Lanczos3)
}

pub fn encode_jpeg(image: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let rgb_image = image.to_rgb8();
    let mut bytes = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);

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
        let bytes = encode_jpeg(&image).expect("image should encode as jpeg");

        assert!(bytes.starts_with(&[0xFF, 0xD8]));
        assert!(bytes.ends_with(&[0xFF, 0xD9]));
    }
}
