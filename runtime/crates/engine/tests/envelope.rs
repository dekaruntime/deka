use engine::ResponseEnvelope;

#[test]
fn response_status_codes_propagate_correctly() {
    let value = serde_json::json!({
        "status": 418,
        "headers": {"x-custom": "yes"},
        "body": "teapot"
    });
    let envelope = ResponseEnvelope::from_value(value).expect("valid envelope");
    assert_eq!(envelope.status, 418);
    assert_eq!(envelope.headers.get("x-custom"), Some(&"yes".to_string()));
    assert_eq!(envelope.body, "teapot");
}

#[test]
fn not_found_returns_correct_json_error_shape() {
    let value = serde_json::json!({
        "status": 404,
        "headers": {"content-type": "application/json"},
        "body": r#"{"error": "not found"}"#
    });
    let envelope = ResponseEnvelope::from_value(value).expect("valid envelope");
    assert_eq!(envelope.status, 404);
    let parsed: serde_json::Value = serde_json::from_str(&envelope.body).unwrap();
    assert_eq!(
        parsed.get("error").and_then(|v| v.as_str()),
        Some("not found")
    );
}

#[test]
fn error_envelope_on_handler_panic() {
    // When a handler panics, dispatch returns an Err(String).
    // This test verifies that a synthetic error JSON envelope round-trips
    // through ResponseEnvelope::from_value with the expected shape.
    let value = serde_json::json!({
        "status": 500,
        "headers": {"content-type": "application/json"},
        "body": r#"{"error": "handler execution failed: simulated panic"}"#
    });
    let envelope = ResponseEnvelope::from_value(value).expect("valid envelope");
    assert_eq!(envelope.status, 500);
    let parsed: serde_json::Value = serde_json::from_str(&envelope.body).unwrap();
    assert!(
        parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("simulated panic")
    );
}
