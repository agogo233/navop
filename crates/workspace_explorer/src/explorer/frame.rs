use super::WorkspaceExplorer;
use gpui::{Context, EventEmitter};

/// 面板在宿主侧边栏中的停靠位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerFramePlacement {
    Left,
    Right,
    Bottom,
}

/// 工作区浏览器向宿主发出的框架事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceExplorerEvent {
    Close,
    MoveTo(ExplorerFramePlacement),
    SyncTerminalCwd,
    RootChanged(std::path::PathBuf),
    /// 当前根目录的 Git 仓库句柄变了（认出仓库 / 换仓库 / 不再是仓库）。
    ///
    /// 仓库是**异步**发现的：装机时 `repository()` 是 `None`，首帧工作区快照
    /// 落地后才有值。宿主若在装机那一帧读过一次就缓存结论，没有这个事件就永远
    /// 等不到第二次机会（典型表现：底栏的分支 / worktree 入口一直是空的）。
    ///
    /// 不带载荷：订阅方需要什么直接问 [`super::WorkspaceExplorer::repository`]。
    RepositoryChanged,
    /// 请求宿主用已配置的 LLM 生成提交信息（Explorer 不依赖 LLM 层）。
    CommitMessageRequested,
    /// 用户请求查看一个文档（点开文件或 Git 变更）。宿主工作台借此把
    /// 审阅面板带到前台——Explorer 只知道编辑器实体，不知道落位布局。
    DocumentRequested,
    /// 某个会话「可以回滚到的轮次」列表发生变化：快照刚锚定成功，或回滚后把
    /// 后续轮次截掉了。
    ///
    /// 这里送**全量**列表而不是增量，因为快照由 Explorer 独占：宿主只需要把看到的
    /// 结果原样灌进视图，不用自己维护一份可能漂移的镜像。
    RestorableTurnsChanged {
        session_id: String,
        turn_ids: Vec<String>,
    },
    /// 某个会话「留下了整轮审阅 diff」的轮次列表发生变化。
    ///
    /// 与 [`Self::RestorableTurnsChanged`] **分开**：回滚入口只对锚定成功的轮次出现，
    /// 审阅入口只要 diff 真算出来了就该在——轮次 id 不合规导致 ref 锚不上时，
    /// 「回到这一轮」不该出现，但「查看改动」没有理由跟着一起消失。
    ///
    /// 同样送全量列表：快照由 Explorer 独占，宿主不用维护可能漂移的镜像。
    ReviewsChanged {
        session_id: String,
        turn_ids: Vec<String>,
    },
    /// 刚才那一轮的整轮 diff 已捕获，请宿主把它摆到审阅面板。
    ///
    /// 由**宿主**驱动而不是 Explorer 自己在 render 里打开：Explorer 只在它自己的
    /// 面板被渲染时才有 render，用户把文件面板关掉时，那一刻的自动打开永远不会
    /// 发生——渲染不到就发不出事件，等于谁都没动。
    TurnReviewCaptured {
        session_id: String,
        turn_id: String,
    },
}

impl EventEmitter<WorkspaceExplorerEvent> for WorkspaceExplorer {}

impl WorkspaceExplorer {
    pub fn set_frame_placement(
        &mut self,
        placement: ExplorerFramePlacement,
        cx: &mut Context<Self>,
    ) {
        if self.frame_placement == placement {
            return;
        }
        self.frame_placement = placement;
        cx.notify();
    }

    /// 切换点文件可见性，并按新过滤规则重建列表。
    pub fn toggle_show_hidden(&mut self, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        self.rebuild_file_tree(cx);
    }

    /// 切换 Git ignored 文件可见性，并按新过滤规则重建列表。
    pub fn toggle_show_ignored(&mut self, cx: &mut Context<Self>) {
        self.show_ignored = !self.show_ignored;
        self.rebuild_file_tree(cx);
    }

    fn rebuild_file_tree(&mut self, cx: &mut Context<Self>) {
        self.listings.clear();
        self.expanded.clear();
        self.loading_directories.clear();
        self.selected_path = None;
        self.refresh(cx);
    }
}
