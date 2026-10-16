//! SQL 库：跨连接的 SQL 文档管理面板（侧边栏）。
//!
//! - [`tree`]：纯状态的库树（根目录 + 展开状态 + 子项缓存），可单独单测。
//! - [`view`]：头部工具栏与树行的渲染。
//! - [`panel`]：面板实体，负责文件操作与对外事件。

mod panel;
mod tree;
mod view;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use panel::{SqlLibraryEvent, SqlLibraryPanel};
