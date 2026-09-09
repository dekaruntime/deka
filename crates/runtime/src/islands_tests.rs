use super::*;

    #[test]
    fn content_hash_is_stable_short_hex() {
        let hash = content_hash_hex(b"deka");
        assert_eq!(hash.len(), ASSET_HASH_LEN);
        assert!(
            hash.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_eq!(hash, content_hash_hex(b"deka"));
        assert_ne!(hash, content_hash_hex(b"deka!"));
    }

    #[test]
    fn hashed_name_round_trips() {
        let name = hashed_asset_name("islands-load", "js", b"chunk");
        assert!(is_hashed_asset_name(&name, "js"), "{name}");
        assert!(!is_hashed_asset_name("islands-load.js", "js"));
        assert!(!is_hashed_asset_name("islands-load.JS", "js"));
        assert!(!is_hashed_asset_name(&name, "css"));
    }

    #[test]
    fn rewrite_ui_imports_targets_hashed_chunks() {
        let ui = UiChunkNames {
            hashed: BTreeMap::from([
                ("ui/jsx".to_string(), "jsx.a1b2c3d4e5.js".to_string()),
                (
                    "ui/server".to_string(),
                    "server-stub.a1b2c3d4e5.js".to_string(),
                ),
            ]),
        };
        let js = rewrite_ui_imports(
            "import { x } from \"ui/jsx\";\nimport { y } from \"ui/server\";\n",
            &ui,
        );
        assert!(js.contains("from \"./ui/jsx.a1b2c3d4e5.js\""), "{js}");
        assert!(
            js.contains("from \"./ui/server-stub.a1b2c3d4e5.js\""),
            "{js}"
        );
    }

    /// Invariant behind deka#750's no-split decision: the ui/island-marker
    /// source must keep exporting the SSR producers (as plain function
    /// declarations, droppable by the pruner) alongside `parseIslandMarker`.
    /// If the module is ever split or refactored to const-arrow exports, the
    /// dist pruning premise silently changes — fail here instead.
    #[test]
    fn island_marker_source_keeps_ssr_producers_unsplit() {
        for symbol in ["formatIslandStart", "formatIslandEnd", "encodeB64", "utf8Bytes"] {
            assert!(
                deka_ui::ISLAND_MARKER.contains(symbol),
                "ui/island-marker.js must still define {symbol}"
            );
            assert!(
                deka_ui::ISLAND_MARKER.contains(&format!("function {symbol}")),
                "{symbol} must stay a plain function declaration (prunable)"
            );
        }
        assert!(deka_ui::ISLAND_MARKER.contains("parseIslandMarker"));
    }

    /// Dist flavor: ui/island-marker prunes to the browser-visible surface
    /// (`parseIslandMarker` and its transitive deps); the SSR producers are
    /// gone, and the dev flavor keeps the readable source untouched.
    #[test]
    fn dist_prunes_island_marker_dev_keeps_it() {
        let keep = compute_ui_keep_sets(&[]).expect("keep sets");
        let dist = tempfile::tempdir().expect("tempdir");
        let dist_names = write_ui_chunks(dist.path(), ClientAssetFlavor::Dist, &keep)
            .expect("write dist ui chunks");
        let dist_marker = fs::read_to_string(dist.path().join(dist_names.file("ui/island-marker")))
            .expect("read dist island-marker");
        assert!(dist_marker.contains("parseIslandMarker"), "{dist_marker}");
        for symbol in ["formatIslandStart", "formatIslandEnd", "encodeB64", "utf8Bytes"] {
            assert!(
                !dist_marker.contains(symbol),
                "dist island-marker must not contain {symbol}: {dist_marker}"
            );
        }

        let dev = tempfile::tempdir().expect("tempdir");
        let dev_names =
            write_ui_chunks(dev.path(), ClientAssetFlavor::Dev, &None).expect("write dev ui chunks");
        let dev_marker = fs::read_to_string(dev.path().join(dev_names.file("ui/island-marker")))
            .expect("read dev island-marker");
        assert!(dev_marker.contains("formatIslandStart"), "{dev_marker}");
        assert!(dev_marker.contains("export function parseIslandMarker"), "{dev_marker}");
    }

    // The client chunk 404s the moment it loads if any relative sibling
    // import misses the hashed graph. Derive the import list from the source
    // so a newly added sibling fails here until it is wired into the graph.
    #[test]
    fn ui_chunks_rewrite_every_client_sibling_import() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let names = write_ui_chunks(tmp.path(), ClientAssetFlavor::Dev, &None).expect("write ui chunks");
        let client_src =
            fs::read_to_string(tmp.path().join(names.file("ui/client"))).expect("read client");
        let written: BTreeSet<String> = fs::read_dir(tmp.path())
            .expect("read ui dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        let mut checked = 0usize;
        for (index, _) in deka_ui::CLIENT.match_indices("from \"./") {
            let rest = &deka_ui::CLIENT[index + "from \"./".len()..];
            let relative = rest.split('"').next().expect("quoted relative import");
            checked += 1;
            assert!(
                !client_src.contains(&format!("./{relative}")),
                "client chunk still imports unhashed ./{relative}"
            );
            let hashed = written
                .iter()
                .find(|name| name.starts_with(relative.trim_end_matches(".js")))
                .unwrap_or_else(|| panic!("no hashed chunk written for ./{relative}"));
            assert!(
                client_src.contains(&format!("./{hashed}")),
                "client chunk does not reference sibling {hashed}"
            );
        }
        assert!(
            checked >= 3,
            "expected client.js sibling imports, found {checked}"
        );
    }

    /// deka#622 finding E: the island rewrite used to list `ui/form`,
    /// `ui/suspense`, and `ui/router` as unhashed `./ui/form.js` (etc.)
    /// while `write_ui_chunks` never wrote those files. Form / Suspense
    /// inside an island 404ed. The rewrite set is now the written set.
    #[test]
    fn rewrite_ui_imports_resolves_to_written_chunks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let names = write_ui_chunks(tmp.path(), ClientAssetFlavor::Dev, &None).expect("write ui chunks");
        assert_eq!(
            names.hashed.len(),
            deka_ui::SPECIFIERS.len(),
            "every deka_ui specifier must be written (ui/server as the stub)"
        );
        let js = deka_ui::SPECIFIERS
            .iter()
            .map(|spec| format!("import {{ x }} from \"{spec}\";"))
            .collect::<Vec<_>>()
            .join("\n");
        let rewritten = rewrite_ui_imports(&js, &names);
        for spec in deka_ui::SPECIFIERS {
            assert!(
                !rewritten.contains(&format!("from \"{spec}\"")),
                "bare {spec} must be rewritten to a hashed relative path: {rewritten}"
            );
        }
        let mut rest = rewritten.as_str();
        let mut checked = 0usize;
        while let Some(at) = rest.find("from \"./ui/") {
            let after = &rest[at + "from \"./ui/".len()..];
            let file = after.split('"').next().expect("quoted path");
            assert!(
                is_hashed_asset_name(file, "js"),
                "rewrite must target a hashed chunk, got {file}"
            );
            assert!(
                tmp.path().join(file).is_file(),
                "rewrite targets {file} which was never written; dir has {:?}",
                fs::read_dir(tmp.path())
                    .expect("read ui dir")
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            );
            checked += 1;
            rest = &after[file.len()..];
        }
        assert_eq!(
            checked,
            deka_ui::SPECIFIERS.len(),
            "every specifier must produce a relative import: {rewritten}"
        );

        // Relative sibling imports inside the written chunks must also
        // resolve (form.js → jsx.js, client.js → island-marker.js, …).
        for name in names.hashed.values() {
            let src = fs::read_to_string(tmp.path().join(name)).expect("read chunk");
            let mut src_rest = src.as_str();
            while let Some(at) = src_rest.find("from \"./") {
                let after = &src_rest[at + "from \"./".len()..];
                let relative = after.split('"').next().expect("quoted relative import");
                assert!(
                    is_hashed_asset_name(relative, "js"),
                    "{name} still imports unhashed ./{relative}"
                );
                assert!(
                    tmp.path().join(relative).is_file(),
                    "{name} imports ./{relative} which was never written"
                );
                src_rest = &after[relative.len()..];
            }
        }
    }

    #[test]
    fn island_assets_are_content_addressed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let island_src = tmp.path().join("counter.dsx");
        fs::write(
            &island_src,
            "export fn Counter() {\n    return <button>0</button>;\n}\n",
        )
        .expect("write island");
        let island = ClientIsland {
            component: "Counter".to_string(),
            directive: "load".to_string(),
            file: island_src.to_string_lossy().into_owned(),
            props: vec![],
        };
        let first = tmp.path().join("first");
        write_island_client_assets(&first, &[island.clone()], ClientAssetFlavor::Dev).expect("first write");
        let second = tmp.path().join("second");
        write_island_client_assets(&second, &[island.clone()], ClientAssetFlavor::Dev).expect("second write");
        let names = |dir: &Path| -> BTreeSet<String> {
            fs::read_dir(dir)
                .expect("read dir")
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(names(&first), names(&second), "same input -> same names");

        // Changing the island source changes its hash; ui/* chunks stay put.
        fs::write(
            &island_src,
            "export fn Counter() {\n    return <button>1</button>;\n}\n",
        )
        .expect("rewrite island");
        let third = tmp.path().join("third");
        write_island_client_assets(&third, &[island], ClientAssetFlavor::Dev).expect("third write");
        let before = names(&first);
        let after = names(&third);
        assert_ne!(before, after, "changed island -> changed names");
        let ui_before: BTreeSet<String> = before
            .iter()
            .filter(|n| n.starts_with("jsx.") || n.starts_with("client."))
            .cloned()
            .collect();
        let ui_after: BTreeSet<String> = after
            .iter()
            .filter(|n| n.starts_with("jsx.") || n.starts_with("client."))
            .cloned()
            .collect();
        assert_eq!(ui_before, ui_after, "shared chunks must not rotate");

        // The importmap tracks the hashed URLs for both runs.
        let map: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(first.join(CLIENT_IMPORTMAP_FILE)).expect("read importmap"),
        )
        .expect("parse importmap");
        let imports = map["imports"].as_object().expect("imports object");
        let entry_url = imports["islands/load"].as_str().expect("islands/load url");
        let entry_name = entry_url.trim_start_matches("/assets/");
        assert!(
            first.join(entry_name).is_file(),
            "importmap url must exist: {entry_url}"
        );
        assert!(
            entry_name.contains('.'),
            "importmap url must be hashed: {entry_url}"
        );
    }

    #[test]
    fn importmap_tag_is_inlined_without_src() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // No map on disk -> no tag.
        assert!(
            inline_importmap_tag(tmp.path())
                .expect("read missing map")
                .is_none()
        );

        write_importmap_entries(
            tmp.path(),
            &BTreeMap::from([(
                "ui/client".to_string(),
                "/assets/ui/client.a1b2c3d4e5.js".to_string(),
            )]),
        )
        .expect("write importmap");
        let tag = inline_importmap_tag(tmp.path())
            .expect("build tag")
            .expect("tag for existing map");
        assert!(tag.starts_with(r#"<script type="importmap">"#), "{tag}");
        assert!(tag.ends_with("</script>"), "{tag}");
        assert!(
            !tag.contains("src="),
            "browsers reject the src form of <script type=\"importmap\">: {tag}"
        );
        let body = tag
            .strip_prefix(r#"<script type="importmap">"#)
            .and_then(|rest| rest.strip_suffix("</script>"))
            .expect("tag body");
        let map: serde_json::Value =
            serde_json::from_str(body).expect("inline body parses as JSON");
        let imports = map["imports"].as_object().expect("imports object");
        assert_eq!(
            imports["ui/client"].as_str().expect("ui/client url"),
            "/assets/ui/client.a1b2c3d4e5.js"
        );
    }

    #[test]
    fn rewrite_swaps_placeholder_for_inline_map() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache = tmp.path().join(".cache").join("dekascript");
        let assets = cache.join("assets");
        fs::create_dir_all(&assets).expect("mkdir assets");
        let island_src = assets.join("..").join("counter.dsx");
        fs::write(
            &island_src,
            "export fn Counter() {\n    return <button>0</button>;\n}\n",
        )
        .expect("write island");
        let island = ClientIsland {
            component: "Counter".to_string(),
            directive: "load".to_string(),
            file: island_src.to_string_lossy().into_owned(),
            props: vec![],
        };
        write_island_client_assets(&assets, &[island], ClientAssetFlavor::Dev).expect("write assets");

        // The entry as generated: the document is embedded as JSON string
        // literals (framework::json_str), so the placeholder appears
        // quote-escaped, and the island script still has its logical URL.
        let entry = cache.join("serve-entry.dsx");
        let placeholder = runtime_core::framework::CLIENT_IMPORTMAP_PLACEHOLDER_TAG;
        let doc = format!(
            "<head>{placeholder}</head><script type=\"module\" src=\"/assets/islands-load.js\"></script>"
        );
        let doc_literal = serde_json::to_string(&doc).expect("encode doc literal");
        fs::write(&entry, format!("const doc_head = {doc_literal};\n")).expect("write entry");

        rewrite_serve_entry_asset_urls(tmp.path()).expect("rewrite");
        let served = fs::read_to_string(&entry).expect("read rewritten entry");
        assert!(
            !served.contains(placeholder)
                && !served.contains(r#"<script type=\"importmap\" src=\"#),
            "placeholder src-reference must be swapped out: {served}"
        );
        // Unescape the JSON literal and assert on the document itself.
        let served_doc = served.replace("\\\"", "\"");
        assert!(
            !served_doc.contains(r#"<script type="importmap" src="#),
            "no external import map may survive: {served_doc}"
        );
        let tag_start = served_doc
            .find(r#"<script type="importmap">"#)
            .expect("inline tag present");
        let body = &served_doc[tag_start + r#"<script type="importmap">"#.len()..];
        let body = &body[..body.find("</script>").expect("tag closes")];
        let map: serde_json::Value = serde_json::from_str(body).expect("inline body parses");
        let imports = map["imports"].as_object().expect("imports object");
        let ui_client = imports["ui/client"].as_str().expect("ui/client mapped");
        assert!(
            ui_client.starts_with("/assets/ui/client."),
            "hashed: {ui_client}"
        );
        assert!(
            !served_doc.contains("/assets/islands-load.js"),
            "logical URL rewritten: {served_doc}"
        );
    }

