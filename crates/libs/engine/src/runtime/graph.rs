use async_trait::async_trait;
use namu_core::ContextId;

#[async_trait]
pub trait ContextGraph: Send + Sync {
    async fn parent(&self, ctx_id: ContextId) -> anyhow::Result<Option<ContextId>>;
}
