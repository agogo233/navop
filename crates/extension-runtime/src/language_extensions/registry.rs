use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use anyhow::{Context, Result};
use tree_sitter::{Language, Parser, WasmStore, wasmtime};

use super::LanguageManifest;

#[derive(Clone)]
struct RegisteredManifest {
    manifest: LanguageManifest,
    source_path: PathBuf,
    loaded: bool,
}

#[derive(Default)]
struct RuntimeState {
    manifests: HashMap<String, RegisteredManifest>,
}

static STATE: LazyLock<Mutex<RuntimeState>> = LazyLock::new(Mutex::default);
static LOAD_LOCK: LazyLock<Mutex<()>> = LazyLock::new(Mutex::default);
static ENGINE: LazyLock<wasmtime::Engine> = LazyLock::new(wasmtime::Engine::default);
static WASM_STORE: LazyLock<Mutex<WasmStore>> = LazyLock::new(|| {
    let store = run_on_wasm_compile_thread(|| WasmStore::new(&ENGINE).map_err(anyhow::Error::msg))
        .expect("init language extension wasm store");
    Mutex::new(store)
});

/// Compiling tree-sitter's WASM runtime (and every WASM language) with cranelift
/// recurses deeply enough — the egraph passes need roughly 0.5-1 MiB of stack —
/// to outgrow a libdispatch worker thread, whose stack we measured at 536_576
/// bytes. GPUI's background executor runs highlight parses on such a worker, so
/// compiling there walks off the thread stack into its guard page; wasmtime's
/// macOS signal handler cannot attribute that fault to a wasm stack and aborts
/// with SIGILL ("Could not determine thread index for stack guard region",
/// see navop-2026-10-12-181737.ips). Every WASM compilation therefore runs on a
/// dedicated thread with a stack that comfortably fits the recursion.
const WASM_COMPILE_STACK_BYTES: usize = 32 * 1024 * 1024;
const WASM_COMPILE_THREAD_NAME: &str = "wasm-compile";

