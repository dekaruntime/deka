use crate::{Literal, Result};
use std::collections::BTreeMap;

// PHPX used a u32 index into Vec<Zval>. Add generations and actual tracing:
// free slots now drop their payload immediately and stale handles cannot alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    Bytes(Vec<u8>),
    Host(crate::HostHandle),
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
    Promise(Option<Result<Outcome>>),
}
/// Language throws carry rooted values separately from VM faults.
#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    Value(Handle),
    Thrown(Handle),
}
/// Ordinary records and nominal structs share field storage. Struct identity
/// and embed names come from bytecode, never from the shape of user data.
#[derive(Clone, Debug, Default)]
pub(crate) struct Record {
    pub fields: BTreeMap<String, Handle>,
    pub struct_name: Option<String>,
    pub struct_identity: Option<String>,
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
    created: usize,
    revision: u64,
}
#[derive(Default)]
pub(crate) struct Heap {
    slots: Vec<Slot>,
    free: Vec<usize>,
    pub collections: usize,
    allocations: usize,
    reads: std::cell::RefCell<Option<Reads>>,
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
            self.slots[i].created = self.allocations;
            self.slots[i].revision = 0;
            i
        } else {
            self.slots.push(Slot {
                value: Some(value),
                generation: 0,
                marked: false,
                newtype: None,
                created: self.allocations,
                revision: 0,
            });
            self.slots.len() - 1
        };
        Handle {
            index,
            generation: self.slots[index].generation,
        }
    }
    /// All enum producers, including typed JSON hydration, use nominal metadata.
    pub fn alloc_enum(
        &mut self,
        name: String,
        case: String,
        index: usize,
        value: Option<Handle>,
    ) -> Handle {
        let label = self.alloc(Value::String(case));
        let index = self.alloc(Value::Number(index as f64));
        let mut record: Record = [("name".into(), label), ("index".into(), index)]
            .into_iter()
            .collect();
        if let Some(value) = value {
            record.order.push("value".into());
            record.insert("value".into(), value);
        }
        record.enum_name = Some(name);
        self.alloc(Value::Record(record))
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
        self.get(h)?;
        self.observe_write(h);
        self.slots[h.index].revision = self.slots[h.index].revision.wrapping_add(1);
        self.slots
            .get_mut(h.index)
            .filter(|s| s.generation == h.generation)
            .and_then(|s| s.value.as_mut())
            .ok_or_else(|| "stale heap handle".into())
    }
    pub fn replace(&mut self, h: Handle, value: Value) -> Result<()> {
        self.get(h)?;
        self.observe_write(h);
        self.slots[h.index].revision = self.slots[h.index].revision.wrapping_add(1);
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
        if let Some(reads) = self.reads.borrow().as_ref() {
            todo.extend(reads.dependencies.iter().flat_map(Dependency::roots));
        }
        while let Some(h) = todo.pop() {
            self.get(h)?;
            let slot = &mut self.slots[h.index];
            if slot.marked {
                continue;
            }
            slot.marked = true;
            match slot.value.as_ref().unwrap() {
                Value::Cell(h)
                | Value::Promise(Some(Ok(Outcome::Value(h) | Outcome::Thrown(h)))) => todo.push(*h),
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
/// Source reads, not tracing/adapter reads. Field/index snapshots invalidate only
/// that address; entity revisions cover operations consuming a whole container.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Read {
    Cell,
    Field(String),
    Index(usize),
    Length,
    Entity,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Observed {
    Address(Option<Handle>),
    Size(usize),
    Revision(u64),
}
#[derive(Clone)]
pub(crate) struct Dependency {
    object: Handle,
    read: Read,
    observed: Observed,
}
impl Dependency {
    pub(crate) fn roots(&self) -> Vec<Handle> {
        let mut roots = vec![self.object];
        if let Observed::Address(Some(h)) = self.observed {
            roots.push(h);
        }
        roots
    }
}
struct Reads {
    start: usize,
    dependencies: Vec<Dependency>,
    volatile: bool,
}
#[cfg(feature = "ui")]
#[derive(Clone, Default)]
pub(crate) struct Dependencies {
    values: Vec<Dependency>,
    volatile: bool,
}
#[cfg(feature = "ui")]
impl Dependencies {
    #[cfg(feature = "ui")]
    pub(crate) fn roots(&self) -> Vec<Handle> {
        self.values.iter().flat_map(Dependency::roots).collect()
    }
    #[cfg(feature = "ui")]
    pub(crate) fn dirty(&self, heap: &Heap) -> bool {
        self.volatile
            || self
                .values
                .iter()
                .any(|d| heap.observed(d.object, &d.read).ok().as_ref() != Some(&d.observed))
    }
}
impl Heap {
    fn observed(&self, object: Handle, read: &Read) -> Result<Observed> {
        let value = self.get(object)?;
        Ok(match (read, value) {
            (Read::Cell, Value::Cell(h)) => Observed::Address(Some(*h)),
            (Read::Field(name), Value::Record(fields)) => {
                Observed::Address(fields.get(name).copied())
            }
            (Read::Field(name), Value::Props(fields)) => {
                Observed::Address(fields.get(name).copied())
            }
            (Read::Index(i), Value::List(items)) => Observed::Address(items.get(*i).copied()),
            (Read::Length, Value::List(items)) => Observed::Size(items.len()),
            _ => Observed::Revision(self.slots[object.index].revision),
        })
    }
    pub(crate) fn observe(&self, object: Handle, read: Read) -> Result<()> {
        if self.reads.borrow().is_none() {
            return Ok(());
        }
        let observed = self.observed(object, &read)?;
        if let Some(reads) = self.reads.borrow_mut().as_mut()
            && self.slots[object.index].created <= reads.start
            && !reads
                .dependencies
                .iter()
                .any(|d| d.object == object && d.read == read)
        {
            reads.dependencies.push(Dependency {
                object,
                read,
                observed,
            });
        }
        Ok(())
    }
    fn observe_write(&self, object: Handle) {
        if let Some(reads) = self.reads.borrow_mut().as_mut()
            && self.slots[object.index].created <= reads.start
        {
            reads.volatile = true;
        }
    }
    pub(crate) fn volatile(&self) {
        if let Some(reads) = self.reads.borrow_mut().as_mut() {
            reads.volatile = true;
        }
    }
    #[cfg(feature = "ui")]
    pub(crate) fn begin_reads(&self) {
        *self.reads.borrow_mut() = Some(Reads {
            start: self.allocations,
            dependencies: vec![],
            volatile: false,
        });
    }
    #[cfg(feature = "ui")]
    pub(crate) fn end_reads(&self) -> Dependencies {
        let reads = self
            .reads
            .borrow_mut()
            .take()
            .expect("binding capture started");
        Dependencies {
            values: reads.dependencies,
            volatile: reads.volatile,
        }
    }
}

#[cfg(all(test, feature = "ui"))]
mod binding_tests {
    use super::*;
    #[test]
    fn snapshots_track_only_the_read_address_and_root_it_during_capture() {
        let mut heap = Heap::default();
        let first = heap.alloc(Value::Number(1.));
        let second = heap.alloc(Value::Number(2.));
        let list = heap.alloc(Value::List(vec![first, second]));
        heap.begin_reads();
        heap.observe(list, Read::Index(0)).unwrap();
        heap.collect([]).unwrap(); // The capture itself roots source and observed addresses.
        let deps = heap.end_reads();
        let replacement = heap.alloc(Value::Number(3.));
        let Value::List(items) = heap.get_mut(list).unwrap() else {
            panic!()
        };
        items[1] = replacement;
        assert!(!deps.dirty(&heap));
        let Value::List(items) = heap.get_mut(list).unwrap() else {
            panic!()
        };
        items[0] = replacement;
        assert!(deps.dirty(&heap));
        heap.collect([]).unwrap();
        heap.alloc(Value::Number(7.));
        heap.alloc(Value::Number(8.));
        heap.alloc(Value::Number(9.));
        assert!(deps.dirty(&heap)); // Recycled indices must never alias an old generation.
        assert!(heap.get(list).is_err());
    }
    #[test]
    fn transient_function_locals_are_not_dependencies() {
        let mut heap = Heap::default();
        heap.begin_reads();
        let value = heap.alloc(Value::Number(1.));
        let cell = heap.alloc(Value::Cell(value));
        heap.observe(cell, Read::Cell).unwrap();
        assert!(heap.end_reads().roots().is_empty());
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
