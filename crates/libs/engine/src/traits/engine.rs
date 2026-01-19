use async_trait::async_trait;
use namu_core::ir::Workflow;
use namu_core::registry::{PackFn, TaskImpl, UnpackFn};

use crate::kernel::KernelPlan;

#[async_trait]
pub trait Engine: Send + Sync {
    type WorkflowId: Send + Sync + Copy + 'static;
    type RunId: Send + Sync + Copy + 'static;

    async fn create_workflow(&self, workflow: Workflow) -> Self::WorkflowId;
    async fn create_run(&self, workflow_id: Self::WorkflowId) -> Self::RunId;
    async fn run(&self, run_id: Self::RunId) -> anyhow::Result<()>;
}

#[async_trait]
pub trait TaskRegistry: Send + Sync {
    async fn add_task(
        &self,
        task_name: &str,
        task: TaskImpl,
        pack: Option<PackFn>,
        unpack: Option<UnpackFn>,
    );
}

#[async_trait]
pub trait OrchestratorEngine: Send + Sync {
    type Value: Clone + Send + Sync + 'static;

    async fn drive(
        &self,
        run_id: uuid::Uuid,
        ctx_id: namu_core::ContextId,
        start_op: usize,
        pred_op: Option<usize>,
    ) -> anyhow::Result<KernelPlan>;

    async fn dispatch(&self, run_id: uuid::Uuid, action: KernelPlan) -> anyhow::Result<()>;

    async fn apply_task_output(
        &self,
        run_id: uuid::Uuid,
        op_id: usize,
        ctx_id: namu_core::ContextId,
        output: Self::Value,
    ) -> anyhow::Result<()>;
}

#[async_trait]
pub trait WorkerEngine: Send + Sync {
    type Value: Clone + Send + Sync + 'static;

    async fn execute(
        &self,
        manifest: &namu_proto::TaskManifest,
        artifact_path: &std::path::Path,
        input: &Self::Value,
    ) -> anyhow::Result<Result<Self::Value, String>>;
}
