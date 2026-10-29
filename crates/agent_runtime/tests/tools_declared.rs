//! 工具声明进 transcript 的集成测试。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use agent_runtime::history::HistoryItem;
use agent_runtime::model::{
    ModelClient, ModelRequest, ModelResponse, ModelStream,
};
use agent_runtime::tools::{ToolRegistry, ToolRouter};
use agent_runtime::{Runtime, RuntimeServices, TaskKind};
use async_trait::async_trait;

struct AnswerModel(AtomicUsize);

#[async_trait]
impl ModelClient for AnswerModel {
    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, agent_runtime::RuntimeError> {
        Ok(ModelResponse::text("unused"))
    }

    async fn complete_stream(&self, _request: ModelRequest) -> Result<ModelStream, agent_runtime::RuntimeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(agent_runtime::model::model_response_into_stream(
            ModelResponse::text("回答。"),
        ))
    }

    fn model_name(&self) -> &str {
        "answer"
    }
}

#[tokio::test]
async fn tool_declaration_is_recorded_into_history() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(agent_runtime::tools::builtin::EchoTool));
    let runtime = Runtime::new(RuntimeServices::new(
        std::sync::Arc::new(AnswerModel(AtomicUsize::new(0))) as std::sync::Arc<dyn ModelClient>,
        std::sync::Arc::new(ToolRouter::new(registry)),
    ));
    let session = runtime.create_session(Default::default());

    runtime
        .run_turn_blocking(session.id(), "你好".into(), TaskKind::Agent)
        .await
        .expect("turn failed");

    let history = session.history_snapshot();
    let names = history
        .items()
        .iter()
        .find_map(|item| match item {
            HistoryItem::ToolsDeclared { names } => Some(names.clone()),
            _ => None,
        })
        .expect("历史里应有工具声明");
    assert!(
        names.iter().any(|name| name == "echo"),
        "声明应包含已注册的 echo 工具，实际 {names:?}"
    );
}
