use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use thiserror::Error;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum AppError {
    #[error("No file provided")]
    NoFileProvided,

    #[error("Invalid image file")]
    InvalidImageFile,

    #[error("File too large")]
    FileTooLarge,

    #[error("Failed to create uploads directory")]
    FailedToCreateUploadsDirectory {
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to save original image")]
    FailedToSaveOriginalImage {
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to save compressed image")]
    FailedToSaveCompressedImage {
        #[source]
        source: anyhow::Error,
    },

    #[error("Internal server error")]
    Internal {
        #[source]
        source: anyhow::Error,
    },
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!(error = %self, "Image processing error");

        let status = match self {
            AppError::NoFileProvided | AppError::InvalidImageFile => StatusCode::BAD_REQUEST,
            AppError::FileTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            AppError::FailedToCreateUploadsDirectory { .. }
            | AppError::FailedToSaveOriginalImage { .. }
            | AppError::FailedToSaveCompressedImage { .. }
            | AppError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let body = Json(ErrorResponse {
            error: self.to_string(),
        });

        (status, body).into_response()
    }
}
