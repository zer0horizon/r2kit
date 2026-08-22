use crate::ServiceError;

#[cfg(feature = "tracing")]
pub(crate) fn remote_failure(error: &ServiceError) {
    tracing::warn!(
        target: "r2kit",
        operation = error.operation(),
        error.kind = error.kind().as_str(),
        http.status = ?error.status(),
        "R2 request failed"
    );
}

#[cfg(not(feature = "tracing"))]
pub(crate) fn remote_failure(_: &ServiceError) {}

#[cfg(feature = "tracing")]
pub(crate) fn preflight(phase: &'static str) {
    tracing::debug!(
        target: "r2kit",
        operation = "ListObjectsV2",
        preflight.phase = phase,
        "R2 bucket preflight"
    );
}

#[cfg(not(feature = "tracing"))]
pub(crate) fn preflight(_: &'static str) {}

#[cfg(feature = "tracing")]
pub(crate) fn managed_upload(
    phase: &'static str,
    part_size: u64,
    concurrency: usize,
    max_attempts: u8,
) {
    tracing::debug!(
        target: "r2kit",
        operation = "ManagedMultipartUpload",
        upload.phase = phase,
        part_size,
        concurrency,
        max_attempts,
        "managed R2 upload"
    );
}

#[cfg(not(feature = "tracing"))]
pub(crate) fn managed_upload(_: &'static str, _: u64, _: usize, _: u8) {}

#[cfg(feature = "tracing")]
pub(crate) fn upload_part_retry(part_number: u16, attempt: u8, max_attempts: u8, delay_ms: u64) {
    tracing::warn!(
        target: "r2kit",
        operation = "UploadPart",
        part_number,
        attempt,
        max_attempts,
        retry.delay_ms = delay_ms,
        "retrying R2 upload part"
    );
}

#[cfg(not(feature = "tracing"))]
pub(crate) fn upload_part_retry(_: u16, _: u8, _: u8, _: u64) {}

#[cfg(all(test, feature = "tracing"))]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing::{
        Event, Metadata, Subscriber,
        field::{Field, Visit},
        span::{Attributes, Id, Record},
    };

    use super::*;
    use crate::ServiceErrorKind;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<String>>);

    struct Visitor<'a>(&'a mut String);

    impl Visit for Visitor<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                return;
            }
            self.0.push_str(field.name());
            self.0.push('=');
            self.0.push_str(&format!("{value:?};"));
        }
    }

    impl Subscriber for Capture {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut output = self.0.lock().unwrap();
            event.record(&mut Visitor(&mut output));
        }

        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    #[test]
    fn events_expose_only_the_documented_safe_field_set() {
        let capture = Capture::default();
        let output = Arc::clone(&capture.0);
        tracing::subscriber::with_default(capture, || {
            remote_failure(&ServiceError::sanitized(
                "PutObject",
                ServiceErrorKind::PermissionDenied,
                Some(403),
            ));
            preflight("start");
            managed_upload("start", 8 * 1024 * 1024, 4, 3);
            upload_part_retry(2, 1, 3, 264);
        });

        let output = output.lock().unwrap();
        for forbidden in [
            "bucket",
            "key",
            "path",
            "account",
            "credential",
            "upload_id",
            "url",
            "header",
        ] {
            assert!(
                !output.contains(forbidden),
                "unsafe trace field: {forbidden}"
            );
        }
        for required in [
            "operation",
            "error.kind",
            "http.status",
            "part_size",
            "concurrency",
            "max_attempts",
            "part_number",
            "attempt",
            "retry.delay_ms",
        ] {
            assert!(output.contains(required), "missing trace field: {required}");
        }
    }
}
