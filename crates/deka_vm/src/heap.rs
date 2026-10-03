use crate::{Literal, Result};
use std::collections::BTreeMap;

// PHPX used a u32 index into Vec<Zval>. Add generations and actual tracing:
// free slots now drop their payload immediately and stale handles cannot alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Handle {
    index: usize,
    generation: u64,
}
#[derive(Clone, Debug)]
pub(crate) enum Value {
    Unit,
    Uninitialized,
    Descriptor(crate::TypeDescriptor),
    Number(f64),
    Bool(bool),
    String(String),
    List(Vec<Handle>),
    Record(Record),
    // Component attributes are getters; nested children are retained values.
    Props(BTreeMap<String, Handle>),
    Cell(Handle),
    Closure {
        function: usize,
        captures: Vec<Handle>,
        slot_children: Option<Handle>,
    },
    Promise(Option<Result<Handle>>),
}
/// Ordinary records and nominal structs share field storage. Struct identity
/// and embed names come from bytecode, never from the shape of user data.
#[derive(Clone, Debug, Default)]
pub(crate) struct Record {
    pub fields: BTreeMap<String, Handle>,
    pub struct_name: Option<String>,
    pub enum_name: Option<String>,
    pub order: Vec<String>,
    pub embeds: Vec<String>,
}
impl std::ops::Deref for Record {
    type Target = BTreeMap<String, Handle>;
    fn deref(&self) -> &Self::Target {
        &self.fields
    }
}
impl std::ops::DerefMut for Record {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.fields
    }
}
impl FromIterator<(String, Handle)> for Record {
    fn from_iter<T: IntoIterator<Item = (String, Handle)>>(iter: T) -> Self {
        let mut record = Self::default();
        for (name, value) in iter {
            if !record.fields.contains_key(&name) {
                record.order.push(name.clone());
            }
            record.fields.insert(name, value);
        }
        record
    }
}
impl From<Literal> for Value {
    fn from(v: Literal) -> Self {
        match v {
            Literal::Unit => Self::Unit,
            Literal::Uninitialized => Self::Uninitialized,
            Literal::Number(n) => Self::Number(n),
            Literal::Bool(b) => Self::Bool(b),
            Literal::String(s) => Self::String(s),
        }
    }
}
struct Slot {
    value: Option<Value>,
    generation: u64,
    marked: bool,
    newtype: Option<String>,
}
#[derive(Default)]
pub(crate) struct Heap {
    slots: Vec<Slot>,
    free: Vec<usize>,
    pub collections: usize,
    allocations: usize,
}
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct HeapStats {
    pub live: usize,
    pub slots: usize,
    pub collections: usize,
    pub allocations: usize,
}
impl Heap {
    pub fn alloc(&mut self, value: Value) -> Handle {
        self.allocations += 1;
        let index = if let Some(i) = self.free.pop() {
            self.slots[i].value = Some(value);
            self.slots[i].newtype = None;
            i
        } else {
            self.slots.push(Slot {
                value: Some(value),
                generation: 0,
                marked: false,
                newtype: None,
            });
            self.slots.len() - 1
        };
        Handle {
            index,
            generation: self.slots[index].generation,
        }
    }
    pub fn alloc_newtype(&mut self, value: Value, name: String) -> Handle {
        let h = self.alloc(value);
        self.slots[h.index].newtype = Some(name);
        h
    }
    pub fn newtype_name(&self, h: Handle) -> Result<Option<&str>> {
        self.get(h)?;
        Ok(self.slots[h.index].newtype.as_deref())
    }
    pub fn get(&self, h: Handle) -> Result<&Value> {
        self.slots
            .get(h.index)
            .filter(|s| s.generation == h.generation)
            .and_then(|s| s.value.as_ref())
            .ok_or_else(|| "stale heap handle".into())
    }
    pub fn get_mut(&mut self, h: Handle) -> Result<&mut Value> {
        self.slots
            .get_mut(h.index)
            .filter(|s| s.generation == h.generation)
            .and_then(|s| s.value.as_mut())
            .ok_or_else(|| "stale heap handle".into())
    }
    pub fn replace(&mut self, h: Handle, value: Value) -> Result<()> {
        self.get(h)?;
        self.slots[h.index].value = Some(value);
        Ok(())
    }
    pub fn stats(&self) -> HeapStats {
        HeapStats {
            live: self.slots.len() - self.free.len(),
            slots: self.slots.len(),
            collections: self.collections,
            allocations: self.allocations,
        }
    }
    pub fn collect(&mut self, roots: impl IntoIterator<Item = Handle>) -> Result<()> {
        let mut todo: Vec<_> = roots.into_iter().collect();
        while let Some(h) = todo.pop() {
            self.get(h)?;
            let slot = &mut self.slots[h.index];
            if slot.marked {
                continue;
            }
            slot.marked = true;
            match slot.value.as_ref().unwrap() {
                Value::Cell(h) | Value::Promise(Some(Ok(h))) => todo.push(*h),
                Value::List(items) => todo.extend(items),
                Value::Closure {
                    captures,
                    slot_children,
                    ..
                } => {
                    todo.extend(captures);
                    todo.extend(slot_children);
                }
                Value::Record(fields) => todo.extend(fields.values()),
                Value::Props(fields) => todo.extend(fields.values()),
                _ => {}
            }
        }
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.value.is_some() && !slot.marked {
                slot.value = None;
                slot.newtype = None;
                slot.generation = slot
                    .generation
                    .checked_add(1)
                    .ok_or("heap generation exhausted")?;
                self.free.push(i);
            }
            slot.marked = false;
        }
        self.collections += 1;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cycles_are_collected_and_stale_handles_rejected() {
        let mut heap = Heap::default();
        let a = heap.alloc(Value::Unit);
        let b = heap.alloc(Value::List(vec![a]));
        heap.replace(a, Value::List(vec![b])).unwrap();
        heap.collect([a]).unwrap();
        assert_eq!(heap.stats().live, 2);
        heap.collect([]).unwrap();
        assert_eq!(heap.stats().live, 0);
        heap.alloc(Value::Number(42.));
        heap.alloc(Value::Number(43.));
        assert!(heap.get(a).is_err());
        assert!(heap.get(b).is_err());
    }
}
