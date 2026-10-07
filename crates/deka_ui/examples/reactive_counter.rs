use deka_ui::prelude::*;

fn main() {
    let scope = Scope::new();
    let mut count = scope.run(|| {
        let count = signal(0);
        let doubled = derived(move || count.get().unwrap_or_default() * 2);
        effect(move || println!("Count: {count}, doubled: {doubled}"));
        count
    });
    scope.batch(|| {
        count += 1;
        count += 1;
    });
    count -= 1;
}
