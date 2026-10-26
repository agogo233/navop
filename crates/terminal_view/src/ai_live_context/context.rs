//! 用终端现场组装本轮的 AI 资源上下文。

use agent_runtime::{ResourceContext, ResourceRef, ResourceScope};

use super::apply_live_scopes;

/// 用最新现场组装资源上下文，并保持 `current_terminal` 是默认目标。
///
/// `catalog` 是连接等其它可寻址资源：它们只作为 target 候选出现，不会抢走
/// 默认目标，这样模型省略 target 时命中的仍是眼前这台终端。
pub(crate) fn build_live_resource_context(
    current_terminal: &ResourceRef,
    live_scopes: Vec<ResourceScope>,
    catalog: Vec<ResourceRef>,
) -> ResourceContext {
    let mut terminal = current_terminal.clone();
    apply_live_scopes(&mut terminal, live_scopes);
    let current = terminal.id.clone();
    let mut resources = Vec::with_capacity(catalog.len() + 1);
    resources.push(terminal);
    resources.extend(catalog);
    ResourceContext {
        current: Some(current),
        resources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::ResourceKind;

    fn terminal() -> ResourceRef {
        ResourceRef::new("term-1", ResourceKind::Terminal, "本地终端").with_alias("local")
    }

    fn scope(key: &str, value: &str) -> ResourceScope {
        ResourceScope::new(key, "Label", value)
    }

    #[test]
    fn keeps_terminal_current_and_appends_catalog() {
        let context = build_live_resource_context(
            &terminal(),
            vec![scope("cwd", "/tmp")],
            vec![ResourceRef::new("conn-1", ResourceKind::Ssh, "prod")],
        );

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
        let context = build_live_resource_context(
            &terminal(),
            vec![scope("screen", "full-screen program running (vim)")],
            Vec::new(),
        );

        let described = context.describe();
        assert!(described.contains("screen=full-screen program running (vim)"));
        assert!(described.contains("id=term-1"));
    }

    #[test]
    fn stale_live_scope_is_replaced_by_the_new_one() {
        let stale = terminal().with_scope(scope("screen", "normal shell"));

        let context =
            build_live_resource_context(&stale, vec![scope("screen", "full-screen")], Vec::new());

        assert_eq!(context.resources[0].scopes.len(), 1);
        assert_eq!(context.resources[0].scopes[0].value, "full-screen");
    }
}
