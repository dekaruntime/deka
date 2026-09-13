#[cfg(test)]
mod tests {
    use crate::websocket::build_patch_from_snapshot;
    use serde_json::Value;

    fn parse(payload: &str) -> Value {
        serde_json::from_str(payload).expect("valid json payload")
    }

    #[test]
    fn first_snapshot_uses_container_replace() {
        let payload = build_patch_from_snapshot(
            "/__hmr_test_first",
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Hello</div>",
        );
        let json = parse(&payload);
        assert_eq!(json["type"], "patch");
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["op"], "set_html");
        assert_eq!(json["ops"][0]["selector"], "#app");
    }

    #[test]
    fn stable_structure_produces_granular_node_patch() {
        let path = "/__hmr_test_granular";
        let first = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Hello</div><span data-deka-id=\"b\">World</span>",
        );
        let first_json = parse(&first);
        assert_eq!(first_json["ops"][0]["selector"], "#app");

        let second = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Hello 2</div><span data-deka-id=\"b\">World</span>",
        );
        let second_json = parse(&second);
        assert_eq!(second_json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(second_json["ops"][0]["selector"], "[data-deka-id=\"a\"]");
        assert_eq!(second_json["ops"][0]["html"], "Hello 2");
    }

    fn b64(value: &str) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
    }

    // Builds an island exactly the way the paused ui/server runtime emits
    // it: comment markers carrying the b64-encoded component name, directive,
    // and props around the server-rendered body.
    fn island(name: &str, directive: &str, props: &str, body: &str) -> String {
        format!(
            "<!--deka-island start:{} directive:{} props:{}-->{}<!--deka-island end:{}-->",
            b64(name),
            b64(directive),
            b64(props),
            body,
            b64(name)
        )
    }

    #[test]
    fn structural_changes_fallback_to_island_patch_when_possible() {
        let path = "/__hmr_test_island";
        let _ = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &island("Widget", "load", "{}", "<div data-deka-id=\"n1\">A</div>"),
        );
        let payload = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &island(
                "Widget",
                "load",
                "{}",
                "<section data-deka-id=\"n2\">B</section>",
            ),
        );
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "Widget");
        assert_eq!(json["ops"][0]["occurrence"], 1);
        assert_eq!(
            json["ops"][0]["html"],
            "<section data-deka-id=\"n2\">B</section>"
        );
    }

    #[test]
    fn multiple_islands_patch_only_the_changed_island() {
        let path = "/__hmr_test_island_multi";
        let _ = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &format!(
                "{}{}",
                island("WidgetA", "load", "{}", "<div data-deka-id=\"a\">A</div>"),
                island("WidgetB", "load", "{}", "<div data-deka-id=\"b\">B</div>")
            ),
        );
        let payload = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &format!(
                "{}{}",
                island("WidgetA", "load", "{}", "<div data-deka-id=\"a\">A</div>"),
                island(
                    "WidgetB",
                    "load",
                    "{}",
                    "<section data-deka-id=\"b2\">B2</section>"
                )
            ),
        );
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "WidgetB");
        assert_eq!(json["ops"][0]["occurrence"], 1);
        assert_eq!(
            json["ops"][0]["html"],
            "<section data-deka-id=\"b2\">B2</section>"
        );
    }

    #[test]
    fn island_name_change_forces_container_fallback_patch() {
        let path = "/__hmr_test_island_id_change";
        let _ = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &island("WidgetA", "load", "{}", "<div data-deka-id=\"n1\">A</div>"),
        );
        let payload = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &island("WidgetB", "load", "{}", "<div data-deka-id=\"n1\">A</div>"),
        );
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["selector"], "#app");
    }

    #[test]
    fn island_patch_works_with_real_rendered_marker_shape() {
        // Regression test for the marker/element drift: snapshots must be
        // shaped like actual server.js output (padded b64, real props JSON),
        // not like synthetic wrapper elements.
        let path = "/__hmr_test_island_real_shape";
        let first = format!(
            "{}<p>static</p>",
            island(
                "Counter",
                "load",
                "{\"count\":1}",
                "<button data-deka-id=\"test:Counter/i0\">0</button>"
            )
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &first);
        let second = format!(
            "{}<p>static</p>",
            island(
                "Counter",
                "load",
                "{\"count\":1}",
                "<button data-deka-id=\"test:Counter/i1\">1</button>"
            )
        );
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &second);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "Counter");
        assert_eq!(json["ops"][0]["occurrence"], 1);
        assert_eq!(
            json["ops"][0]["html"],
            "<button data-deka-id=\"test:Counter/i1\">1</button>"
        );
    }

    #[test]
    fn defer_shaped_markers_with_extra_fields_are_collected() {
        // wrapDeferred (server.js:433) emits markers with id/enc/cache fields
        // and no props; those must be located too.
        let path = "/__hmr_test_island_defer_shape";
        let marker_a = format!(
            "<!--deka-island start:{} directive:{} id:{} cache:{}--><span data-deka-defer=\"D:1\"><span data-deka-id=\"test:_/i0/i0\" slot=\"fallback\">.</span></span><!--deka-island end:{}-->",
            b64("Badge"),
            b64("defer"),
            b64("D:1"),
            b64("60s"),
            b64("Badge")
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &marker_a);
        let marker_b = format!(
            "<!--deka-island start:{} directive:{} id:{} cache:{}--><span data-deka-defer=\"D:1\"><span data-deka-id=\"test:_/i0/i1\" slot=\"fallback\">!</span></span><!--deka-island end:{}-->",
            b64("Badge"),
            b64("defer"),
            b64("D:1"),
            b64("60s"),
            b64("Badge")
        );
        let payload =
            build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &marker_b);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "Badge");
        assert_eq!(json["ops"][0]["occurrence"], 1);
        assert_eq!(
            json["ops"][0]["html"],
            "<span data-deka-defer=\"D:1\"><span data-deka-id=\"test:_/i0/i1\" slot=\"fallback\">!</span></span>"
        );
    }

    #[test]
    fn repeated_island_names_patch_by_occurrence() {
        let path = "/__hmr_test_island_duplicate_names";
        let _ = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &format!(
                "{}{}",
                island("Counter", "load", "{}", "<i data-deka-id=\"a1\">0</i>"),
                island("Counter", "load", "{}", "<i data-deka-id=\"b1\">10</i>")
            ),
        );
        let payload = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &format!(
                "{}{}",
                island("Counter", "load", "{}", "<i data-deka-id=\"a1\">0</i>"),
                island("Counter", "load", "{}", "<i data-deka-id=\"b2\">11</i>")
            ),
        );
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "Counter");
        assert_eq!(json["ops"][0]["occurrence"], 2);
        assert_eq!(json["ops"][0]["html"], "<i data-deka-id=\"b2\">11</i>");
    }

    // Mirrors the client-side resolution in the HMR client (crates/http/src/
    // router.rs): find the `occurrence`-th start marker for `name`, walk to
    // the end marker that closes its depth, and return the body between the
    // two. Kept separate from `collect_islands` on purpose: the test must
    // resolve the op the way the browser will, not the way the producer
    // parses.
    fn resolve_island_body(html: &str, name: &str, occurrence: usize) -> Option<String> {
        let name_b64 = b64(name);
        let start_needle = "<!--deka-island start:";
        let marker_needle = "<!--deka-island ";
        let mut seen = 0usize;
        let mut offset = 0usize;
        while let Some(pos) = html[offset..].find(start_needle) {
            let abs = offset + pos;
            let header_start = abs + start_needle.len();
            let header_end = header_start + html[header_start..].find("-->")?;
            if html[header_start..header_end].split(' ').next() == Some(name_b64.as_str()) {
                seen += 1;
                if seen == occurrence {
                    let body_start = header_end + 3;
                    let mut depth = 1usize;
                    let mut cursor = body_start;
                    while let Some(rel) = html[cursor..].find(marker_needle) {
                        let marker_abs = cursor + rel;
                        let after = marker_abs + marker_needle.len();
                        if html[after..].starts_with("start:") {
                            depth += 1;
                        } else if html[after..].starts_with("end:") {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                return Some(html[body_start..marker_abs].to_string());
                            }
                        }
                        cursor = after;
                    }
                    return None;
                }
            }
            offset = header_end + 3;
        }
        None
    }

    #[test]
    fn island_op_resolves_to_body_between_server_markers() {
        // Asserts the EFFECT, not the op encoding: given the island HTML
        // server.js actually emits (comment markers around a fragment body
        // with no wrapper element), the op's island identity must resolve to
        // the exact range the client will replace. The old
        // `[data-deka-island-id=...]` selector could never be checked this
        // way, which is how the drift shipped.
        let path = "/__hmr_test_island_effect";
        let first = format!(
            "<main>{}<p>static</p></main>",
            island(
                "Widget",
                "load",
                "{}",
                "<b data-deka-id=\"w1\">old</b><i>body</i>"
            )
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &first);
        let second = format!(
            "<main>{}<p>static</p></main>",
            island(
                "Widget",
                "load",
                "{}",
                "<b data-deka-id=\"w2\">new</b><i>body</i>"
            )
        );
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &second);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        let op = &json["ops"][0];
        assert_eq!(op["island"], "Widget");
        assert_eq!(op["occurrence"], 1);
        // The identity must resolve to the island range in the live document
        // (modelled here by the previous render) ...
        let name = op["island"].as_str().unwrap_or_default().to_string();
        let occurrence = op["occurrence"].as_u64().unwrap_or(1) as usize;
        assert_eq!(
            resolve_island_body(&first, &name, occurrence).as_deref(),
            Some("<b data-deka-id=\"w1\">old</b><i>body</i>")
        );
        // ... and the op payload must be exactly the new body between the
        // same markers.
        assert_eq!(
            resolve_island_body(&second, &name, occurrence).as_deref(),
            Some(op["html"].as_str().unwrap_or_default())
        );
    }

    #[test]
    fn island_op_occurrence_resolves_to_second_instance() {
        // Effect check for repeated components: occurrence 2 must resolve to
        // the SECOND island's range, not the first.
        let path = "/__hmr_test_island_effect_occurrence";
        let first = format!(
            "{}{}",
            island("Counter", "load", "{}", "<i data-deka-id=\"a1\">0</i>"),
            island("Counter", "load", "{}", "<i data-deka-id=\"b1\">10</i>")
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &first);
        let second = format!(
            "{}{}",
            island("Counter", "load", "{}", "<i data-deka-id=\"a1\">0</i>"),
            island("Counter", "load", "{}", "<i data-deka-id=\"b2\">11</i>")
        );
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &second);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        let op = &json["ops"][0];
        assert_eq!(op["island"], "Counter");
        assert_eq!(op["occurrence"], 2);
        let name = op["island"].as_str().unwrap_or_default().to_string();
        let occurrence = op["occurrence"].as_u64().unwrap_or(1) as usize;
        assert_eq!(
            resolve_island_body(&first, &name, occurrence).as_deref(),
            Some("<i data-deka-id=\"b1\">10</i>")
        );
        assert_eq!(
            resolve_island_body(&second, &name, occurrence).as_deref(),
            Some(op["html"].as_str().unwrap_or_default())
        );
    }

    #[test]
    fn unterminated_island_marker_falls_back_to_container_patch() {
        let path = "/__hmr_test_island_unterminated";
        let first = format!(
            "{}<p>tail</p>",
            island("Widget", "load", "{}", "<div data-deka-id=\"n1\">A</div>")
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &first);
        // No end marker: the island cannot be bounded, so no island op may be
        // emitted for it.
        let second = "<!--deka-island start:V2lkZ2V0 directive:bG9hZA== props:e30=--><div data-deka-id=\"n2\">B</div><p>tail</p>";
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", second);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["selector"], "#app");
    }

    #[test]
    fn granular_patch_payload_is_smaller_than_full_replace() {
        let full_path = "/__hmr_test_size_full";
        let full_payload = build_patch_from_snapshot(
            full_path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Alpha</div><div data-deka-id=\"b\">Bravo</div>",
        );

        let patch_path = "/__hmr_test_size_patch";
        let _ = build_patch_from_snapshot(
            patch_path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Alpha</div><div data-deka-id=\"b\">Bravo</div>",
        );
        let patch_payload = build_patch_from_snapshot(
            patch_path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Alpha 2</div><div data-deka-id=\"b\">Bravo</div>",
        );
        assert!(patch_payload.len() < full_payload.len());
    }
}
