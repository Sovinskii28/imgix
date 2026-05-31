use crate::{
    errors::{AppError, AppResult},
    state::AppState,
    utils::{image as image_utils, storage},
};

use axum::{
    Json,
    extract::{Multipart, State},
};
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
    pub output_format: String,
    pub output_extension: String,
    pub avatar_path: Option<String>,
    pub avatar_relative_path: Option<String>,
}

pub async fn compress(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<Json<CompressResponse>> {
    let mut target_path: Option<String> = None;
    tracing::info!(stage = "request", "Image compression request started");

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|source| AppError::Internal {
            source: source.into(),
        })?
    {
        if field.name() == Some("target_path") {
            target_path = Some(field.text().await.map_err(|source| AppError::Internal {
                source: source.into(),
            })?);
            tracing::info!(
                stage = "request",
                target_path = target_path.as_deref().unwrap_or(""),
                "Avatar target path received"
            );
            continue;
        }

        if field.name() != Some("file") {
            tracing::debug!(
                stage = "request",
                field_name = field.name().unwrap_or("unknown"),
                "Skipping non-file multipart field"
            );
            continue;
        }

        let extension = file_extension_or_default(field.file_name());
        let bytes = field.bytes().await.map_err(|source| AppError::Internal {
            source: source.into(),
        })?;

        tracing::info!(
            stage = "receive",
            size_bytes = bytes.len(),
            extension,
            "Image payload received"
        );

        if bytes.len() > MAX_FILE_SIZE {
            tracing::warn!(
                stage = "validate",
                size_bytes = bytes.len(),
                max_size_bytes = MAX_FILE_SIZE,
                "Image rejected because it exceeds the max size"
            );
            return Err(AppError::FileTooLarge);
        }

        tracing::info!(
            stage = "queue",
            max_concurrent = crate::state::MAX_CONCURRENT_COMPRESSIONS,
            "Waiting for compression slot"
        );
        let _permit = state
            .compression_permits
            .acquire_owned()
            .await
            .map_err(|source| AppError::Internal {
                source: source.into(),
            })?;
        tracing::info!(stage = "queue", "Compression slot acquired");

        let compression_bytes = bytes.clone();
        let (original_width, original_height, compressed_image) =
            tokio::task::spawn_blocking(move || -> AppResult<_> {
                tracing::info!(stage = "decode", "Decoding image and validating format");
                let image = image::load_from_memory(&compression_bytes)
                    .map_err(|_| AppError::InvalidImageFile)?;
                let original_width = image.width();
                let original_height = image.height();
                tracing::info!(
                    stage = "decode",
                    original_width,
                    original_height,
                    "Image decoded successfully"
                );
                let compressed_image =
                    image_utils::compress_jpeg_or_keep_original(image, &compression_bytes)
                        .map_err(|source| AppError::FailedToSaveCompressedImage { source })?;

                Ok((original_width, original_height, compressed_image))
            })
            .await
            .map_err(|source| AppError::Internal {
                source: source.into(),
            })??;

        let file_id = storage::generate_file_id();
        tracing::info!(
            stage = "save",
            file_id = %file_id,
            "Saving original and compressed images"
        );
        let original_file = storage::save_original_image(&bytes, &extension, file_id).await?;
        let compressed_file = storage::save_compressed_image(
            &compressed_image.bytes,
            file_id,
            compressed_image.extension,
        )
        .await?;
        let avatar_file = if let Some(target_path) = target_path.as_deref() {
            Some(
                storage::save_avatar_image(
                    &compressed_image.bytes,
                    target_path,
                    compressed_image.extension,
                )
                .await?,
            )
        } else {
            None
        };
        let original_path = original_file.path.to_string_lossy().into_owned();
        let compressed_path = compressed_file.path.to_string_lossy().into_owned();
        let avatar_path = avatar_file
            .as_ref()
            .map(|file| file.path.to_string_lossy().into_owned());
        let avatar_relative_path = target_path
            .as_deref()
            .map(|path| avatar_relative_path(path, compressed_image.extension));

        tracing::info!(
            stage = "save",
            original_path,
            "Original image saved for audit/debugging"
        );
        tracing::info!(
            stage = "save",
            compressed_path,
            "Compressed clean image saved"
        );
        if let Some(path) = avatar_path.as_deref() {
            tracing::info!(stage = "save", avatar_path = path, "Avatar file saved");
        }
        tracing::info!(
            stage = "complete",
            original_width,
            original_height,
            compressed_width = compressed_image.width,
            compressed_height = compressed_image.height,
            output_format = compressed_image.format,
            output_extension = compressed_image.extension,
            original_size_bytes = bytes.len(),
            compressed_size_bytes = compressed_image.bytes.len(),
            saved_percent = compression_saved_percent(bytes.len(), compressed_image.bytes.len()),
            "Image compression request completed"
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
            output_format: compressed_image.format.to_string(),
            output_extension: compressed_image.extension.to_string(),
            avatar_path,
            avatar_relative_path,
        }));
    }

    Err(AppError::NoFileProvided)
}

fn compression_saved_percent(original_size: usize, compressed_size: usize) -> i64 {
    if original_size == 0 {
        return 0;
    }

    100 - ((compressed_size as i64 * 100) / original_size as i64)
}

fn avatar_relative_path(path: &str, extension: &str) -> String {
    let normalized = path.replace('\\', "/");
    let clean = normalized.trim_start_matches('/');
    let mut path = Path::new(clean).to_path_buf();
    path.set_extension(extension.trim_start_matches('.'));
    path.to_string_lossy().replace('\\', "/")
}

fn file_extension_or_default(file_name: Option<&str>) -> String {
    file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .unwrap_or("jpg")
        .to_ascii_lowercase()
}
