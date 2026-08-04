//! Built-in HTML error documents shipped with the server binary.

use hyper::StatusCode;

pub fn document(status: StatusCode) -> String {
    let document = match status {
        StatusCode::BAD_REQUEST => include_str!("../../assets/error-pages/400.html"),
        StatusCode::FORBIDDEN => include_str!("../../assets/error-pages/403.html"),
        StatusCode::NOT_FOUND => include_str!("../../assets/error-pages/404.html"),
        StatusCode::METHOD_NOT_ALLOWED => include_str!("../../assets/error-pages/405.html"),
        StatusCode::LENGTH_REQUIRED => include_str!("../../assets/error-pages/411.html"),
        StatusCode::PAYLOAD_TOO_LARGE => include_str!("../../assets/error-pages/413.html"),
        StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE => {
            include_str!("../../assets/error-pages/431.html")
        }
        StatusCode::BAD_GATEWAY => include_str!("../../assets/error-pages/502.html"),
        StatusCode::SERVICE_UNAVAILABLE => include_str!("../../assets/error-pages/503.html"),
        StatusCode::GATEWAY_TIMEOUT => include_str!("../../assets/error-pages/504.html"),
        _ => include_str!("../../assets/error-pages/500.html"),
    };
    document.replace(
        "</head>",
        "<style>body{min-height:0;display:block}.page{margin:clamp(3.5rem,14vh,10rem) auto 0}</style></head>",
    )
}
