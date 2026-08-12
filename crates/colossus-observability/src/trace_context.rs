use colossus_contracts::RemoteTraceContext;
use opentelemetry::{
    Context, global,
    propagation::{Extractor, Injector, TextMapPropagator},
    trace::TraceContextExt as _,
};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use std::collections::BTreeMap;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

struct Carrier(BTreeMap<String, String>);

impl Extractor for Carrier {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

impl Injector for Carrier {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.into(), value);
    }
}

/// Validate and normalize inbound W3C Trace Context without accepting baggage.
pub fn extract_remote_trace_context(
    traceparent: Option<&str>,
    tracestate: Option<&str>,
) -> Option<RemoteTraceContext> {
    let traceparent = traceparent?;
    let mut fields = BTreeMap::from([("traceparent".into(), traceparent.into())]);
    if let Some(tracestate) = tracestate {
        fields.insert("tracestate".into(), tracestate.into());
    }
    let context = TraceContextPropagator::new().extract(&Carrier(fields));
    if !context.span().span_context().is_valid() {
        return None;
    }
    inject_context(&context)
}

/// Capture the current span as normalized W3C Trace Context.
pub fn current_trace_context() -> Option<RemoteTraceContext> {
    inject_context(&Span::current().context())
}

/// Capture a specific tracing span as normalized W3C Trace Context.
pub fn trace_context_for_span(span: &Span) -> Option<RemoteTraceContext> {
    inject_context(&span.context())
}

/// Set a validated remote parent on a tracing span.
pub fn set_remote_parent(span: &Span, remote: &RemoteTraceContext) -> bool {
    let mut fields = BTreeMap::from([("traceparent".into(), remote.traceparent.clone())]);
    if let Some(tracestate) = remote.tracestate.as_ref() {
        fields.insert("tracestate".into(), tracestate.clone());
    }
    let parent = TraceContextPropagator::new().extract(&Carrier(fields));
    parent.span().span_context().is_valid() && span.set_parent(parent).is_ok()
}

/// Link a new local span to a validated durable W3C context.
pub fn add_remote_link(span: &Span, remote: &RemoteTraceContext) -> bool {
    let mut fields = BTreeMap::from([("traceparent".into(), remote.traceparent.clone())]);
    if let Some(tracestate) = remote.tracestate.as_ref() {
        fields.insert("tracestate".into(), tracestate.clone());
    }
    let linked = TraceContextPropagator::new().extract(&Carrier(fields));
    let span_context = linked.span().span_context().clone();
    if !span_context.is_valid() {
        return false;
    }
    span.add_link(span_context);
    true
}

/// Inject the current tracing span into an outbound header callback.
///
/// Only `traceparent` and `tracestate` are produced; baggage is never forwarded.
pub fn inject_current_trace_context(mut set_header: impl FnMut(&str, &str)) {
    let mut carrier = Carrier(BTreeMap::new());
    TraceContextPropagator::new().inject_context(&Span::current().context(), &mut carrier);
    for (key, value) in carrier.0 {
        set_header(&key, &value);
    }
}

/// Return the current W3C propagation headers for HTTP-capable adapters.
pub fn current_trace_headers() -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(2);
    inject_current_trace_context(|name, value| headers.push((name.into(), value.into())));
    headers
}

/// Install the W3C Trace Context propagator globally for compatible adapters.
pub fn install_trace_context_propagator() {
    global::set_text_map_propagator(TraceContextPropagator::new());
}

fn inject_context(context: &Context) -> Option<RemoteTraceContext> {
    if !context.span().span_context().is_valid() {
        return None;
    }
    let mut carrier = Carrier(BTreeMap::new());
    TraceContextPropagator::new().inject_context(context, &mut carrier);
    Some(RemoteTraceContext {
        traceparent: carrier.0.remove("traceparent")?,
        tracestate: carrier.0.remove("tracestate"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

    #[test]
    fn rejects_invalid_traceparent_and_never_retains_baggage() {
        assert!(extract_remote_trace_context(Some("not-a-trace"), None).is_none());
        let extracted = extract_remote_trace_context(
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
            Some("vendor=value"),
        )
        .expect("valid context");
        assert_eq!(
            extracted.traceparent,
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );
        assert_eq!(extracted.tracestate.as_deref(), Some("vendor=value"));
    }

    #[test]
    fn validated_remote_context_parents_a_span_before_it_starts() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer().with_tracer(provider.tracer("trace-context-test")),
        );
        let remote = RemoteTraceContext {
            traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
            tracestate: None,
        };
        {
            let _guard = subscriber.set_default();
            let span = tracing::info_span!("accepted");
            assert!(set_remote_parent(&span, &remote));
            span.in_scope(|| {});
        }
        provider.force_flush().expect("flush spans");
        let spans = exporter.get_finished_spans().expect("finished spans");
        assert_eq!(spans.len(), 1);
        assert_eq!(
            spans[0].span_context.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(spans[0].parent_span_id.to_string(), "00f067aa0ba902b7");
    }
}
