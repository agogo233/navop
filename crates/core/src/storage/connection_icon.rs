//! 连接的图标解析:把连接设置里的图标选择转换成可渲染的 `Icon`。
//!
//! 列表、卡片与标签页共用同一套优先级:
//! 本地图片文件(存在时) → 内置图标 ID → 调用方回退(扩展 manifest 图标 / 类型图标)。

use std::path::Path;

use gpui_component::Icon;
use serde_json::{Map, Value};

use super::models::{SSH_ICON_IDS, SshParams, StoredConnection, ssh_os_icon};

/// 本地图标文件的资产命名空间:`local-icon/{path}`。
///
/// 不能直接交绝对路径:Windows 盘符(`C:\..`)会被 `url::Url::parse`
/// 判成 scheme = `c`,gpui 会当 HTTP 资源去请求。加一层目录前缀后仍是相对路径,
/// 由应用 `AssetSource` 直接读盘,SVG 经图像解码保留原色。
pub const LOCAL_ICON_ASSET_PREFIX: &str = "local-icon/";

/// 本地图标文件路径 → 无 scheme 的资产路径。
pub fn local_icon_asset_path(path: &Path) -> String {
    format!("{LOCAL_ICON_ASSET_PREFIX}{}", path.display())
}

/// 内置图标 ID → 原色 `Icon`;未知 ID 视为未选择。
pub fn builtin_icon(id: &str) -> Option<Icon> {
    SSH_ICON_IDS
        .contains(&id)
        .then(|| ssh_os_icon(Some(id)).color())
}

/// 本地图片文件 → 原色 `Icon`;文件不存在时返回 `None`。
pub fn file_icon(path: &Path) -> Option<Icon> {
    path.is_file()
        .then(|| Icon::default().path(local_icon_asset_path(path)).color())
}

/// SSH 连接设置的图标:自定义图片优先,其次手动选择 / 探测到的系统图标。
pub fn ssh_connection_icon(params: &SshParams) -> Icon {
    params
        .icon_file_path
        .as_deref()
        .map(Path::new)
        .and_then(file_icon)
        .unwrap_or_else(|| params.os_icon().color())
}

/// 扩展连接表单里 `Icon` 字段的选择:`{"file": "<path>"}` 优先,其次 `{"icon": "<id>"}`。
///
/// 字段 ID 由扩展决定,所以这里只认存储形态、不认字段名;返回 `None`
/// 表示用户没有选择,调用方回退到扩展 manifest 声明的图标。
pub fn extension_icon_override(config: &Map<String, Value>) -> Option<Icon> {
    let entries = || config.values().filter_map(|value| value.as_object());
    entries()
        .filter_map(|object| object.get("file")?.as_str().map(Path::new))
        .find_map(file_icon)
        .or_else(|| {
            entries()
                .filter_map(|object| object.get("icon")?.as_str())
                .find_map(builtin_icon)
        })
}

/// 连接自身保存的图标选择(SSH / 扩展);其他连接类型返回 `None`,
/// 由调用方按连接类型回退。
pub fn stored_connection_icon(connection: &StoredConnection) -> Option<Icon> {
    match connection.connection_type {
        super::models::ConnectionType::SshSftp => connection
            .to_ssh_params()
            .ok()
            .map(|params| ssh_connection_icon(&params)),
        super::models::ConnectionType::Extension => connection
            .to_extension_params()
            .ok()
            .and_then(|params| extension_icon_override(&params.config)),
        _ => None,
    }
}
