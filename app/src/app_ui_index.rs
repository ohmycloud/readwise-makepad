//! Projection-local lookup of fixed app controls. Unlike repeated global tree
//! searches, one walk also makes missing (uninstantiated) controls cheap. The
//! reader's projection context owns this index only for a synchronous pass; no
//! areas, geometry or paragraph caches are retained across events/reloads.
use makepad_widgets::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct AppUiIndex {
    root: WidgetRef,
    named: RefCell<Option<HashMap<LiveId, WidgetRef>>>,
}

impl std::ops::Deref for AppUiIndex {
    type Target = WidgetRef;
    fn deref(&self) -> &WidgetRef {
        &self.root
    }
}

impl AppUiIndex {
    pub fn begin_projection(&mut self, root: &WidgetRef) {
        self.root = root.clone();
        *self.named.get_mut() = None;
    }

    pub fn widget(&self, cx: &Cx, path: &[LiveId]) -> WidgetRef {
        // Explicit/scoped paths must keep Makepad's own path semantics.
        if path.len() != 1 {
            return self.root.widget(cx, path);
        }
        let mut named = self.named.borrow_mut();
        let index = named.get_or_insert_with(|| {
            let mut index = HashMap::new();
            let mut seen = HashSet::new();
            let mut stack = Vec::new();
            self.root
                .children(&mut |name, widget| stack.push((name, widget)));
            stack.reverse();
            while let Some((name, widget)) = stack.pop() {
                if widget.is_empty() || !seen.insert(widget.widget_uid()) {
                    continue;
                }
                index.entry(name).or_insert_with(|| widget.clone());
                // Match Makepad: the skipped node itself remains addressable,
                // but paragraph/list item descendants are locally owned.
                if widget.skip_widget_tree_search() {
                    continue;
                }
                let mut children = Vec::new();
                widget.children(&mut |name, child| children.push((name, child)));
                stack.extend(children.into_iter().rev());
            }
            index
        });
        index.get(&path[0]).cloned().unwrap_or_default()
    }

    pub fn label(&self, cx: &Cx, path: &[LiveId]) -> LabelRef {
        self.widget(cx, path).as_label()
    }
    pub fn view(&self, cx: &Cx, path: &[LiveId]) -> ViewRef {
        self.widget(cx, path).as_view()
    }
    pub fn check_box(&self, cx: &Cx, path: &[LiveId]) -> CheckBoxRef {
        self.widget(cx, path).as_check_box()
    }
    pub fn radio_button(&self, cx: &Cx, path: &[LiveId]) -> RadioButtonRef {
        self.widget(cx, path).as_radio_button()
    }
    pub fn slider(&self, cx: &Cx, path: &[LiveId]) -> SliderRef {
        self.widget(cx, path).as_slider()
    }
    pub fn drop_down(&self, cx: &Cx, path: &[LiveId]) -> DropDownRef {
        self.widget(cx, path).as_drop_down()
    }
    pub fn text_input(&self, cx: &Cx, path: &[LiveId]) -> TextInputRef {
        self.widget(cx, path).as_text_input()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Node {
        uid: WidgetUid,
        skip: bool,
        children: Vec<(LiveId, WidgetRef)>,
    }
    impl ScriptApply for Node {
        fn script_apply(&mut self, _: &mut ScriptVm, _: &Apply, _: &mut Scope, _: ScriptValue) {}
    }
    impl WidgetNode for Node {
        fn widget_uid(&self) -> WidgetUid {
            self.uid
        }
        fn children(&self, visit: &mut dyn FnMut(LiveId, WidgetRef)) {
            for (name, child) in &self.children {
                visit(*name, child.clone());
            }
        }
        fn skip_widget_tree_search(&self) -> bool {
            self.skip
        }
        fn walk(&mut self, _: &mut Cx) -> Walk {
            Walk::default()
        }
        fn area(&self) -> Area {
            Area::Empty
        }
        fn redraw(&mut self, _: &mut Cx) {}
    }
    impl Widget for Node {
        fn draw_walk(&mut self, _: &mut Cx2d, _: &mut Scope, _: Walk) -> DrawStep {
            DrawStep::done()
        }
    }
    fn node(uid: u64, skip: bool, children: Vec<(LiveId, WidgetRef)>) -> WidgetRef {
        WidgetRef::new_with_inner(Box::new(Node {
            uid: WidgetUid(uid),
            skip,
            children,
        }))
    }
    #[test]
    fn index_preserves_first_match_and_list_item_ownership() {
        let cx = Cx::new(Box::new(|_, _| {}));
        let first = node(3, false, vec![]);
        let list = node(5, true, vec![(id!(paragraph), node(6, false, vec![]))]);
        let root = node(
            1,
            false,
            vec![
                (
                    id!(container),
                    node(2, false, vec![(id!(control), first.clone())]),
                ),
                (id!(control), node(4, false, vec![])),
                (id!(list), list.clone()),
            ],
        );
        let mut index = AppUiIndex::default();
        index.begin_projection(&root);
        assert_eq!(index.widget(&cx, ids!(control)), first);
        assert_eq!(index.widget(&cx, ids!(list)), list);
        assert!(index.widget(&cx, ids!(paragraph)).is_empty());
        assert!(index.widget(&cx, ids!(missing)).is_empty());
    }
    #[test]
    fn a_new_projection_rebuilds_handles_including_previous_misses() {
        let cx = Cx::new(Box::new(|_, _| {}));
        let mut index = AppUiIndex::default();
        index.begin_projection(&node(1, false, vec![]));
        assert!(index.widget(&cx, ids!(control)).is_empty());
        let replacement = node(3, false, vec![]);
        index.begin_projection(&node(2, false, vec![(id!(control), replacement.clone())]));
        assert_eq!(index.widget(&cx, ids!(control)), replacement);
    }
}
