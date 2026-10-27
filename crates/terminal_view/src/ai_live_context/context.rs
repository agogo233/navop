//! 用终端现场组装本轮的 AI 资源上下文。

use agent_runtime::{ResourceContext, ResourceId, ResourceRef, ResourceScope};

use super::{SCOPE_CONNECTION, SCOPE_LIVE_TERMINAL, apply_live_scopes};

/// 组装资源上下文需要的现场输入。
pub(crate) struct LiveContextInputs<'a> {
    /// 当前终端的资源引用（SSH / 本地终端都由宿主注册）。
    pub(crate) terminal: &'a ResourceRef,
    /// 终端的实时作用域，每轮重新抓取。
    pub(crate) live_scopes: Vec<ResourceScope>,
    /// 其它可寻址资源（连接等），只作为 target 候选。
    pub(crate) catalog: Vec<ResourceRef>,
    /// SSH 终端所属连接的资源 id；`None` 表示本地终端（终端自身即默认目标）。
    pub(crate) current_connection: Option<&'a str>,
}

/// 用最新现场组装资源上下文。
///
/// - 本地终端：终端是默认目标，连接等资源只作为候选。
/// - SSH 终端：连接仍是默认目标（连接级工具保持原有行为），终端带实时状态
///   紧跟其后，并在两边互相标注 id，模型无需 `list_sessions` 就能指到它。
///
/// 返回 `None` 表示「这台终端当前不应暴露给 AI」：连接被「禁止 AI 操作」时
/// 连接不会出现在 catalog 里，此时沿用宿主的静态资源池，避免绕过连接策略。
pub(crate) fn build_live_resource_context(
    inputs: LiveContextInputs<'_>,
) -> Option<ResourceContext> {
    let mut terminal = inputs.terminal.clone();
    apply_live_scopes(&mut terminal, inputs.live_scopes);
    match inputs.current_connection {
        Some(connection_id) => {
            build_with_connection_default(connection_id, terminal, inputs.catalog)
        }
        None => Some(build_with_terminal_default(terminal, inputs.catalog)),
    }
}

fn build_with_terminal_default(
    terminal: ResourceRef,
    catalog: Vec<ResourceRef>,
) -> ResourceContext {
    let current = terminal.id.clone();
    let mut resources = Vec::with_capacity(catalog.len() + 1);
    resources.push(terminal);
    resources.extend(catalog);
    ResourceContext {
        current: Some(current),
        resources,
    }
}

fn build_with_connection_default(
    connection_id: &str,
    mut terminal: ResourceRef,
    catalog: Vec<ResourceRef>,
) -> Option<ResourceContext> {
    let index = catalog
        .iter()
        .position(|resource| resource.id.as_str() == connection_id)?;
    let mut connection = catalog[index].clone();
    connection.set_scope(ResourceScope::new(
        SCOPE_LIVE_TERMINAL,
        "Live terminal",
        terminal.id.as_str(),
    ));
    terminal.set_scope(ResourceScope::new(
        SCOPE_CONNECTION,
        "Connection",
        connection_id,
    ));

    let mut resources = Vec::with_capacity(catalog.len() + 1);
    resources.push(connection);
    resources.push(terminal);
    resources.extend(
        catalog
            .into_iter()
            .enumerate()
            .filter(|(position, _)| *position != index)
            .map(|(_, resource)| resource),
    );
    Some(ResourceContext {
        current: Some(ResourceId::new(connection_id)),
        resources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::ResourceKind;

    fn terminal() -> ResourceRef {
        ResourceRef::new("term-1", ResourceKind::Terminal, "本地终端").with_alias("local")
    }

    fn connection() -> ResourceRef {
        ResourceRef::new("74", ResourceKind::Ssh, "政务comi 24").with_alias("comi24")
    }

    fn scope(key: &str, value: &str) -> ResourceScope {
        ResourceScope::new(key, "Label", value)
    }

    fn describe_of(inputs: LiveContextInputs<'_>) -> String {
        build_live_resource_context(inputs)
            .expect("context built")
            .describe()
    }

    #[test]
    fn keeps_terminal_current_and_appends_catalog() {
        let context = build_live_resource_context(LiveContextInputs {
            terminal: &terminal(),
            live_scopes: vec![scope("cwd", "/tmp")],
            catalog: vec![connection()],
            current_connection: None,
        })
        .expect("context built");

        assert_eq!(
            context.current().map(|resource| resource.id.clone()),
            Some("term-1".into())
        );
        assert_eq!(context.resources.len(), 2);
        assert_eq!(context.resources[0].scopes[0].value, "/tmp");
        assert_eq!(context.resources[0].aliases, vec!["local".to_string()]);
    }

    #[test]
    fn describe_exposes_live_state_to_the_model() {
        let described = describe_of(LiveContextInputs {
            terminal: &terminal(),
            live_scopes: vec![scope("screen", "full-screen program running (vim)")],
            catalog: Vec::new(),
            current_connection: None,
        });

        assert!(described.contains("screen=full-screen program running (vim)"));
        assert!(described.contains("id=term-1"));
    }

    #[test]
    fn stale_live_scope_is_replaced_by_the_new_one() {
        let stale = terminal().with_scope(scope("screen", "normal shell"));

        let context = build_live_resource_context(LiveContextInputs {
            terminal: &stale,
            live_scopes: vec![scope("screen", "full-screen")],
            catalog: Vec::new(),
            current_connection: None,
        })
        .expect("context built");

        assert_eq!(context.resources[0].scopes.len(), 1);
        assert_eq!(context.resources[0].scopes[0].value, "full-screen");
    }

    #[test]
    fn ssh_keeps_connection_current_and_attaches_live_terminal_state() {
        let context = build_live_resource_context(LiveContextInputs {
            terminal: &terminal(),
            live_scopes: vec![scope("cwd", "/data/Seeyon"), scope("selection", "12 lines")],
            catalog: vec![connection()],
            current_connection: Some("74"),
        })
        .expect("context built");

        // 连接仍是默认目标：连接级工具（sftp、ssh.exec）行为不变。
        assert_eq!(
            context.current().map(|resource| resource.id.as_str()),
            Some("74")
        );
        assert_eq!(context.resources[0].id.as_str(), "74");
        // 终端紧随其后，带实时状态，并在两边互相标注 id。
        assert_eq!(context.resources[1].id.as_str(), "term-1");
        let described = context.describe();
        assert!(described.contains("cwd=/data/Seeyon"));
        assert!(described.contains("terminal=term-1"));
        assert!(described.contains("connection=74"));
    }

    #[test]
    fn ssh_terminal_state_is_visible_without_listing_sessions() {
        let context = build_live_resource_context(LiveContextInputs {
            terminal: &terminal(),
            live_scopes: vec![scope("screen", "full-screen program running (vi)")],
            catalog: vec![connection()],
            current_connection: Some("74"),
        })
        .expect("context built");

        let terminal = context
            .resources
            .iter()
            .find(|resource| resource.id.as_str() == "term-1")
            .expect("terminal listed");
        assert!(
            terminal
                .scopes
                .iter()
                .any(|scope| scope.key == "screen" && scope.value.contains("vi"))
        );
    }

    #[test]
    fn connection_denied_for_ai_yields_no_live_context() {
        // 连接被「禁止 AI 操作」时不会出现在 catalog 里：不能只把终端塞进池子，
        // 否则等于绕过了连接级策略。
        let context = build_live_resource_context(LiveContextInputs {
            terminal: &terminal(),
            live_scopes: vec![scope("cwd", "/data")],
            catalog: Vec::new(),
            current_connection: Some("74"),
        });

        assert!(context.is_none());
    }
}
