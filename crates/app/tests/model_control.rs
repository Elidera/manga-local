#[path = "../src/llm/control.rs"]
mod control;

use control::{pause_model, ServerCmd};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn benchmark_waits_for_stop_ack_and_releases_model_on_drop() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let caller = tokio::spawn(async move { pause_model(&tx).await });
    let ServerCmd::PauseForBenchmark(request) = rx.recv().await.unwrap() else {
        panic!("wrong command")
    };
    assert!(
        !caller.is_finished(),
        "benchmark must wait for the model to stop"
    );
    let supervisor = tokio::spawn(request.wait());
    let pause = caller.await.unwrap().unwrap();
    assert!(
        !supervisor.is_finished(),
        "model must stay paused during comparison"
    );
    drop(pause);
    tokio::time::timeout(Duration::from_secs(1), supervisor)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancellation_before_ack_does_not_leave_supervisor_waiting() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let caller = tokio::spawn(async move { pause_model(&tx).await });
    let ServerCmd::PauseForBenchmark(request) = rx.recv().await.unwrap() else {
        panic!("wrong command")
    };
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), request.wait())
        .await
        .unwrap();
}

#[tokio::test]
async fn cancellation_during_benchmark_resumes_the_managed_model() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (held_tx, held_rx) = oneshot::channel();
    let caller = tokio::spawn(async move {
        let _pause = pause_model(&tx).await.unwrap();
        held_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let ServerCmd::PauseForBenchmark(request) = rx.recv().await.unwrap() else {
        panic!("wrong command")
    };
    let supervisor = tokio::spawn(request.wait());
    held_rx.await.unwrap();
    assert!(!supervisor.is_finished());
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), supervisor)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn missing_stop_ack_fails_instead_of_starting_another_model() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let caller = tokio::spawn(async move { pause_model(&tx).await });
    drop(rx.recv().await.unwrap());
    assert!(caller.await.unwrap().is_err());
}
