//! Integration tests for the testing framework.
//!
//! These tests verify that the TestContext and mock providers work correctly
//! together to enable deterministic testing.

use serde_json::json;
use std::path::Path;
use std::time::Duration;
use xerv_core::testing::{
    ChaosConfig, ChaosEngine, ChaosFault, ClockProvider, EventRecorder, MockClock, MockEnv, MockFs,
    MockHttp, MockRng, MockSecrets, MockUuid, RecordedEvent, RngProvider, TestContextBuilder,
    UuidProvider,
};

#[test]
fn test_deterministic_uuid_generation() {
    // Two contexts with the same configuration should produce identical UUIDs
    let ctx1 = TestContextBuilder::new()
        .with_sequential_uuids()
        .build()
        .unwrap();

    let ctx2 = TestContextBuilder::new()
        .with_sequential_uuids()
        .build()
        .unwrap();

    assert_eq!(ctx1.new_uuid(), ctx2.new_uuid());
    assert_eq!(ctx1.new_uuid(), ctx2.new_uuid());
    assert_eq!(ctx1.new_uuid(), ctx2.new_uuid());
}

#[test]
fn test_deterministic_random_generation() {
    let ctx1 = TestContextBuilder::new().with_seed(42).build().unwrap();

    let ctx2 = TestContextBuilder::new().with_seed(42).build().unwrap();

    // Same seed should produce same sequence
    for _ in 0..10 {
        assert_eq!(ctx1.random_u64(), ctx2.random_u64());
    }
}

#[test]
fn test_fixed_time_context() {
    let ctx = TestContextBuilder::new()
        .with_fixed_time("2024-06-15T12:00:00Z")
        .build()
        .unwrap();

    let time1 = ctx.system_time_millis();
    std::thread::sleep(Duration::from_millis(10));
    let time2 = ctx.system_time_millis();

    // Time should not advance
    assert_eq!(time1, time2);
}

#[test]
fn test_mock_clock_advance() {
    let clock = MockClock::new();

    assert_eq!(clock.now(), 0);

    clock.advance(Duration::from_secs(5));
    assert_eq!(clock.now(), 5_000_000_000); // 5 seconds in nanoseconds

    clock.advance(Duration::from_millis(100));
    assert_eq!(clock.now(), 5_100_000_000);
}

