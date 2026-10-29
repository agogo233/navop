use std::collections::HashMap;

use gpui::{AppContext as _, Context, Entity};
use gpui_component::resizable::ResizableState;

use super::split_model::{TerminalSplitChild, TerminalSplitId, TerminalSplitTree};

/// 分屏窗格尺寸的宿主。
///
/// gpui 的 `h_resizable` / `v_resizable` 在未显式传入 state 时,把分隔比例挂在
/// 窗口的元素缓存里。元素一旦不再渲染——切到别的标签页时就是如此——缓存被丢弃,
/// 切回来窗格尺寸便退回默认均分。这里让工作区自己持有每个分组的 `ResizableState`
/// 实体:标签页内容实体存活期间尺寸就存活,与 `DockArea` 由宿主持有布局的做法一致。
pub(super) struct TerminalSplitSizing {
    groups: HashMap<TerminalSplitId, GroupSizing>,
}

struct GroupSizing {
    state: Entity<ResizableState>,
    /// state 里各槽位对应的子节点,用来在结构变化时把尺寸搬到正确的槽位。
    children: Vec<TerminalSplitChild>,
}

impl TerminalSplitSizing {
    pub(super) fn new() -> Self {
        Self {
            groups: HashMap::new(),
        }
    }

    /// 取分组对应的 state;首次出现时创建并登记它的子节点。
    pub(super) fn state<T: 'static>(
        &mut self,
        split_id: TerminalSplitId,
        children: Vec<TerminalSplitChild>,
        cx: &mut Context<T>,
    ) -> Entity<ResizableState> {
        if let Some(group) = self.groups.get(&split_id) {
            return group.state.clone();
        }
        let state = cx.new(|_| ResizableState::default());
        self.groups.insert(
            split_id,
            GroupSizing {
                state: state.clone(),
                children,
            },
        );
        state
    }

    /// 按最新的拆分树对齐宿主记录的布局。
    ///
    /// 只处理结构确实变化的分组:新建分组拿一份空 state(交给首次渲染均分),
    /// 存活分组按子节点增删精确搬运槽位尺寸,已删除分组连同 state 一并丢弃。
    pub(super) fn reconcile<T: 'static>(&mut self, tree: &TerminalSplitTree, cx: &mut Context<T>) {
        let live = tree.groups();
        self.groups
            .retain(|split_id, _| live.iter().any(|(live_id, _)| live_id == split_id));

        for (split_id, children) in live {
            match self.groups.get_mut(&split_id) {
                Some(group) if group.children == children => {}
                Some(group) => {
                    let synced = group.state.read(cx).sizes().len() == group.children.len();
                    if synced {
                        reconcile_group(group, children, cx);
                    } else {
                        // 记录与实体对不上(例如分组还没渲染过就改了结构):
                        // 丢掉旧比例,交给下一次渲染重新均分。
                        group.state.update(cx, |state, _| state.clear());
                        group.children = children;
                    }
                }
                None => {}
            }
        }
    }
}

fn reconcile_group<T: 'static>(
    group: &mut GroupSizing,
    children: Vec<TerminalSplitChild>,
    cx: &mut Context<T>,
) {
    let state = group.state.clone();
    let mut current = group.children.clone();

    for index in (0..current.len()).rev() {
        if !children.contains(&current[index]) {
            state.update(cx, |state, cx| state.remove_panel(index, cx));
            current.remove(index);
        }
    }
    for (index, child) in children.iter().enumerate() {
        if !current.contains(child) {
            state.update(cx, |state, cx| state.insert_panel(None, Some(index), cx));
            current.insert(index, *child);
        }
    }

    group.children = children;
}
