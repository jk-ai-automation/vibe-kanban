use axum::response::Json;
use utils::response::ApiResponse;

pub(crate) async fn health_check() -> Json<ApiResponse<String>> {
    Json(ApiResponse::success("OK".to_string()))
}