#[tokio::test]
async fn test_mock_http_response() {
    let mock = MockHttp::new()
        .on_get(r"^https://api\.example\.com/users$")
        .respond_json(
            200,
            json!([{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]),
        );

    use xerv_core::testing::HttpProvider;
    let response = mock
        .request(
            "GET",
            "https://api.example.com/users",
            std::collections::HashMap::new(),
            None,
        )
        .await
        .unwrap();

    assert_eq!(response.status, 200);
    let body: serde_json::Value = response.body_json().unwrap();
    assert_eq!(body[0]["name"], "Alice");
}

#[tokio::test]
async fn test_mock_http_recording() {
    let mock = MockHttp::new().on_any(r".*").respond_json(200, json!({}));

    use xerv_core::testing::HttpProvider;
    mock.request(
        "GET",
        "https://example.com/a",
        std::collections::HashMap::new(),
        None,
    )
    .await
    .unwrap();
    mock.request(
        "POST",
        "https://example.com/b",
        std::collections::HashMap::new(),
        Some(b"body".to_vec()),
    )
    .await
    .unwrap();

    assert_eq!(mock.request_count(), 2);
    assert!(mock.assert_request_made("GET", r"example\.com/a"));
    assert!(mock.assert_request_made("POST", r"example\.com/b"));
}

#[test]
fn test_mock_filesystem() {
    use xerv_core::testing::FsProvider;

    let fs = MockFs::new()
        .with_file("/config/app.yaml", b"name: test\nversion: 1.0")
        .with_file("/data/users.json", br#"[{"id": 1}]"#)
        .with_dir("/logs");

    assert!(fs.exists(Path::new("/config/app.yaml")));
    assert!(fs.exists(Path::new("/data/users.json")));
    assert!(fs.is_dir(Path::new("/logs")));

    let content = fs.read(Path::new("/config/app.yaml")).unwrap();
    assert!(String::from_utf8_lossy(&content).contains("name: test"));
}

#[test]
fn test_mock_environment() {
    use xerv_core::testing::EnvProvider;

    let env = MockEnv::new()
        .with_var("DATABASE_URL", "postgres://localhost/test")
        .with_var("DEBUG", "true");

    assert_eq!(
        env.var("DATABASE_URL"),
        Some("postgres://localhost/test".to_string())
    );
    assert_eq!(env.var("DEBUG"), Some("true".to_string()));
    assert_eq!(env.var("UNDEFINED"), None);
}

#[test]
fn test_mock_secrets() {
    use xerv_core::testing::SecretsProvider;

    let secrets = MockSecrets::new()
        .with_secret("API_KEY", "sk-test-123456")
        .with_secret("DB_PASSWORD", "super-secret");

    assert_eq!(secrets.get("API_KEY"), Some("sk-test-123456".to_string()));
    assert!(secrets.exists("DB_PASSWORD"));
    assert!(!secrets.exists("MISSING"));
}

#[test]
fn test_event_recording() {
    let recorder = EventRecorder::new();

    recorder.record(RecordedEvent::ClockNow { nanos: 1000 });
    recorder.record(RecordedEvent::UuidGenerated {
        uuid: "test-uuid".to_string(),
    });
    recorder.record(RecordedEvent::HttpRequest {
        method: "GET".to_string(),
        url: "https://api.example.com/test".to_string(),
        headers: std::collections::HashMap::new(),
        body_size: 0,
    });

    assert_eq!(recorder.len(), 3);
    assert!(recorder.assert_recorded("clock_now"));
    assert!(recorder.assert_recorded("uuid_generated"));
    assert!(recorder.assert_http_request("GET", r"api\.example\.com/test"));
}

#[test]
fn test_event_recorder_json_serialization() {
    let recorder = EventRecorder::new();
    recorder.record(RecordedEvent::RandomU64 { value: 42 });
    recorder.record(RecordedEvent::ClockNow { nanos: 100 });

    let json = recorder.to_json();
    let loaded = EventRecorder::from_json(&json).unwrap();

    assert_eq!(loaded.len(), 2);
}

#[test]
fn test_context_with_recording() {
    let ctx = TestContextBuilder::new()
        .with_fixed_time("2024-01-01T00:00:00Z")
        .with_seed(42)
        .with_sequential_uuids()
        .with_recording()
        .build()
        .unwrap();

    // Perform some operations
    ctx.now();
    ctx.system_time_millis();
    ctx.new_uuid();
    ctx.random_u64();
    ctx.env_var("TEST_VAR");
    ctx.secret("API_KEY");

    let recorder = ctx.recorder().unwrap();
    assert!(recorder.len() >= 6);

    assert!(recorder.assert_recorded("clock_now"));
    assert!(recorder.assert_recorded("system_time"));
    assert!(recorder.assert_recorded("uuid_generated"));
    assert!(recorder.assert_recorded("random_u64"));
    assert!(recorder.assert_recorded("env_read"));
    assert!(recorder.assert_recorded("secret_read"));
}

#[test]
fn test_chaos_engine_deterministic() {
    let config = ChaosConfig::new().with_seed(42).with_fault_rate(0.5);

    let engine1 = ChaosEngine::new(config.clone());
    let engine2 = ChaosEngine::new(config);

    // Same seed should produce same results
    let results1: Vec<bool> = (0..10).map(|_| engine1.should_inject()).collect();
    engine1.reset();
    let results2: Vec<bool> = (0..10).map(|_| engine2.should_inject()).collect();

    assert_eq!(results1, results2);
}

#[test]
fn test_chaos_fault_selection() {
    let config = ChaosConfig::new()
        .with_seed(42)
        .with_fault(ChaosFault::NetworkTimeout { probability: 0.5 })
        .with_fault(ChaosFault::ServerError { probability: 0.5 });

    let engine = ChaosEngine::new(config);

    // With high probability, should get some faults
    let mut got_timeout = false;
    let mut got_server_error = false;

    for _ in 0..100 {
        if let Some(fault) = engine.select_fault() {
            match fault.name() {
                "network_timeout" => got_timeout = true,
                "server_error" => got_server_error = true,
                _ => {}
            }
        }
    }

    assert!(got_timeout || got_server_error);
}

#[test]
fn test_full_test_context_configuration() {
    let mock_http = MockHttp::new()
        .on_get(r"^https://api\.example\.com/status$")
        .respond_json(200, json!({"status": "ok"}));

    let mock_fs = MockFs::new().with_file("/config/settings.yaml", b"debug: true");

    let mock_env = MockEnv::new().with_var("ENV", "test");

    let mock_secrets = MockSecrets::new().with_secret("API_KEY", "test-key");

    let ctx = TestContextBuilder::new()
        .with_fixed_time("2024-06-15T12:00:00Z")
        .with_seed(42)
        .with_sequential_uuids()
        .with_mock_http(mock_http)
        .with_mock_fs(mock_fs)
        .with_mock_env(mock_env)
        .with_mock_secrets(mock_secrets)
        .with_recording()
        .build()
        .unwrap();

    // All providers should be mocks
    assert!(ctx.clock.is_mock());
    assert!(ctx.http.is_mock());
    assert!(ctx.rng.is_mock());
    assert!(ctx.uuid.is_mock());
    assert!(ctx.fs.is_mock());
    assert!(ctx.env.is_mock());
    assert!(ctx.secrets.is_mock());

    // Test operations
    assert!(ctx.env_var("ENV").is_some());
    assert!(ctx.secret("API_KEY").is_some());
}

#[test]
fn test_predetermined_uuids() {
    let predetermined = MockUuid::from_strings(&[
        "11111111-1111-1111-1111-111111111111",
        "22222222-2222-2222-2222-222222222222",
        "33333333-3333-3333-3333-333333333333",
    ]);

    assert_eq!(
        predetermined.new_v4().to_string(),
        "11111111-1111-1111-1111-111111111111"
    );
    assert_eq!(
        predetermined.new_v4().to_string(),
        "22222222-2222-2222-2222-222222222222"
    );
    assert_eq!(
        predetermined.new_v4().to_string(),
        "33333333-3333-3333-3333-333333333333"
    );
}

#[test]
fn test_rng_reproducibility() {
    let rng = MockRng::seeded(12345);

    let first_run: Vec<u64> = (0..5).map(|_| rng.next_u64()).collect();

    rng.reset();

    let second_run: Vec<u64> = (0..5).map(|_| rng.next_u64()).collect();

    assert_eq!(first_run, second_run);
}
