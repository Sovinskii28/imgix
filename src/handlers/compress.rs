use crate::{
    errors::{AppError, AppResult},
    utils::{image as image_utils, storage},
};

use axum::{Json, extract::Multipart};
use serde::Serialize;
use std::path::Path;

pub const MAX_FILE_SIZE: usize = 32 * 1024 * 1024;

#[derive(Serialize)]
pub struct CompressResponse {
    pub original_file_name: String,
    pub original_path: String,
    pub compressed_file_name: String,
    pub compressed_path: String,
    pub original_size: usize,
    pub compressed_size: usize,
    pub original_width: u32,
    pub original_height: u32,
    pub compressed_width: u32,
    pub compressed_height: u32,
}

pub async fn compress(mut multipart: Multipart) -> AppResult<Json<CompressResponse>> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|source| AppError::Internal {
            source: source.into(),
        })?
    {
        if field.name() != Some("file") {
            continue;
        }

        let extension = file_extension_or_default(field.file_name());
        let bytes = field.bytes().await.map_err(|source| AppError::Internal {
            source: source.into(),
        })?;

        if bytes.len() > MAX_FILE_SIZE {
            return Err(AppError::FileTooLarge);
        }

        tracing::info!("Image received: {} bytes", bytes.len());

        let image = image::load_from_memory(&bytes).map_err(|_| AppError::InvalidImageFile)?;
        let original_width = image.width();
        let original_height = image.height();
        let compressed_image = image_utils::compress_jpeg_or_keep_original(image, &bytes)
            .map_err(|source| AppError::FailedToSaveCompressedImage { source })?;

        let file_id = storage::generate_file_id();
        let original_file = storage::save_original_image(&bytes, &extension, file_id).await?;
        let compressed_file =
            storage::save_compressed_image(&compressed_image.bytes, file_id).await?;
        let original_path = original_file.path.to_string_lossy().into_owned();
        let compressed_path = compressed_file.path.to_string_lossy().into_owned();

        tracing::info!("Original saved: {}", original_path);
        tracing::info!("Compressed saved: {}", compressed_path);
        tracing::info!(
            original_width,
            original_height,
            compressed_width = compressed_image.width,
            compressed_height = compressed_image.height,
            "Image compressed: {} -> {} bytes",
            bytes.len(),
            compressed_image.bytes.len()
        );

        return Ok(Json(CompressResponse {
            original_file_name: original_file.file_name,
            original_path,
            compressed_file_name: compressed_file.file_name,
            compressed_path,
            original_size: bytes.len(),
            compressed_size: compressed_image.bytes.len(),
            original_width,
            original_height,
            compressed_width: compressed_image.width,
            compressed_height: compressed_image.height,
        }));
    }

    Err(AppError::NoFileProvided)
}

fn file_extension_or_default(file_name: Option<&str>) -> String {
    file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .unwrap_or("jpg")
        .to_ascii_lowercase()
}
