//! A temporary, acknowledged pause of the managed model. The benchmark starts
//! its own model only after the supervisor has stopped the reading model.

use anyhow::{anyhow, Result};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub enum ServerCmd {
    Restart,
    PauseForBenchmark(PauseRequest),
}

#[derive(Debug)]
pub struct PauseRequest {
    ready: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

impl PauseRequest {
    /// Called by the supervisor only when its child has stopped. A cancelled
    /// benchmark drops its resume sender, which also releases this wait.
    pub async fn wait(self) {
        if self.ready.send(()).is_ok() {
            let _ = self.resume.await;
        }
    }
}

/// Holding this guard keeps the managed model stopped. Dropping it resumes
/// the supervisor on success, error, timeout or task cancellation.
#[derive(Debug)]
pub struct ModelPause {
    _resume: oneshot::Sender<()>,
}

pub async fn pause_model(tx: &mpsc::UnboundedSender<ServerCmd>) -> Result<ModelPause> {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    tx.send(ServerCmd::PauseForBenchmark(PauseRequest {
        ready: ready_tx,
        resume: resume_rx,
    }))
    .map_err(|_| anyhow!("управление нейросетью недоступно"))?;
    tokio::time::timeout(std::time::Duration::from_secs(30), ready_rx)
        .await
        .map_err(|_| anyhow!("не удалось дождаться выгрузки текущей модели"))?
        .map_err(|_| anyhow!("текущая модель не подтвердила выгрузку"))?;
    Ok(ModelPause { _resume: resume_tx })
}
