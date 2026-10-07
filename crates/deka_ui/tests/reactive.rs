use deka_ui::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn copy_handles_events_arithmetic_toggle_and_display_are_reactive() {
    let scope = Scope::new();
    let observations = Rc::new(RefCell::new(Vec::new()));
    let (mut count, visible) = scope.run(|| {
        let count = signal(0);
        let visible = signal(false);
        let output = observations.clone();
        effect(move || output.borrow_mut().push(format!("{count}:{visible}")));
        (count, visible)
    });
    let alias = count;
    scope.batch(|| {
        count += 3;
        count -= 1;
        visible.toggle();
    });
    assert_eq!(alias.get(), 2);
    assert_eq!(*observations.borrow(), ["0:false", "2:true"]);
    count.update(|value| *value *= 2);
    assert_eq!(*observations.borrow(), ["0:false", "2:true", "4:true"]);
}

#[test]
fn dependencies_follow_branches_and_unobserved_signals_do_no_work() {
    let scope = Scope::new();
    let output = Rc::new(RefCell::new(Vec::new()));
    let (left, right, choose, unused) = scope.run(|| {
        let left = signal(1);
        let right = signal(10);
        let choose = signal(true);
        let unused = signal(0);
        let output = output.clone();
        effect(move || {
            output.borrow_mut().push(if choose.get() {
                left.get()
            } else {
                right.get()
            })
        });
        (left, right, choose, unused)
    });
    right.set(20);
    unused.set(10);
    assert_eq!(*output.borrow(), [1]);
    left.set(2);
    choose.toggle();
    left.set(3);
    right.set(21);
    assert_eq!(*output.borrow(), [1, 2, 20, 21]);
}

#[test]
fn derived_values_cache_suppress_equal_results_and_settle_diamonds() {
    let scope = Scope::new();
    let output = Rc::new(RefCell::new(Vec::new()));
    let computations = Rc::new(Cell::new(0));
    let input = scope.run(|| {
        let input = signal(1);
        let computations = computations.clone();
        let left = derived(move || {
            computations.set(computations.get() + 1);
            input.get() * 2
        });
        let right = derived(move || input.get() + 1);
        let sum = derived(move || left.get() + right.get());
        let parity = derived(move || input.get() % 2);
        let output = output.clone();
        effect(move || output.borrow_mut().push((sum.get(), parity.get())));
        assert_eq!(left.get(), 2);
        assert_eq!(left.get(), 2);
        input
    });
    assert_eq!(computations.get(), 1);
    scope.batch(|| {
        input.set(2);
        input.set(3);
    });
    assert_eq!(computations.get(), 2);
    assert_eq!(*output.borrow(), [(4, 1), (10, 1)]);
    input.set(3);
    assert_eq!(*output.borrow(), [(4, 1), (10, 1)]);
}

#[test]
fn derived_unchanged_values_do_not_notify_downstream_effects() {
    let scope = Scope::new();
    let calls = Rc::new(Cell::new(0));
    let input = scope.run(|| {
        let input = signal(1);
        let parity = derived(move || input.get() % 2);
        let calls = calls.clone();
        effect(move || {
            let _ = parity.get();
            calls.set(calls.get() + 1);
        });
        input
    });
    input.set(3);
    assert_eq!(calls.get(), 1);
    input.set(4);
    assert_eq!(calls.get(), 2);
}

#[test]
fn nested_batches_and_nested_reactions_preserve_the_outer_read_capture() {
    let scope = Scope::new();
    let outer_runs = Rc::new(Cell::new(0));
    let inner_runs = Rc::new(Cell::new(0));
    let (outer, inner) = scope.run(|| {
        let outer = signal(0);
        let inner = signal(0);
        let outer_runs = outer_runs.clone();
        let inner_runs = inner_runs.clone();
        effect(move || {
            if outer_runs.get() == 0 {
                let inner_runs = inner_runs.clone();
                effect(move || {
                    inner.get();
                    inner_runs.set(inner_runs.get() + 1);
                });
            }
            outer.get();
            outer_runs.set(outer_runs.get() + 1);
        });
        (outer, inner)
    });
    scope.batch(|| {
        inner.set(1);
        batch(|| {
            inner.set(2);
            inner.set(3);
        });
    });
    assert_eq!(outer_runs.get(), 1);
    assert_eq!(inner_runs.get(), 2);
    outer.set(1);
    assert_eq!(outer_runs.get(), 2);
    assert_eq!(inner_runs.get(), 2);
}

