//! Access-log and error-log setup.

use opentelemetry::trace::TracerProvider as _;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT").is_some()
        || std::env::var_os("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").is_some()
    {
        let exporter = match opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .build()
        {
            Ok(exporter) => exporter,
            Err(error) => {
                eprintln!("failed to initialize OTLP tracing exporter: {error}");
                init_without_otel(filter);
                return;
            }
        };
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();
        let tracer = provider.tracer("webserver");
        opentelemetry::global::set_tracer_provider(provider);
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json())
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .init();
        return;
    }
    init_without_otel(filter);
}

fn init_without_otel(filter: EnvFilter) {
    if std::env::var("WEBSERVER_LOG_FORMAT").is_ok_and(|value| value.eq_ignore_ascii_case("json")) {
        fmt()
            .json()
            .with_env_filter(filter)
            .with_current_span(true)
            .init();
    } else {
        fmt()
            .with_env_filter(filter)
            .with_target(false)
            .compact()
            .init();
    }
}
