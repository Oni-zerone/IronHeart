use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// Errore inatteso: si registra nel log e il client riceve solo un 500.
pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(err = ?self.0, "errore nella richiesta");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

pub type AppResult<T> = Result<T, AppError>;