#[test]
fn effect_disposal_and_scope_drop_release_reactions_and_values() {
    let scope = Scope::new();
    let value = Rc::new(());
    let weak_value = Rc::downgrade(&value);
    let captured = Rc::new(());
    let weak_capture = Rc::downgrade(&captured);
    let calls = Rc::new(Cell::new(0));
    let (state, registration, retained) = scope.run(|| {
        let state = signal(0);
        let retained = signal(value);
        let calls = calls.clone();
        let registration = effect(move || {
            state.get();
            let _ = &captured;
            calls.set(calls.get() + 1);
        });
        (state, registration, retained)
    });
    registration.dispose();
    registration.dispose();
    state.set(1);
    assert_eq!(calls.get(), 1);
    assert!(weak_capture.upgrade().is_none());
    drop(scope);
    assert!(weak_value.upgrade().is_none());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| retained.get())).is_err());
    let replacement = Scope::new();
    replacement.run(|| {
        signal(42);
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.get())).is_err());
}

#[test]
fn panic_restores_context_batch_depth_and_pending_reactions() {
    let scope = Scope::new();
    let observations = Rc::new(RefCell::new(Vec::new()));
    let state = scope.run(|| {
        let state = signal(0);
        let observations = observations.clone();
        effect(move || observations.borrow_mut().push(state.get()));
        state
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scope.batch(|| {
            state.update(|value| {
                *value = 7;
                panic!("event failed");
            })
        });
    }));
    assert!(result.is_err());
    assert_eq!(*observations.borrow(), [0]);
    scope.flush();
    assert_eq!(*observations.borrow(), [0, 7]);
    state.set(8);
    assert_eq!(*observations.borrow(), [0, 7, 8]);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| signal(0))).is_err());
}

#[test]
fn separate_scopes_cannot_accidentally_alias_or_cross_subscribe() {
    let first = Scope::new();
    let second = Scope::new();
    let a = first.run(|| signal(1));
    let b = second.run(|| signal(2));
    a.set(3);
    assert_eq!(b.get(), 2);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        second.run(|| {
            effect(move || {
                a.get();
            })
        });
    }));
    assert!(result.is_err());
    second.run(|| {
        effect(move || {
            b.get();
        })
    });
    b.set(4);
    assert_eq!(b.get(), 4);
}

#[test]
fn event_effect_can_update_another_signal_without_reentrant_borrows() {
    let scope = Scope::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let input = scope.run(|| {
        let input = signal(1);
        let output = signal(0);
        effect(move || output.set(input.get() * 2));
        let seen = seen.clone();
        effect(move || seen.borrow_mut().push(output.get()));
        input
    });
    input.set(3);
    assert_eq!(*seen.borrow(), [2, 6]);
}

#[test]
fn effect_writes_during_first_execution_reschedule_until_settled() {
    let scope = Scope::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let state = scope.run(|| {
        let state = signal(0);
        let seen = seen.clone();
        effect(move || {
            let next = state.get();
            seen.borrow_mut().push(next);
            if next < 3 {
                state.set(next + 1);
            }
        });
        state
    });
    assert_eq!(state.get(), 3);
    assert_eq!(*seen.borrow(), [0, 1, 2, 3]);
}

#[test]
fn a_cycle_reports_failure_and_the_scope_can_recover() {
    let scope = Scope::new();
    let state = scope.run(|| signal(0));
    let armed = scope.run(|| signal(false));
    let registration = scope.run(|| {
        effect(move || {
            if armed.get() {
                state.set(state.get() + 1);
            }
        })
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| armed.set(true)));
    assert!(result.is_err());
    registration.dispose();
    scope.flush();
    state.set(1);
    assert_eq!(state.get(), 1);
}

#[test]
fn handles_are_copy_even_for_non_copy_values_and_update_returns_a_result() {
    let scope = Scope::new();
    let title = scope.run(|| signal(String::from("Ready")));
    let alias = title;
    let previous_length = title.update(|text| {
        let len = text.len();
        text.push('!');
        len
    });
    assert_eq!(previous_length, 5);
    assert_eq!(alias.get(), "Ready!");
    title.set(String::from("Done"));
    assert_eq!(alias.get(), "Done");
}

#[test]
fn effects_registered_before_a_later_derived_value_still_see_settled_state() {
    let scope = Scope::new();
    let later = Rc::new(RefCell::new(None::<Derived<i32>>));
    let observations = Rc::new(RefCell::new(Vec::new()));
    let input = scope.run(|| {
        let input = signal(1);
        let later_read = later.clone();
        let observations = observations.clone();
        effect(move || {
            let result = input.get() + later_read.borrow().as_ref().map_or(0, |value| value.get());
            observations.borrow_mut().push(result);
        });
        *later.borrow_mut() = Some(derived(move || input.get() * 2));
        input
    });
    input.set(2);
    assert_eq!(*observations.borrow(), [1, 6]);
}
