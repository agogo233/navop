//! steering（运行中插话）集成测试。
//!
//! 验证 codex 风格 `run_agent_loop` 在下一轮采样前注入排队输入：
//! - 模型在第一轮发起工具调用、工具阻塞期间，用户插话入队
//! - 第二轮请求的历史里应出现该插话（作为 user 消息）
//! - 事件流里应广播 `UserMessage`（供 UI / 回放消费）
//! - 会话空闲时 steer 不入队（不开新轮）

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_runtime::error::ToolError;
use agent_runtime::model::{
    ModelClient, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent, function_tool_call,
    model_response_into_stream,
};
use agent_runtime::tools::{
    ObservationData, Tool, ToolInvocation, ToolName, ToolObservation, ToolRegistry, ToolRouter,
    ToolSpec,
};
use agent_runtime::{
    Runtime, RuntimeEvent, RuntimeServices, TaskKind, UserInput,
};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Notify;

/// 第一轮调工具，之后每轮直接回答；记录每个请求。
struct ToolThenAnswerModel {
    count: AtomicUsize,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ToolThenAnswerModel {
    fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn received_requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("requests lock").clone()
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("requests lock").len()
    }
}

#[async_trait]
impl ModelClient for ToolThenAnswerModel {
    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, agent_runtime::RuntimeError> {
        Ok(ModelResponse::text("unused"))
    }

    async fn complete_stream(&self, request: ModelRequest) -> Result<ModelStream, agent_runtime::RuntimeError> {
        self.requests.lock().expect("requests lock").push(request);
        match self.count.fetch_add(1, Ordering::SeqCst) {
            0 => {
                let call =
                    function_tool_call("call_block", "block", json!({"text": "hello"}).to_string());
                Ok(Box::pin(futures::stream::iter(vec![
                    Ok(ModelStreamEvent::ToolCall(call.clone())),
                    Ok(ModelStreamEvent::Completed(ModelResponse::tool_call(call))),
                ])))
            }
            _ => Ok(model_response_into_stream(ModelResponse::text("后续回答。"))),
        }
    }

    fn model_name(&self) -> &str {
        "tool-then-answer"
    }
}

/// 阻塞在 [`Notify`] 上的工具：测试先插话、再放行，保证确定性窗口。
struct BlockingTool {
    release: Arc<Notify>,
}

#[async_trait]
impl Tool for BlockingTool {
    fn name(&self) -> ToolName {
        ToolName::new("block")
    }

    fn spec(&self, _resources: &agent_runtime::ResourceContext) -> ToolSpec {
        ToolSpec::new(
            "block",
            "阻塞直到测试放行。",
            json!({"type": "object", "properties": {"text": {"type": "string"}}}),
        )
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        self.release.notified().await;
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            "released",
            ObservationData::Text("released".into()),
        ))
    }
}

#[tokio::test]
async fn steering_message_is_injected_before_next_sample() {
    let model = Arc::new(ToolThenAnswerModel::new());
    let release = Arc::new(Notify::new());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(BlockingTool {
        release: release.clone(),
    }));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone() as Arc<dyn ModelClient>,
        Arc::new(ToolRouter::new(registry)),
    ));
    let session = runtime.create_session(Default::default());
    let mut rx = runtime.subscribe();

    // 后台开一轮：第一轮模型调 block 工具，工具阻塞期间主测试插话。
    runtime
        .start_turn(session.id(), "初始目标".into(), TaskKind::Agent)
        .expect("start turn");

    // 等第一轮采样发出（请求已记录）——此时工具尚未放行，轮次必然还在跑。
    for _ in 0..400 {
        if model.request_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        model.request_count() >= 1,
        "第一轮采样应已发生（工具已阻塞）"
    );

    let steered = runtime.steer(session.id(), UserInput::new("先别动，改成只读检查"));
    assert!(steered, "运行中的会话应接受 steering");

    // 放行工具，让轮次带着插话完成第二轮。收集期间所有事件（含插话广播）。
    release.notify_one();
    let mut events = Vec::new();
    for _ in 0..400 {
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        if events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::TurnCompleted { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::TurnCompleted { .. })),
        "轮次应正常完成"
    );

    let requests = model.received_requests();
    assert!(requests.len() >= 2, "至少两轮采样，实际 {}", requests.len());
    let second = &requests[1];
    let steer_in_history = second.messages.iter().any(|message| {
        message.role == agent_runtime::model::Role::User
            && message.content_as_text().contains("先别动，改成只读检查")
    });
    assert!(
        steer_in_history,
        "第二轮请求必须带上插话（user 消息），否则 steering 没生效"
    );

    // 事件流里能看到插话广播（UI 气泡 / 回放依赖）。
    let saw_user_message = events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::UserMessage { text, .. } if text.contains("先别动，改成只读检查")
        )
    });
    assert!(saw_user_message, "插话必须广播 UserMessage 事件");
}

#[tokio::test]
async fn steer_is_rejected_when_session_idle() {
    let model = Arc::new(ToolThenAnswerModel::new());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(agent_runtime::tools::builtin::EchoTool));
    let runtime = Runtime::new(RuntimeServices::new(
        model as Arc<dyn ModelClient>,
        Arc::new(ToolRouter::new(registry)),
    ));
    let session = runtime.create_session(Default::default());

    assert!(
        !runtime.steer(session.id(), UserInput::new("空闲插话")),
        "空闲会话不应接受 steering（由 UI 决定何时开新轮）"
    );
}
