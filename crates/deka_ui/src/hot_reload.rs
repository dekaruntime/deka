//! Development templates shared by native polling and browser source transport.
use crate::view::{Anchor, Context};
use deka_native_ir::tree::{NodeHandle, TemplateEdit};
use deka_ui_hot_reload::{Kind, Literal, Source, Template, TemplateNode};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

#[doc(hidden)]
pub struct Origin {
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
    pub source: &'static str,
    pub compiled_file: &'static str,
}
type PropUpdate = Box<dyn FnOnce()>;
pub(crate) type Props = BTreeMap<String, Rc<dyn Fn(&Literal) -> Result<PropUpdate, String>>>;
thread_local! {
    static COMPONENT_PROPS: RefCell<Vec<Props>> = const { RefCell::new(vec![]) };
}
/// Typed decoding is validated for every instance before any tree or signal
/// mutation. Arbitrary expressions are always compiled, never evaluated here.
#[doc(hidden)]
pub trait LiteralProp: Clone + 'static {
    fn decode(value: &Literal) -> Result<Self, String>;
}
impl LiteralProp for String {
    fn decode(value: &Literal) -> Result<Self, String> {
        if value.kind == "string" {
            Ok(value.value.clone())
        } else {
            Err("expected a string prop".into())
        }
    }
}
impl LiteralProp for bool {
    fn decode(value: &Literal) -> Result<Self, String> {
        if value.kind == "bool" {
            value.value.parse().map_err(|e| format!("{e}"))
        } else {
            Err("expected a bool prop".into())
        }
    }
}
impl LiteralProp for char {
    fn decode(value: &Literal) -> Result<Self, String> {
        if value.kind == "char" {
            value.value.parse().map_err(|e| format!("{e}"))
        } else {
            Err("expected a char prop".into())
        }
    }
}
macro_rules! numeric_props {
    ($kind:literal; $($ty:ty),*) => {$(
        impl LiteralProp for $ty {
            fn decode(value: &Literal) -> Result<Self, String> {
                let suffix = value.kind.strip_prefix($kind).ok_or("literal prop type changed")?;
                if !suffix.is_empty() && suffix != stringify!($ty) { return Err("literal prop suffix changed".into()); }
                value.value.parse().map_err(|e| format!("prop out of range for {}: {e}", stringify!($ty)))
            }
        }
    )*};
}
numeric_props!("int:"; u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);
macro_rules! float_props {
    ($($ty:ty),*) => {$(
        impl LiteralProp for $ty {
            fn decode(value: &Literal) -> Result<Self, String> {
                let suffix = value.kind.strip_prefix("float:").ok_or("literal prop type changed")?;
                if !suffix.is_empty() && suffix != stringify!($ty) { return Err("literal prop suffix changed".into()); }
                let parsed: $ty = value.value.parse().map_err(|e| format!("prop out of range for {}: {e}", stringify!($ty)))?;
                if !parsed.is_finite() { return Err(format!("prop out of range for {}", stringify!($ty))); }
                Ok(parsed)
            }
        }
    )*};
}
float_props!(f32, f64);
#[doc(hidden)]
pub fn prop<T: LiteralProp>(name: &str, value: T) -> crate::Signal<T> {
    let signal = crate::signal(value);
    COMPONENT_PROPS.with(|frames| {
        if let Some(frame) = frames.borrow_mut().last_mut() {
            frame.insert(
                name.into(),
                Rc::new(move |literal| {
                    let value = T::decode(literal)?;
                    signal.try_get().map_err(|e| e.to_string())?;
                    Ok(Box::new(move || signal.set(value)))
                }),
            );
        }
    });
    signal
}
#[doc(hidden)]
pub fn component(build: impl FnOnce() -> crate::View) -> crate::View {
    struct Frame;
    impl Drop for Frame {
        fn drop(&mut self) {
            COMPONENT_PROPS.with(|frames| {
                frames.borrow_mut().pop();
            });
        }
    }
    COMPONENT_PROPS.with(|frames| frames.borrow_mut().push(BTreeMap::new()));
    let frame = Frame;
    let view = build();
    let props =
        COMPONENT_PROPS.with(|frames| std::mem::take(frames.borrow_mut().last_mut().unwrap()));
    drop(frame);
    crate::View::__hot_props(props, view)
}
#[derive(Clone)]
pub(crate) struct Location {
    pub id: usize,
    pub origin: &'static Origin,
    pub prefix: Vec<usize>,
    pub roots: Vec<Anchor>,
    pub placement: Rc<Placement>,
    pub props: Props,
    pub movable: bool,
}
/// Mutable destination of a compiled structural reaction. Paths identify the
/// allocation; sibling order follows live destinations, including empty slots.
pub(crate) struct Placement {
    pub prefix: Vec<usize>,
    pub parent: RefCell<Anchor>,
    following: RefCell<Vec<Weak<Self>>>,
    pub configured: std::cell::Cell<bool>,
}
impl Placement {
    pub(crate) fn new(prefix: Vec<usize>, parent: Anchor) -> Rc<Self> {
        Rc::new(Self {
            prefix,
            parent: RefCell::new(parent),
            following: RefCell::new(vec![]),
            configured: std::cell::Cell::new(false),
        })
    }
    pub(crate) fn nodes(&self) -> Vec<NodeHandle> {
        self.parent
            .borrow()
            .borrow()
            .as_ref()
            .map(|parent| {
                parent
                    .0
                    .borrow()
                    .children
                    .iter()
                    .filter(|node| node.slot().starts_with(&self.prefix))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(crate) fn before(&self) -> Option<NodeHandle> {
        self.following
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .find_map(|slot| slot.nodes().into_iter().next())
    }
}
impl Location {
    fn compiled_nodes(&self, code: &str) -> Vec<NodeHandle> {
        if code.starts_with('{') {
            self.nodes()
        } else {
            self.roots
                .iter()
                .filter_map(|anchor| anchor.borrow().clone())
                .collect()
        }
    }

    fn nodes(&self) -> Vec<NodeHandle> {
        if let Some(parent) = self.placement.parent.borrow().borrow().as_ref() {
            parent
                .0
                .borrow()
                .children
                .iter()
                .filter(|node| node.slot().starts_with(&self.prefix))
                .cloned()
                .collect()
        } else {
            self.roots
                .iter()
                .filter_map(|anchor| anchor.borrow().clone())
                .collect()
        }
    }
}
pub(crate) struct Mount {
    origin: &'static Origin,
    template: RefCell<Template>,
    locations: RefCell<BTreeMap<usize, Location>>,
    prefix: Vec<usize>,
    parent: Anchor,
    roots: RefCell<Vec<Anchor>>,
    next_slot: std::cell::Cell<usize>,
    decorated: bool,
}
impl Mount {
    fn parent(&self) -> Option<NodeHandle> {
        self.roots
            .borrow()
            .iter()
            .find_map(|anchor| anchor.borrow().clone())
            .map_or_else(|| self.parent.borrow().clone(), |node| node.parent())
    }
    pub(crate) fn new(
        origin: &'static Origin,
        prefix: Vec<usize>,
        parent: Anchor,
        roots: Vec<Anchor>,
        locations: Vec<Location>,
        decorated: bool,
    ) -> Result<Rc<Self>, String> {
        Ok(Rc::new(Self {
            origin,
            template: RefCell::new(deka_ui_hot_reload::parse(
                origin.source.parse().map_err(|e| format!("{e}"))?,
            )?),
            locations: RefCell::new(locations.into_iter().map(|loc| (loc.id, loc)).collect()),
            prefix,
            parent,
            roots: RefCell::new(roots),
            next_slot: std::cell::Cell::new(0),
            decorated,
        }))
    }
    fn nodes(&self) -> Vec<NodeHandle> {
        if let Some(parent) = self.parent().as_ref() {
            parent
                .0
                .borrow()
                .children
                .iter()
                .filter(|node| node.slot().starts_with(&self.prefix))
                .cloned()
                .collect()
        } else {
            self.roots
                .borrow()
                .iter()
                .filter_map(|anchor| anchor.borrow().clone())
                .collect()
        }
    }
    fn plan(&self, next: Template) -> Result<Plan, String> {
        if self.decorated {
            return Err("builder decorations outside view! require rebuilding".into());
        }
        let old = self.template.borrow();
        let locations = self.locations.borrow();
        let mut known = std::collections::BTreeSet::new();
        fn visit(node: &NodeHandle, out: &mut std::collections::BTreeSet<String>) {
            out.insert(node.renderer_id());
            for child in node.0.borrow().children.clone() {
                visit(&child, out);
            }
        }
        for descriptor in old.flattened() {
            if let Some(location) = locations.get(&descriptor.id) {
                if matches!(&descriptor.kind, Kind::Hole(_) | Kind::Component { .. }) {
                    let roots = match &descriptor.kind {
                        Kind::Hole(code) => location.compiled_nodes(code),
                        _ => location
                            .roots
                            .iter()
                            .filter_map(|root| root.borrow().clone())
                            .collect(),
                    };
                    for node in roots {
                        visit(&node, &mut known);
                    }
                } else {
                    known.extend(
                        location
                            .roots
                            .iter()
                            .filter_map(|root| root.borrow().as_ref().map(NodeHandle::renderer_id)),
                    );
                }
            }
        }
        let mut actual = std::collections::BTreeSet::new();
        for root in self.nodes() {
            visit(&root, &mut actual);
        }
        if !actual.is_subset(&known) {
            return Err("builder children outside view! require rebuilding".into());
        }
        drop(locations);
        let map = next.map_from(&old)?;
        let locations = self.locations.borrow();
        fn topology(
            nodes: &[TemplateNode],
            parent: Option<usize>,
            out: &mut BTreeMap<usize, (Option<usize>, Vec<usize>)>,
        ) {
            let siblings = nodes.iter().map(|node| node.id).collect::<Vec<_>>();
            for node in nodes {
                out.insert(node.id, (parent, siblings.clone()));
                topology(&node.children, Some(node.id), out);
            }
        }
        let mut before = BTreeMap::new();
        topology(&old.nodes, None, &mut before);
        let mut after = BTreeMap::new();
        topology(&next.nodes, None, &mut after);
        for (id, (parent, siblings)) in &after {
            if let Some(previous) = map.get(id)
                && let Some(location) = locations.get(previous)
                && !location.movable
            {
                let (old_parent, old_siblings) = &before[previous];
                if parent.and_then(|id| map.get(&id).copied()) != *old_parent
                    || siblings
                        .iter()
                        .map(|id| map.get(id).copied())
                        .collect::<Vec<_>>()
                        != old_siblings.iter().copied().map(Some).collect::<Vec<_>>()
                {
                    return Err(
                        "moving an opaque fragment with structural children requires rebuilding"
                            .into(),
                    );
                }
            }
        }
        let mut output = BTreeMap::new();
        let mut serial = self.next_slot.get();
        let mut props = vec![];
        fn edit(
            node: &TemplateNode,
            mount: &Mount,
            map: &BTreeMap<usize, usize>,
            locations: &BTreeMap<usize, Location>,
            output: &mut BTreeMap<usize, Location>,
            serial: &mut usize,
            props: &mut Vec<PropUpdate>,
        ) -> Result<TemplateEdit, String> {
            let previous = map.get(&node.id).and_then(|id| locations.get(id));
            let mut location = if let Some(previous) = previous {
                previous.clone()
            } else {
                let mut prefix = mount.prefix.clone();
                prefix.extend([usize::MAX, *serial]);
                *serial += 1;
                Location {
                    id: node.id,
                    origin: mount.origin,
                    prefix: prefix.clone(),
                    roots: vec![Anchor::default()],
                    placement: Placement::new(prefix.clone(), Anchor::default()),
                    props: BTreeMap::new(),
                    movable: true,
                }
            };
            location.id = node.id;
            if matches!(&node.kind, Kind::Hole(_) | Kind::Component { .. }) {
                let compiled_location = previous.ok_or("missing compiled slot")?;
                let roots = match &node.kind {
                    Kind::Hole(code) => compiled_location.compiled_nodes(code),
                    Kind::Component { props: values, .. } => {
                        let previous = &mount.template.borrow();
                        let old_id = map[&node.id];
                        let old = previous
                            .flattened()
                            .into_iter()
                            .find(|node| node.id == old_id)
                            .unwrap();
                        let Kind::Component {
                            props: old_values, ..
                        } = &old.kind
                        else {
                            unreachable!()
                        };
                        for (name, value) in values {
                            if old_values.get(name) != Some(value) {
                                let update = compiled_location.props.get(name)
                                    .ok_or_else(|| format!("component prop `{name}` is used by compiled logic; requires rebuilding"))?;
                                props.push(update(value)?);
                            }
                        }
                        compiled_location
                            .roots
                            .iter()
                            .filter_map(|root| root.borrow().clone())
                            .collect()
                    }
                    _ => unreachable!(),
                };
                output.insert(node.id, location);
                return Ok(TemplateEdit::Keep(roots));
            }
            let current = previous
                .and_then(|loc| loc.roots.first())
                .and_then(|anchor| anchor.borrow().clone());
            let mut wire = current
                .as_ref()
                .map(NodeHandle::authored_wire)
                .unwrap_or_default();
            match &node.kind {
                Kind::Text(text) => {
                    wire.text = Some(text.clone());
                    wire.tag = String::new();
                }
                Kind::Element {
                    tag, attributes, ..
                } => {
                    // Replace only authored static values. Live values and handlers
                    // belong to the compiled node's existing registrations.
                    if let Some(previous) = map.get(&node.id).and_then(|id| {
                        mount
                            .template
                            .borrow()
                            .flattened()
                            .into_iter()
                            .find(|n| n.id == *id)
                            .cloned()
                    }) && let Kind::Element { attributes, .. } = previous.kind
                    {
                        for name in attributes.keys() {
                            wire.attributes.remove(name);
                            if name == "class" {
                                wire.classes.clear();
                            }
                        }
                    }
                    wire.tag = tag.clone();
                    wire.text = None;
                    for (name, value) in attributes {
                        wire.attributes.insert(name.clone(), value.clone());
                        if name == "class" {
                            wire.classes = value.clone();
                        }
                    }
                }
                Kind::Hole(_) | Kind::Component { .. } => unreachable!(),
            }
            wire.style()?;
            let children = node
                .children
                .iter()
                .map(|node| edit(node, mount, map, locations, output, serial, props))
                .collect::<Result<_, _>>()?;
            output.insert(node.id, location.clone());
            Ok(TemplateEdit::Node {
                current,
                wire,
                slot: location.prefix,
                children,
            })
        }
        let edits = next
            .nodes
            .iter()
            .map(|node| {
                edit(
                    node,
                    self,
                    &map,
                    &locations,
                    &mut output,
                    &mut serial,
                    &mut props,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if self.parent().is_none() && next.nodes.len() != 1 {
            return Err("changing the number of application roots requires rebuilding".into());
        }
        Ok(Plan {
            next,
            edits,
            locations: output,
            serial,
            props,
        })
    }
    fn commit(&self, plan: Plan, context: &Rc<Context>) -> Result<(), String> {
        let parent = self.parent();
        let roots =
            context
                .tree
                .borrow_mut()
                .patch_template(parent.as_ref(), &self.nodes(), plan.edits)?;
        // Static anchors continue to target retained allocations even after moves.
        fn collect(node: &NodeHandle, out: &mut BTreeMap<Vec<usize>, NodeHandle>) {
            out.insert(node.slot(), node.clone());
            for child in node.0.borrow().children.clone() {
                collect(&child, out);
            }
        }
        let mut nodes = BTreeMap::new();
        for root in &roots {
            collect(root, &mut nodes);
        }
        for location in plan.locations.values() {
            if let Some(node) = nodes.get(&location.prefix)
                && let Some(anchor) = location.roots.first()
            {
                anchor.replace(Some(node.clone()));
            }
        }
        fn place(nodes: &[TemplateNode], parent: Anchor, locations: &BTreeMap<usize, Location>) {
            for (index, node) in nodes.iter().enumerate() {
                let location = &locations[&node.id];
                location.placement.parent.replace(parent.clone());
                location.placement.following.replace(
                    nodes[index + 1..]
                        .iter()
                        .map(|node| Rc::downgrade(&locations[&node.id].placement))
                        .collect(),
                );
                location.placement.configured.set(true);
                if !node.children.is_empty() {
                    place(&node.children, location.roots[0].clone(), locations);
                }
            }
        }
        place(
            &plan.next.nodes,
            Rc::new(RefCell::new(parent)),
            &plan.locations,
        );
        self.roots.replace(
            roots
                .into_iter()
                .map(|node| Rc::new(RefCell::new(Some(node))))
                .collect(),
        );
        self.locations.replace(plan.locations);
        self.template.replace(plan.next);
        self.next_slot.set(plan.serial);
        context.changed(true);
        for update in plan.props {
            update();
        }
        Ok(())
    }
}
struct Plan {
    next: Template,
    edits: Vec<TemplateEdit>,
    locations: BTreeMap<usize, Location>,
    serial: usize,
    props: Vec<PropUpdate>,
}
struct File {
    baseline: Source,
    #[cfg(not(target_arch = "wasm32"))]
    seen: String,
    applied: String,
    #[cfg(not(target_arch = "wasm32"))]
    observed_at: std::time::Instant,
    reported_error: Option<String>,
}
#[derive(Default)]
pub(crate) struct Registry {
    mounts: Vec<Weak<Mount>>,
    files: BTreeMap<PathBuf, File>,
    stale: bool,
    #[cfg(target_arch = "wasm32")]
    remote: deka_ui_hot_reload::files::Files,
    #[cfg(not(target_arch = "wasm32"))]
    watch_roots: std::collections::BTreeSet<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    watch_baseline: deka_ui_hot_reload::files::Files,
    #[cfg(not(target_arch = "wasm32"))]
    watch_seen: deka_ui_hot_reload::files::Files,
    #[cfg(not(target_arch = "wasm32"))]
    watch_observed: Option<std::time::Instant>,
    #[cfg(not(target_arch = "wasm32"))]
    watch_error: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadStatus {
    Unchanged,
    Patched { templates: usize },
    RestartRequired { reason: String },
    Error { message: String },
}
impl Registry {
    pub(crate) fn register(&mut self, mount: &Rc<Mount>) -> Result<(), String> {
        let path = PathBuf::from(mount.origin.file);
        // Capture conventional and custom Rust source paths, including logic
        // outside files containing view!. Workspace roots dominate package roots.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(root) = path
            .ancestors()
            .filter(|path| path.join("Cargo.toml").is_file())
            .last()
            && !self
                .watch_roots
                .iter()
                .any(|watched| root.starts_with(watched))
        {
            let roots = deka_ui_hot_reload::files::source_roots(&[
                "--manifest-path".into(),
                root.join("Cargo.toml").to_string_lossy().into_owned(),
            ])?;
            for (path, source) in deka_ui_hot_reload::files::snapshot(&roots)? {
                self.watch_baseline.entry(path).or_insert(source);
            }
            self.watch_roots.extend(roots);
            self.watch_seen = self.watch_baseline.clone();
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = self.files.entry(path) {
            entry.insert(File {
                baseline: Source::parse(mount.origin.compiled_file)?,
                #[cfg(not(target_arch = "wasm32"))]
                seen: mount.origin.compiled_file.into(),
                applied: mount.origin.compiled_file.into(),
                #[cfg(not(target_arch = "wasm32"))]
                observed_at: std::time::Instant::now(),
                reported_error: None,
            });
        }
        // A component mounted after an earlier save starts from compiled markup.
        // Revisit its file even when existing instances already applied that save.
        self.files
            .get_mut(Path::new(mount.origin.file))
            .unwrap()
            .applied = mount.origin.compiled_file.into();
        self.mounts.push(Rc::downgrade(mount));
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn receive(
        &mut self,
        context: &Rc<Context>,
        baseline: deka_ui_hot_reload::files::Files,
        current: deka_ui_hot_reload::files::Files,
    ) -> ReloadStatus {
        for mount in self.mounts.iter().filter_map(Weak::upgrade) {
            if let Some(original) = baseline.get(Path::new(mount.origin.file))
                && original != mount.origin.compiled_file
            {
                self.stale = true;
                return ReloadStatus::RestartRequired { reason: "browser source baseline differs from its compiled module; rebuilding and restarting (signal state resets)".into() };
            }
        }
        match deka_ui_hot_reload::files::classify(&baseline, &current) {
            deka_ui_hot_reload::files::Change::Markup => (),
            deka_ui_hot_reload::files::Change::Invalid(message) => {
                return ReloadStatus::Error { message };
            }
            deka_ui_hot_reload::files::Change::Restart(reason) => {
                self.stale = true;
                return ReloadStatus::RestartRequired { reason };
            }
        }
        self.remote = current;
        self.replay(context)
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn replay(&mut self, context: &Rc<Context>) -> ReloadStatus {
        self.apply_files(context, &self.remote.clone())
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn poll(&mut self, context: &Rc<Context>) -> ReloadStatus {
        if self.stale {
            return ReloadStatus::Unchanged;
        }
        use deka_ui_hot_reload::files::{Change, classify, snapshot};
        let roots: Vec<_> = self.watch_roots.iter().cloned().collect();
        let watched = match snapshot(&roots) {
            Ok(files) => files,
            Err(error) => {
                return ReloadStatus::Error {
                    message: format!("{error}; keeping last good UI"),
                };
            }
        };
        if watched != self.watch_seen {
            self.watch_seen = watched;
            self.watch_observed = Some(std::time::Instant::now());
            return ReloadStatus::Unchanged;
        }
        if self
            .watch_observed
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(20))
        {
            return ReloadStatus::Unchanged;
        }
        match classify(&self.watch_baseline, &watched) {
            Change::Markup => self.watch_error = None,
            Change::Invalid(error) => {
                if self.watch_error.as_deref() == Some(&error) {
                    return ReloadStatus::Unchanged;
                }
                self.watch_error = Some(error.clone());
                return ReloadStatus::Error {
                    message: format!("{error}; keeping last good UI"),
                };
            }
            Change::Restart(reason) => {
                self.stale = true;
                return ReloadStatus::RestartRequired {
                    reason: format!("{reason}; rebuilding and restarting (signal state resets)"),
                };
            }
        }
        self.apply_files(context, &watched)
    }
    fn apply_files(
        &mut self,
        context: &Rc<Context>,
        watched: &deka_ui_hot_reload::files::Files,
    ) -> ReloadStatus {
        if self.stale {
            return ReloadStatus::Unchanged;
        }
        self.mounts.retain(|mount| mount.strong_count() > 0);
        let mut plans = vec![];
        let mut changed = vec![];
        #[cfg(not(target_arch = "wasm32"))]
        let mut unstable = false;
        for (path, file) in &mut self.files {
            let source = match watched
                .get(path)
                .cloned()
                .map(Ok::<_, std::io::Error>)
                .unwrap_or_else(|| {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        std::fs::read_to_string(path)
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        Ok(file.applied.clone())
                    }
                }) {
                Ok(source) => source,
                Err(error) => {
                    return ReloadStatus::Error {
                        message: format!("{}: {error}; keeping last good UI", path.display()),
                    };
                }
            };
            if source == file.applied {
                continue;
            }
            #[cfg(not(target_arch = "wasm32"))]
            let seen = source == file.seen;
            #[cfg(not(target_arch = "wasm32"))]
            if !seen {
                file.seen = source.clone();
                file.observed_at = std::time::Instant::now();
            }
            #[cfg(not(target_arch = "wasm32"))]
            if !watched.contains_key(path)
                && file.observed_at.elapsed() < std::time::Duration::from_millis(20)
            {
                unstable = true;
                continue;
            }
            let next = match Source::parse(&source) {
                Ok(next) => next,
                Err(error) => {
                    if file.reported_error.as_deref() == Some(source.as_str()) {
                        return ReloadStatus::Unchanged;
                    }
                    file.reported_error = Some(source);
                    return ReloadStatus::Error {
                        message: format!("{}: {error}; keeping last good UI", path.display()),
                    };
                }
            };
            file.reported_error = None;
            if let Err(reason) = file.baseline.compatible(&next) {
                self.stale = true;
                return ReloadStatus::RestartRequired {
                    reason: format!(
                        "{}: {reason}; rebuilding and restarting (signal state resets)",
                        path.display()
                    ),
                };
            }
            changed.push((path.clone(), next, source));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if unstable {
            return ReloadStatus::Unchanged;
        }
        let mut applied = vec![];
        for (path, next, source) in changed {
            applied.push((path.clone(), source));
            for mount in self
                .mounts
                .iter()
                .rev()
                .filter_map(Weak::upgrade)
                .filter(|mount| Path::new(mount.origin.file) == path)
            {
                let file = &self.files[&path];
                let Some(index) = file.baseline.locations.iter().position(|&(line, column)| {
                    line == mount.origin.line as usize && column == mount.origin.column as usize
                }) else {
                    return ReloadStatus::Error {
                        message: format!("{}: compiled template location missing", path.display()),
                    };
                };
                if *mount.template.borrow() == next.templates[index] {
                    continue;
                }
                match mount.plan(next.templates[index].clone()) {
                    Ok(plan) => plans.push((mount, plan)),
                    Err(reason) => {
                        self.stale = true;
                        return ReloadStatus::RestartRequired {
                            reason: format!(
                                "{reason}; rebuilding and restarting (signal state resets)"
                            ),
                        };
                    }
                }
            }
        }
        plans.sort_by_key(|(mount, _)| mount.prefix.len());
        let templates = plans.len();
        for (mount, plan) in plans {
            if let Err(message) = mount.commit(plan, context) {
                return ReloadStatus::Error { message };
            }
        }
        for (path, source) in applied {
            self.files.get_mut(&path).unwrap().applied = source;
        }
        if templates == 0 {
            ReloadStatus::Unchanged
        } else {
            eprintln!("deka dev: patched {templates} template(s); state preserved");
            ReloadStatus::Patched { templates }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UiApp, View};
    struct Fixture {
        path: PathBuf,
        directory: PathBuf,
        origin: &'static Origin,
        source: String,
    }
    impl Fixture {
        fn new(label: &str) -> Self {
            let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tmp")
                .join(format!(
                    "hot-reload-{label}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("main.rs");
            let source = "fn App() { view! { <view><span>\"one\"</span></view> } }".to_owned();
            std::fs::write(&path, &source).unwrap();
            let locations = Source::parse(&source).unwrap().locations;
            let origin = Box::leak(Box::new(Origin {
                file: Box::leak(path.to_string_lossy().into_owned().into_boxed_str()),
                line: locations[0].0 as u32,
                column: locations[0].1 as u32,
                source: "<view><span>\"one\"</span></view>",
                compiled_file: Box::leak(source.clone().into_boxed_str()),
            }));
            Self {
                path,
                directory,
                origin,
                source,
            }
        }
        fn view(&self) -> View {
            Self::build(self.origin)
        }
        fn build(origin: &'static Origin) -> View {
            View::__hot_template(
                origin,
                View::__hot_node(
                    origin,
                    0,
                    View::element("view").child(View::__hot_node(
                        origin,
                        1,
                        View::element("span").child(View::__hot_node(origin, 2, View::text("one"))),
                    )),
                ),
            )
        }
        fn write(&self, source: &str) {
            std::fs::write(&self.path, source).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.directory).unwrap();
        }
    }
    fn text(node: &deka_native_ir::Node) -> String {
        node.text.clone().unwrap_or_default() + &node.children.iter().map(text).collect::<String>()
    }
    fn settled(app: &mut UiApp) -> ReloadStatus {
        let first = app.poll_hot_reload();
        if first != ReloadStatus::Unchanged {
            return first;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        app.poll_hot_reload()
    }
    #[test]
    fn one_malformed_file_blocks_the_batch_and_pending_good_edits_recover() {
        let first = Fixture::new("first");
        let second = Fixture::new("second");
        let mut app = UiApp::new(|| View::fragment([first.view(), second.view()]));
        let original = app.tree().snapshot();
        first.write(&first.source.replace("one", "two"));
        second.write(&second.source.replace("<span>", "<span class=\"p-bad\">"));
        assert!(matches!(settled(&mut app), ReloadStatus::Error { .. }));
        assert_eq!(app.tree().snapshot(), original);
        assert_eq!(app.poll_hot_reload(), ReloadStatus::Unchanged);
        assert_eq!(app.tree().snapshot(), original);
        second.write(&second.source.replace("one", "three"));
        assert_eq!(settled(&mut app), ReloadStatus::Patched { templates: 2 });
        assert_eq!(text(&app.tree().snapshot()), "twothree");
    }
    #[test]
    fn builder_attribute_overrides_rebuild_without_partial_patches() {
        let fixture = Fixture::new("builder-override");
        let mut app = UiApp::new(|| fixture.view().attr("class", "p-8"));
        let original = app.tree().snapshot();
        fixture.write(
            &fixture
                .source
                .replace("one", "two")
                .replace("<view>", "<view class=\"p-4\">"),
        );
        assert!(matches!(
            settled(&mut app),
            ReloadStatus::RestartRequired { .. }
        ));
        assert_eq!(app.tree().snapshot(), original);
    }
    #[test]
    fn a_component_mounted_after_a_save_gets_the_current_template() {
        let fixture = Fixture::new("late-mount");
        let trigger = Rc::new(RefCell::new(None));
        let control = trigger.clone();
        let origin = fixture.origin;
        let mut app = UiApp::new(|| {
            let show = crate::signal(false);
            control.replace(Some(show));
            View::element("view")
                .child(fixture.view())
                .child(move || show.get().unwrap().then(|| Fixture::build(origin)))
        });
        fixture.write(&fixture.source.replace("one", "current"));
        assert_eq!(settled(&mut app), ReloadStatus::Patched { templates: 1 });
        assert_eq!(text(&app.tree().snapshot()), "current");
        trigger.borrow().as_ref().unwrap().set(true);
        assert_eq!(settled(&mut app), ReloadStatus::Patched { templates: 1 });
        assert_eq!(text(&app.tree().snapshot()), "currentcurrent");
    }
    #[test]
    fn any_compiled_code_change_blocks_all_markup_and_remains_explicitly_stale() {
        let first = Fixture::new("logic-first");
        let second = Fixture::new("logic-second");
        let mut app = UiApp::new(|| View::fragment([first.view(), second.view()]));
        let original = app.tree().snapshot();
        first.write(&first.source.replace("one", "two"));
        second.write(&second.source.replace("fn App()", "fn Renamed()"));
        assert!(matches!(
            settled(&mut app),
            ReloadStatus::RestartRequired { .. }
        ));
        assert_eq!(app.tree().snapshot(), original);
        second.write(&second.source);
        assert_eq!(app.poll_hot_reload(), ReloadStatus::Unchanged);
        assert_eq!(app.tree().snapshot(), original);
    }
}
