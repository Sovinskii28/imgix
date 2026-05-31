use crate::errors::{AppError, AppResult};

use std::path::{Path, PathBuf};
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

    tokio::fs::write(&path, bytes)
        .await
        .map_err(|source| AppError::FailedToSaveOriginalImage { source })?;

    Ok(StoredFile { file_name, path })
}

pub async fn save_compressed_image(bytes: &[u8], file_id: Uuid) -> AppResult<StoredFile> {
    tokio::fs::create_dir_all(Path::new(COMPRESSED_DIR))
        .await
        .map_err(|source| AppError::FailedToCreateUploadsDirectory { source })?;

    let file_name = format!("{}_compressed.jpg", file_id);
    let path = Path::new(COMPRESSED_DIR).join(&file_name);

    tokio::fs::write(&path, bytes).await.map_err(|source| {
        AppError::FailedToSaveCompressedImage {
            source: source.into(),
        }
    })?;

    Ok(StoredFile { file_name, path })
}
