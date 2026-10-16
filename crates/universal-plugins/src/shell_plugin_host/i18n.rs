//! `navop.i18n` host 模块：宿主当前界面语言 + 切换信号。
//!
//! **这里没有词条表，也不该有。** 扩展的文案不在宿主的 rust-i18n 目录里，
//! 宿主替扩展解析只会把两边的键空间搅在一起（宿主有一条 `common.connect`，
//! 扩展也有一条，撞上时谁都说不清谁赢）。所以分工是：
//!
//! - 宿主给**事实**：界面现在是什么语言（`locale()`）、宿主可能是什么语言
//!   （`available()`）、以及"变了"（`revision()`）；
//! - 扩展带**词条**：自己的 `locales` 表 + 一个 `t()`，按 `locale()` 选表。
//!
//! `locale()` 读的是 `rust_i18n::locale()` —— 就是界面字符串真正渲染用的那个值
//! （由 `gpui_component::set_locale()` 设置，navop 在 `AppSettings::apply` 与
//! 设置页切语言两处调用）。**不要**改读 `AppSettings.locale`：那是设置项
//! （还可能是 `"system"`），与真正生效的语言不是一回事。
//!
//! 切语言后宿主会调用 [`super::refresh_shell_views`]：`revision()` 加一，
//! 并且每个已挂载的 shell view 都会被要求重建页面描述。
//!
//! ⚠️ **重建的是已有脚本实例的 `render()`，`init()` 不会再跑一遍**
//! （`ScriptView::refresh` → `rebuild` → `call_render(object, …)`，
//! 见 gpui-shell `crates/shell/src/view.rs`）。所以正确写法是**在 `render()`
//! 里读 `locale()`**（或把 `revision()` 存下来比对），而不是在 `init()` 里选好
//! 词条表就完事 —— 后者在切语言后仍然显示旧语言文字，且不报任何错。

use std::sync::atomic::{AtomicU64, Ordering};

use gpui_shell::{HostModule, HostValue};
use one_core::settings::{LOCALE_EN, LOCALE_ZH_CN, LOCALE_ZH_HK};

/// 语言切换计数。每次宿主切语言 +1，页面据此判断缓存的词条表要不要重建。
static LOCALE_REVISION: AtomicU64 = AtomicU64::new(0);

/// 宿主可能处在的语言，顺序无含义（不是偏好链）。
///
/// 取 `one_core` 的常量而不是再抄一份字面量：设置页的下拉项、`set_locale`
/// 的取值、这里的清单必须是同一个集合。
pub(super) const AVAILABLE_LOCALES: &[&str] = &[LOCALE_EN, LOCALE_ZH_CN, LOCALE_ZH_HK];

/// 宿主告知语言变了（由 [`super::refresh_shell_views`] 调用）。
pub(super) fn bump_revision() {
    LOCALE_REVISION.fetch_add(1, Ordering::Relaxed);
}

fn revision() -> u64 {
    LOCALE_REVISION.load(Ordering::Relaxed)
}

/// 装配 `navop.i18n`。
///
/// 三个函数都不需要 `App`（语言是 rust-i18n 的全局值），所以不经过
/// `with_current_app`：它们在任何 host 调用栈里都能答。
pub(super) fn i18n_module() -> HostModule {
    HostModule::new("navop.i18n")
        .declarations(
            r#"
            /** 当前界面语言，如 "zh-CN" / "zh-HK" / "en"。 */
            export function locale(): string;
            /** 宿主可能处在的语言；顺序无含义。 */
            export function available(): string[];
            /** 每次宿主切语言 +1。 */
            export function revision(): number;
            "#,
        )
        .function("locale", |_| {
            Ok(HostValue::Str(rust_i18n::locale().to_string()))
        })
        .function("available", |_| {
            Ok(HostValue::Array(
                AVAILABLE_LOCALES
                    .iter()
                    .map(|locale| HostValue::Str((*locale).to_owned()))
                    .collect(),
            ))
        })
        .function("revision", |_| Ok(HostValue::Number(revision() as f64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `available()` 是设置页真正能选到的集合。这条测试盯的是"有人加了语言却
    /// 忘了这里"或"这里抄了字面量、与 core 漂开"。
    #[test]
    fn available_locales_are_the_ones_core_can_resolve() {
        assert_eq!(AVAILABLE_LOCALES, &["en", "zh-CN", "zh-HK"]);
        // core 的解析面与这份清单必须同集合：`system` 是设置项不是语言。
        for locale in AVAILABLE_LOCALES {
            assert_eq!(
                one_core::settings::effective_locale_for_setting(*locale),
                *locale,
                "`{locale}` 不是 core 能原样解析出来的语言"
            );
        }
        assert_eq!(
            "en",
            one_core::settings::effective_locale_for_setting("fr-FR")
        );
    }

    #[test]
    fn revision_moves_only_when_the_host_says_so() {
        let before = revision();
        bump_revision();
        assert_eq!(before + 1, revision());
    }

    /// 没切过语言时 `locale()` 仍必须是**可解析**的值，而不是空串 ——
    /// 页面在 init 里就要用它选词条表，空串会让所有页面落到兜底分支。
    #[test]
    fn locale_is_never_empty() {
        assert!(!rust_i18n::locale().is_empty());
    }
}
