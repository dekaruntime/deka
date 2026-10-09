//! Native development templates; compiled out of release and browser applications.
use crate::view::{Anchor, Context};
use deka_native_ir::tree::{NodeHandle, TemplateEdit};
use deka_ui_hot_reload::{Kind, Source, Template, TemplateNode};
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
#[derive(Clone)]
pub(crate) struct Location {
    pub id: usize,
    pub origin: &'static Origin,
    pub prefix: Vec<usize>,
    pub parent: Anchor,
    pub roots: Vec<Anchor>,
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
        if let Some(parent) = self.parent.borrow().as_ref() {
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
        if let Some(parent) = self.parent.borrow().as_ref() {
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
                if let Kind::Hole(code) = &descriptor.kind {
                    for node in location.compiled_nodes(code) {
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
        let mut output = BTreeMap::new();
        let mut serial = self.next_slot.get();
        fn edit(
            node: &TemplateNode,
            mount: &Mount,
            map: &BTreeMap<usize, usize>,
            locations: &BTreeMap<usize, Location>,
            output: &mut BTreeMap<usize, Location>,
            serial: &mut usize,
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
                    prefix,
                    parent: Anchor::default(),
                    roots: vec![Anchor::default()],
                }
            };
            location.id = node.id;
            if let Kind::Hole(_) = &node.kind {
                let compiled_location = previous.ok_or("missing compiled slot")?;
                let Kind::Hole(code) = &node.kind else {
                    unreachable!()
                };
                let roots = compiled_location.compiled_nodes(code);
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
                Kind::Hole(_) => unreachable!(),
            }
            wire.style()?;
            let children = node
                .children
                .iter()
                .map(|node| edit(node, mount, map, locations, output, serial))
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
            .map(|node| edit(node, self, &map, &locations, &mut output, &mut serial))
            .collect::<Result<Vec<_>, _>>()?;
        if self.parent.borrow().is_none() && next.nodes.len() != 1 {
            return Err("changing the number of application roots requires rebuilding".into());
        }
        Ok(Plan {
            next,
            edits,
            locations: output,
            serial,
        })
    }
    fn commit(&self, plan: Plan, context: &Rc<Context>) -> Result<(), String> {
        let parent = self.parent.borrow().clone();
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
        Ok(())
    }
}
struct Plan {
    next: Template,
    edits: Vec<TemplateEdit>,
    locations: BTreeMap<usize, Location>,
    serial: usize,
}
struct File {
    baseline: Source,
    seen: String,
    applied: String,
    observed_at: std::time::Instant,
    reported_error: Option<String>,
}
#[derive(Default)]
pub(crate) struct Registry {
    mounts: Vec<Weak<Mount>>,
    files: BTreeMap<PathBuf, File>,
    stale: bool,
    watch_roots: std::collections::BTreeSet<PathBuf>,
    watch_baseline: deka_ui_hot_reload::files::Files,
    watch_seen: deka_ui_hot_reload::files::Files,
    watch_observed: Option<std::time::Instant>,
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
                seen: mount.origin.compiled_file.into(),
                applied: mount.origin.compiled_file.into(),
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
        self.mounts.retain(|mount| mount.strong_count() > 0);
        let mut plans = vec![];
        let mut changed = vec![];
        let mut unstable = false;
        for (path, file) in &mut self.files {
            let source = match watched
                .get(path)
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| std::fs::read_to_string(path))
            {
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
            let seen = source == file.seen;
            if !seen {
                file.seen = source.clone();
                file.observed_at = std::time::Instant::now();
            }
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
