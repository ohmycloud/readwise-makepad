//! Update existing paint properties without evaluating Splash or recompiling shaders.
//! These value-only packets use the same apply path as Animator snapshots. They
//! must not add shader fields, replace templates, or change layout/font styles.
use makepad_widgets::*;

fn set_path(vm: &mut ScriptVm, root: ScriptObject, path: &[LiveId], value: ScriptValue) {
    let Some((&last, parents)) = path.split_last() else {
        return;
    };
    let mut parent = root;
    for &key in parents {
        let existing = vm.bx.heap.value(parent, key.into(), NoTrap);
        parent = if let Some(child) = existing.as_object() {
            child
        } else {
            let child = vm.bx.heap.new_object();
            vm.bx.heap.set_value_def(parent, key.into(), child.into());
            child
        };
    }
    vm.bx.heap.set_value_def(parent, last.into(), value);
}

pub fn apply_widget_paint(
    cx: &mut Cx,
    mut widget: WidgetRef,
    colors: &[(&[LiveId], Vec4f)],
    numbers: &[(&[LiveId], f64)],
) {
    if widget.is_empty() {
        return;
    }
    cx.with_vm(|vm| {
        let root = vm.bx.heap.new_object();
        for &(path, color) in colors {
            let value = color.script_to_value(vm);
            set_path(vm, root, path, value);
        }
        for &(path, number) in numbers {
            set_path(vm, root, path, number.into());
        }
        widget.script_apply(vm, &Apply::Animate, &mut Scope::empty(), root.into());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Script, ScriptHook)]
    struct Probe {
        #[live]
        color: Vec4f,
        #[live]
        ring: f64,
        #[live]
        width: f64,
        #[source]
        source: ScriptObjectRef,
    }

    #[test]
    fn paint_packets_share_nested_objects_and_preserve_unmentioned_layout_and_source() {
        let mut host = ();
        let mut std = ();
        let mut vm = ScriptVm {
            host: &mut host,
            std: &mut std,
            bx: Box::new(ScriptVmBase::new()),
        };
        let source = vm.bx.heap.new_object();
        let mut probe = Probe {
            color: Vec4f::default(),
            ring: 0.0,
            width: 44.0,
            source: vm.bx.heap.new_object_ref(source),
        };
        let root = vm.bx.heap.new_object();
        let color = Vec4f::from_u32(0xdf8267ff);
        let value = color.script_to_value(&mut vm);
        set_path(&mut vm, root, ids!(draw_bg.color), value);
        set_path(&mut vm, root, ids!(draw_bg.ring), 1.4.into());
        let child = vm.bx.heap.value(root, id!(draw_bg).into(), NoTrap);
        probe.script_apply(&mut vm, &Apply::Animate, &mut Scope::empty(), child);
        assert_eq!(probe.color, color);
        assert_eq!(probe.ring, 1.4);
        assert_eq!(probe.width, 44.0);
        assert_eq!(probe.source.as_object(), source);
    }
}
