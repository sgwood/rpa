use ai_rpa_core::{
    ControlMode, DeviceRecord, EvidenceLevel, Provider, RawEventInput, normalize_event,
};
use ai_rpa_server::{
    crypto::ServerCrypto,
    store::{CentralStore, CentralTaskFilter, token_hash},
};
use chrono::Utc;
use sqlx_core::raw_sql::raw_sql;
use sqlx_postgres::PgPoolOptions;

#[tokio::test]
async fn postgres_enrollment_event_and_remote_command_flow() -> anyhow::Result<()> {
    let Ok(database_url) = std::env::var("AI_RPA_TEST_DATABASE_URL") else {
        eprintln!("AI_RPA_TEST_DATABASE_URL not set; PostgreSQL integration test skipped");
        return Ok(());
    };
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&database_url)
        .await?;
    raw_sql(include_str!("../migrations/0001_initial.sql"))
        .execute(&pool)
        .await?;
    raw_sql(
        "TRUNCATE audit_log, commands, events, tasks, adapters, devices, enrollment_codes CASCADE",
    )
    .execute(&pool)
    .await?;
    let crypto = ServerCrypto::from_base64("BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=")?;
    let store = CentralStore::new(pool, crypto);
    let device = DeviceRecord {
        id: "dev-integration".to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        hostname: "integration".to_owned(),
        logical_environment: "test".to_owned(),
        node_version: "0.2.0".to_owned(),
        last_seen_at: Utc::now(),
    };
    store
        .create_enrollment_code(
            &token_hash("CODE"),
            Utc::now() + chrono::Duration::minutes(5),
        )
        .await?;
    store
        .enroll_device(
            &token_hash("CODE"),
            &device,
            "集成测试",
            &token_hash("node-token"),
        )
        .await?;
    assert!(
        store
            .verify_device_token(&device.id, &token_hash("node-token"))
            .await?
    );

    let event = normalize_event(
        RawEventInput {
            provider: Provider::Codex,
            event_type: "turn_started".to_owned(),
            event_id: None,
            idempotency_key: Some("central-integration-event".to_owned()),
            device_id: Some(device.id.clone()),
            session_id: "session-integration".to_owned(),
            turn_id: Some("turn-1".to_owned()),
            occurred_at: Some(Utc::now()),
            title: Some("远程集成测试".to_owned()),
            workspace: Some("/workspace".to_owned()),
            project: Some("rpa".to_owned()),
            control_mode: ControlMode::Managed,
            required_evidence_level: EvidenceLevel::E2,
            payload: serde_json::json!({}),
        },
        &device.id,
    )?;
    assert_eq!(store.ingest_events(&device.id, &[event]).await?, 1);
    let tasks = store.list_tasks(&CentralTaskFilter::default()).await?;
    assert_eq!(tasks.len(), 1);
    let command = store
        .create_command(
            tasks[0].id,
            ai_rpa_core::CommandAction::SendNext,
            "继续完整验证",
            "integration-admin",
            3600,
        )
        .await?;
    let pending = store.pending_commands(&device.id).await?;
    assert_eq!(pending[0].id, command.id);
    assert_eq!(pending[0].message, "继续完整验证");
    store
        .ack_command(
            &device.id,
            command.id,
            ai_rpa_core::CommandState::Accepted,
            Some("accepted"),
        )
        .await?;
    assert!(store.pending_commands(&device.id).await?.is_empty());
    Ok(())
}
