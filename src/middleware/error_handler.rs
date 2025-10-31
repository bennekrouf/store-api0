use crate::app_log;
use actix_web::middleware::ErrorHandlerResponse;
use actix_web::HttpResponse;
use actix_web::{dev::ServiceRequest, dev::ServiceResponse, Error, HttpMessage};

pub fn handle_internal_server_error<B>(
    res: ServiceResponse<B>,
) -> Result<ErrorHandlerResponse<B>, Error> {
    let request = res.request();
    let error_msg = format!(
        "Internal server error on {} {}",
        request.method(),
        request.path()
    );

    app_log!(error,
        method = %request.method(),
        path = %request.path(),
        "Internal server error occurred"
    );

    let response = HttpResponse::InternalServerError().json(serde_json::json!({
        "error": "Internal Server Error",
        "message": "A server error occurred. Please try again later.",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "path": request.path()
    }));

    Ok(ErrorHandlerResponse::Response(res.into_response(response)))
}