/// Runs `work` on a thread with a stack large enough for cranelift to compile a
/// WASM module, and hands the result back. See [`WASM_COMPILE_STACK_BYTES`].
fn run_on_wasm_compile_thread<T, F>(work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    std::thread::Builder::new()
        .name(WASM_COMPILE_THREAD_NAME.into())
        .stack_size(WASM_COMPILE_STACK_BYTES)
        .spawn(work)
        .context("spawn the wasm compile thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("the wasm compile thread panicked"))?
}

pub(super) fn load_wasm_language(name: &str, bytes: &[u8]) -> Result<Language> {
    let compile_name = name.to_string();
    let compile_bytes = bytes.to_vec();
    // Loading a language compiles its WASM, so it needs the same large stack as
    // creating the store — see [`WASM_COMPILE_STACK_BYTES`].
    run_on_wasm_compile_thread(move || {
        WASM_STORE
            .lock()
            .expect("language extension wasm store mutex poisoned")
            .load_language(&compile_name, &compile_bytes)
            .map_err(anyhow::Error::msg)
    })
    .with_context(|| format!("load wasm language {name}"))
}

pub(super) fn parser_factory(
    name: &str,
    bytes: &[u8],
) -> gpui_component::highlighter::LanguageParserFactory {
    let name = name.to_string();
    let bytes = bytes.to_vec();
    Arc::new(move || {
        let name = name.clone();
        let bytes = bytes.clone();
        run_on_wasm_compile_thread(move || {
            let mut store = WasmStore::new(&ENGINE).map_err(anyhow::Error::msg)?;
            let language = store
                .load_language(&name, &bytes)
                .map_err(anyhow::Error::msg)?;
            let mut parser = Parser::new();
            parser.set_wasm_store(store)?;
            Ok((parser, language))
        })
    })
}

pub(crate) fn register_manifest(manifest: LanguageManifest, source_path: PathBuf, loaded: bool) {
    STATE.lock().unwrap().manifests.insert(
        manifest.name.clone(),
        RegisteredManifest {
            manifest,
            source_path,
            loaded,
        },
    );
}

pub(crate) fn replace_manifests(root: &Path, manifests: Vec<(LanguageManifest, PathBuf)>) {
    let mut state = STATE.lock().unwrap();
    let loaded = state
        .manifests
        .iter()
        .filter(|(_, registered)| registered.loaded)
        .map(|(name, _)| name.clone())
        .collect::<std::collections::HashSet<_>>();
    state
        .manifests
        .retain(|_, registered| registered.source_path.parent() != Some(root));
    for (manifest, source_path) in manifests {
        let was_loaded = loaded.contains(&manifest.name);
        state.manifests.insert(
            manifest.name.clone(),
            RegisteredManifest {
                manifest,
                source_path,
                loaded: was_loaded,
            },
        );
    }
}

pub(crate) fn forget(name: &str) {
    STATE.lock().unwrap().manifests.remove(name);
}

pub(super) fn load_registered(identifier: &str) -> Result<bool> {
    let _load_guard = LOAD_LOCK.lock().unwrap();
    let registered = {
        let state = STATE.lock().unwrap();
        state
            .manifests
            .get(identifier)
            .or_else(|| {
                state.manifests.values().find(|registered| {
                    registered.manifest.name.eq_ignore_ascii_case(identifier)
                        || registered
                            .manifest
                            .file_extensions
                            .iter()
                            .any(|extension| extension.eq_ignore_ascii_case(identifier))
                })
            })
            .cloned()
    };
    let Some(registered) = registered else {
        return Ok(false);
    };
    if registered.loaded
        && gpui_component::highlighter::LanguageRegistry::singleton()
            .language(&registered.manifest.name)
            .is_some()
    {
        return Ok(true);
    }
    super::InstalledExtension::load_from_dir(&registered.source_path)?
        .register(gpui_component::highlighter::LanguageRegistry::singleton())?;
    Ok(true)
}

pub(super) fn registered_language_name(identifier: &str) -> Option<String> {
    let identifier = identifier.trim().trim_start_matches('.');
    let state = STATE.lock().unwrap();
    state
        .manifests
        .get(identifier)
        .or_else(|| {
            state.manifests.values().find(|registered| {
                registered.manifest.name.eq_ignore_ascii_case(identifier)
                    || registered
                        .manifest
                        .file_extensions
                        .iter()
                        .any(|extension| extension.eq_ignore_ascii_case(identifier))
            })
        })
        .map(|registered| registered.manifest.name.clone())
}

#[cfg(test)]
mod stack_tests {
    use super::*;

    /// libdispatch worker 线程的栈实测只有 536 576 字节（在 global queue 上用
    /// `pthread_get_stacksize_np` 测得），GPUI 的 background executor 就跑在
    /// 这类线程上。cranelift 编译 tree-sitter 的 store 模块需要 0.5–1 MiB 栈，
    /// 在 512 KiB 上就会撞破线程栈保护页，再被 wasmtime 的 macOS 信号处理器
    /// 转成 SIGILL（Bus error: 10）——见崩溃报告 navop-2026-10-12-181737.ips。
    const GPUI_BACKGROUND_STACK_BYTES: usize = 536_576;

    /// Probe helper: `NAVOP_WASM_STORE_TEST_STACK` overrides the stack size so the
    /// overflow threshold can be measured without touching the default.
    fn probe_stack_bytes() -> usize {
        std::env::var("NAVOP_WASM_STORE_TEST_STACK")
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(GPUI_BACKGROUND_STACK_BYTES)
    }

    #[test]
    fn parser_factory_survives_a_gpui_sized_stack() {
        let stack_bytes = probe_stack_bytes();
        eprintln!("probe stack bytes: {stack_bytes}");
        // 走真实入口 `parser_factory`：闭包里的 `WasmStore::new` 正是崩溃点。
        // 这里用一段非法 wasm，`load_language` 必然报错，但创建 store 时的
        // cranelift 编译仍会跑完——修复前它就在本测试线程上溢出。
        let factory = parser_factory("probe-language", b"\0asm\x01\0\0\0");

        let handle = std::thread::Builder::new()
            .name("test-gpui-background-stack".into())
            .stack_size(stack_bytes)
            .spawn(move || factory().map(|_| ()))
            .expect("spawn the probe stack thread");

        // 崩溃时进程直接终止，join 不会返回 Err，只会以 SIGILL 结束。
        let result = handle
            .join()
            .expect("the probe thread terminated abnormally");

        assert!(
            result.is_err(),
            "the invalid wasm fixture is expected to fail loading"
        );
    }
}
