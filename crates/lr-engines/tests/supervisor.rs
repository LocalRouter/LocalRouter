//! Supervisor tests against the `lr-fake-engine` binary.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lr_engines::{EngineError, EngineState, LaunchSpec, PortArg, Supervisor};

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lr-fake-engine"))
}

fn spec(key: &str, env: &[(&str, &str)]) -> LaunchSpec {
    let mut env: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    env.push(("FAKE_KEY_VAR".into(), "ENGINE_KEY".into()));
    LaunchSpec {
        key: key.to_string(),
        label: format!("fake {key}"),
        program: fake(),
        args: vec![],
        env,
        port: PortArg::Flag("--port".into()),
        api_key_env: "ENGINE_KEY".into(),
        ready_path: "/health".into(),
        start_timeout: Duration::from_secs(10),
        idle_timeout: None,
    }
}

fn supervisor() -> (Arc<Supervisor>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Supervisor::new(dir.path()), dir)
}

async fn chat(handle: &lr_engines::EngineHandle, key: Option<&str>) -> reqwest::StatusCode {
    let mut req = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", handle.base_url()))
        .json(&serde_json::json!({"model": "m", "messages": []}));
    if let Some(key) = key {
        req = req.bearer_auth(key);
    }
    req.send().await.unwrap().status()
}

#[tokio::test]
async fn starts_waits_for_loading_and_requires_the_key() {
    let (sup, _dir) = supervisor();
    let handle = sup
        .ensure(spec("a", &[("FAKE_LOADING_MS", "600")]))
        .await
        .unwrap();
    assert_eq!(chat(&handle, Some(handle.api_key())).await, 200);
    assert_eq!(
        chat(&handle, None).await,
        401,
        "engine must require the key"
    );
    assert!(sup.is_running("a"));
    // The same spec returns the same process.
    let again = sup
        .ensure(spec("a", &[("FAKE_LOADING_MS", "600")]))
        .await
        .unwrap();
    assert_eq!(again.port, handle.port);
    // Logs never contain the key.
    assert!(sup.logs("a").iter().all(|l| !l.contains(handle.api_key())));
    sup.stop_all().await;
    assert!(!sup.is_running("a"));
}

#[tokio::test]
async fn late_binding_and_env_port() {
    let (sup, _dir) = supervisor();
    let mut s = spec(
        "late",
        &[("FAKE_START_DELAY_MS", "700"), ("FAKE_PORT_VAR", "MY_PORT")],
    );
    s.port = PortArg::Env("MY_PORT".into());
    let handle = sup.ensure(s).await.unwrap();
    assert_eq!(chat(&handle, Some(handle.api_key())).await, 200);
    sup.stop_all().await;
}

#[tokio::test]
async fn concurrent_starts_share_one_process() {
    let (sup, _dir) = supervisor();
    let s = spec("shared", &[("FAKE_LOADING_MS", "300")]);
    let (a, b) = tokio::join!(sup.ensure(s.clone()), sup.ensure(s));
    assert_eq!(a.unwrap().port, b.unwrap().port);
    assert_eq!(sup.processes().len(), 1);
    sup.stop_all().await;
}

#[tokio::test]
async fn changed_configuration_restarts() {
    let (sup, _dir) = supervisor();
    let first = sup.ensure(spec("cfg", &[])).await.unwrap();
    let mut changed = spec("cfg", &[]);
    changed.args.push("--unused-flag".into());
    let second = sup.ensure(changed).await.unwrap();
    assert_ne!(first.port, second.port);
    assert_eq!(sup.processes().len(), 1);
    sup.stop_all().await;
}

#[tokio::test]
async fn failing_start_reports_output_and_gives_up_after_three_tries() {
    let (sup, _dir) = supervisor();
    for _ in 0..3 {
        match sup.ensure(spec("bad", &[("FAKE_FAIL_START", "1")])).await {
            Err(EngineError::ExitedDuringStart { tail, code, .. }) => {
                assert_eq!(code, Some(2));
                assert!(tail.contains("failing on purpose"), "{tail}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(matches!(
        sup.ensure(spec("bad", &[("FAKE_FAIL_START", "1")])).await,
        Err(EngineError::TooManyFailures { .. })
    ));
    let info = sup.processes();
    assert_eq!(info[0].state, EngineState::Failed);
    // An explicit stop clears the failure history.
    sup.stop("bad").await;
    assert!(matches!(
        sup.ensure(spec("bad", &[("FAKE_FAIL_START", "1")])).await,
        Err(EngineError::ExitedDuringStart { .. })
    ));
}

#[tokio::test]
async fn crash_is_detected_and_restarted_on_next_use() {
    let (sup, _dir) = supervisor();
    let s = spec("crashy", &[("FAKE_EXIT_AFTER_MS", "400")]);
    let first = sup.ensure(s.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(!sup.is_running("crashy"));
    let second = sup.ensure(s).await.unwrap();
    assert_ne!(first.port, second.port);
    assert_eq!(sup.processes()[0].restarts, 1);
    sup.stop_all().await;
}

#[tokio::test]
async fn start_timeout_kills_the_process() {
    let (sup, _dir) = supervisor();
    let mut s = spec("slow", &[("FAKE_LOADING_MS", "60000")]);
    s.start_timeout = Duration::from_millis(800);
    assert!(matches!(
        sup.ensure(s).await,
        Err(EngineError::StartTimeout { .. })
    ));
    assert!(!sup.is_running("slow"));
}

#[tokio::test]
async fn idle_engines_stop_but_busy_ones_do_not() {
    let (sup, _dir) = supervisor();
    let mut s = spec("idle", &[]);
    s.idle_timeout = Some(Duration::from_millis(200));
    let handle = sup.ensure(s.clone()).await.unwrap();

    let lease = handle.lease();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        sup.reap_idle().await.is_empty(),
        "busy engine must not stop"
    );
    drop(lease);

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(sup.reap_idle().await, vec!["idle".to_string()]);
    assert!(!sup.is_running("idle"));
}

#[tokio::test]
async fn orphans_from_a_previous_session_are_killed() {
    let dir = tempfile::tempdir().unwrap();
    let port = {
        let sup = Supervisor::new(dir.path());
        let handle = sup.ensure(spec("orphan", &[])).await.unwrap();
        // Simulate an app crash: forget the supervisor without stopping.
        std::mem::forget(sup);
        handle.port
    };
    // A new session cleans up the leftover process.
    let _sup = Supervisor::new(dir.path());
    let mut gone = false;
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
        {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "orphaned engine still listening");
}

#[tokio::test(flavor = "multi_thread")]
async fn blocking_stop_all_works_inside_the_runtime() {
    // App exit calls this from inside the async runtime, where blocking on
    // the runtime would panic.
    let (sup, _dir) = supervisor();
    let a = sup.ensure(spec("a", &[])).await.unwrap();
    let b = sup.ensure(spec("b", &[])).await.unwrap();
    let urls = [a.base_url(), b.base_url()];
    sup.stop_all_blocking();
    assert!(!sup.is_running("a") && !sup.is_running("b"));
    for url in urls {
        let reachable = reqwest::Client::new()
            .get(format!("{url}/health"))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .is_ok();
        assert!(!reachable, "{url} still answers after stop_all_blocking");
    }
}
