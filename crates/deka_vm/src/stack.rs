// Recovered from PHPX 61c262e8, crates/php-rs/src/vm/stack.rs.
use crate::heap::Handle;

#[derive(Debug, Default)]
pub(crate) struct Stack {
    values: Vec<Handle>,
}

impl Stack {
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    pub fn push(&mut self, h: Handle) {
        self.values.push(h);
    }

    pub fn pop(&mut self) -> Option<Handle> {
        self.values.pop()
    }

    pub fn roots(&self) -> impl Iterator<Item = Handle> + '_ {
        self.values.iter().copied()
    }
}
