//! SQL 缓存失效的运行时契约。
//!
//! 缓存失效会删除文件缓存（`tokio::fs`），必须在应用 Tokio 运行时上轮询。
//! GPUI 前台 executor（`cx.spawn`）没有运行时上下文，直接 await 会触发
//! `Handle::current()` panic，release 下 `panic = "abort"` 直接崩溃进程。
//! 这里用源码契约锁住「前台必须走 on_runtime 包装」这条约束。

const SQL_RESULT_SOURCE: &str = include_str!("sql_result_tab.rs");
const SQL_EDITOR_SOURCE: &str = include_str!("sql_editor_view.rs");

#[test]
fn sql_result_tab_applies_invalidation_on_tokio_runtime() {
    assert!(SQL_RESULT_SOURCE.contains("apply_sql_cache_invalidation_plan_on_runtime"));
    assert!(!SQL_RESULT_SOURCE.contains(".apply_sql_cache_invalidation_plan("));
}

#[test]
fn sql_editor_view_applies_invalidation_on_tokio_runtime() {
    assert!(SQL_EDITOR_SOURCE.contains("apply_sql_cache_invalidation_plan_on_runtime"));
    assert!(!SQL_EDITOR_SOURCE.contains(".apply_sql_cache_invalidation_plan("));
}
