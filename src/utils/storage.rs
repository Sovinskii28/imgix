use crate::errors::{AppError, AppResult};

use std::{
    env,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const ORIGINALS_DIR: &str = "uploads/originals";
pub const COMPRESSED_DIR: &str = "uploads/compressed";

pub struct StoredFile {
    pub file_name: String,
    pub path: PathBuf,
}

pub async fn ensure_upload_dirs() -> AppResult<()> {
    tokio::fs::create_dir_all(Path::new(ORIGINALS_DIR))
        .await
        .map_err(|source| AppError::FailedToCreateUploadsDirectory { source })?;
    tokio::fs::create_dir_all(Path::new(COMPRESSED_DIR))
        .await
        .map_err(|source| AppError::FailedToCreateUploadsDirectory { source })?;

    tracing::info!(
        stage = "startup",
        originals_dir = ORIGINALS_DIR,
        compressed_dir = COMPRESSED_DIR,
        avatar_uploads_dir = %avatar_uploads_dir().to_string_lossy(),
        "Image storage directories are ready"
    );

    Ok(())
}

pub fn generate_file_id() -> Uuid {
    Uuid::new_v4()
}

pub async fn save_original_image(
    bytes: &[u8],
    extension: &str,
    file_id: Uuid,
) -> AppResult<StoredFile> {
    tokio::fs::create_dir_all(Path::new(ORIGINALS_DIR))
        .await
        .map_err(|source| AppError::FailedToCreateUploadsDirectory { source })?;

    let file_name = format!("{}_original.{}", file_id, extension);
    let path = Path::new(ORIGINALS_DIR).join(&file_name);
    tracing::info!(
        stage = "save",
        kind = "original",
        path = %path.to_string_lossy(),
        size_bytes = bytes.len(),
        "Writing original image"
    );

    tokio::fs::write(&path, bytes)
        .await
        .map_err(|source| AppError::FailedToSaveOriginalImage { source })?;

    Ok(StoredFile { file_name, path })
}

pub async fn save_compressed_image(
    bytes: &[u8],
    file_id: Uuid,
    extension: &str,
) -> AppResult<StoredFile> {
    tokio::fs::create_dir_all(Path::new(COMPRESSED_DIR))
        .await
        .map_err(|source| AppError::FailedToCreateUploadsDirectory { source })?;

    let file_name = format!("{}_compressed.{}", file_id, extension);
    let path = Path::new(COMPRESSED_DIR).join(&file_name);
    tracing::info!(
        stage = "save",
        kind = "compressed",
        path = %path.to_string_lossy(),
        size_bytes = bytes.len(),
        "Writing compressed clean image"
    );

    tokio::fs::write(&path, bytes).await.map_err(|source| {
        AppError::FailedToSaveCompressedImage {
            source: source.into(),
        }
    })?;

    Ok(StoredFile { file_name, path })
}

pub async fn save_avatar_image(
    bytes: &[u8],
    target_path: &str,
    extension: &str,
) -> AppResult<StoredFile> {
    let clean_target = sanitize_avatar_target_path(target_path, extension)?;
    let root = avatar_uploads_dir();
    let path = root.join(&clean_target);
    tracing::info!(
        stage = "save",
        kind = "avatar",
        target_path,
        path = %path.to_string_lossy(),
        size_bytes = bytes.len(),
        "Writing avatar to PHP uploads directory"
    );

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| AppError::FailedToCreateUploadsDirectory { source })?;
    }

    tokio::fs::write(&path, bytes).await.map_err(|source| {
        AppError::FailedToSaveCompressedImage {
            source: source.into(),
        }
    })?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();

    Ok(StoredFile { file_name, path })
}

fn avatar_uploads_dir() -> PathBuf {
    env::var("IMGIX_AVATAR_UPLOADS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/uploads/avatars")
        })
}

fn sanitize_avatar_target_path(target_path: &str, extension: &str) -> AppResult<PathBuf> {
    let normalized = target_path.replace('\\', "/");
    let mut path = Path::new(normalized.trim_start_matches('/')).to_path_buf();

    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AppError::InvalidImageFile);
    }

    let requested_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if requested_extension != "jpg"
        && requested_extension != "jpeg"
        && requested_extension != "png"
        && requested_extension != "gif"
        && requested_extension != "webp"
    {
        return Err(AppError::InvalidImageFile);
    }

    let final_extension = extension.trim_start_matches('.').to_ascii_lowercase();
    if final_extension != "jpg"
        && final_extension != "jpeg"
        && final_extension != "png"
        && final_extension != "gif"
        && final_extension != "webp"
    {
        return Err(AppError::InvalidImageFile);
    }
    path.set_extension(final_extension);

    Ok(path)
}
