//! Rich input and native presentation marks retain separate effect identities.
use super::*;

pub(super) fn specs() -> Vec<ToolSpec> {
    let mut specs = vec![
        tool(
            "browser.evaluate",
            "Inspect the current owned page with a bounded JavaScript expression. Native V8 side-effect checking is mandatory: DOM writes, network calls, promises and debugger control are unavailable. Returns only bounded JSON values; documents containing protected fields or child-document embeddings are rejected.",
            snapshot_schema(
                json!({"expression":{"type":"string","minLength":1,"maxLength":8192}}),
                &["expression"],
            ),
        ),
        tool(
            "browser.cdp",
            "Run a bounded page inspection method through the owned authenticated host. Only listed methods are accepted. Browser/Target/Network control, file access and debugger connections are unavailable; documents containing protected fields or child-document embeddings are rejected. Native node IDs are valid only in this same owned document and snapshot.",
            snapshot_schema(
                json!({"method":{"type":"string","enum":["Page.getLayoutMetrics","Performance.getMetrics","DOM.getDocument","DOM.querySelector","DOM.querySelectorAll","DOM.getAttributes","DOM.getBoxModel","DOM.getOuterHTML","CSS.getComputedStyleForNode","CSS.getMediaQueries"]},"params":{"type":"object"}}),
                &["method", "params"],
            ),
        ),
        tool(
            "browser.hover",
            "Hover over one fresh referenced element in the owned page. Hover can trigger page handlers; it never moves the operating-system cursor.",
            element_schema(json!({}), &[]),
        ),
        tool(
            "browser.double_click",
            "Double-click one fresh referenced element with a real guest pointer sequence. This may submit or change page data.",
            element_schema(json!({}), &[]),
        ),
        tool(
            "browser.right_click",
            "Right-click one fresh referenced element, including page-authored context-menu handlers. Native browser and operating-system menus remain suppressed.",
            element_schema(json!({}), &[]),
        ),
        tool(
            "browser.drag",
            "Drag with the primary guest pointer button from one fresh element to another in the SAME document and snapshot. Refresh the snapshot after the drag.",
            element_schema(json!({"to": reference()}), &["to"]),
        ),
        tool(
            "browser.click_at",
            "Click a point in an inspected screenshot. Supply its exact physical viewport dimensions and the current snapshot ID; resized viewports and stale observations are rejected. Coordinates target only page content.",
            coordinate_schema(
                json!({
                    "point": point(), "button":{"type":"string","enum":["left","middle","right"]},
                    "click_count":{"type":"integer","minimum":1,"maximum":2}
                }),
                &["point"],
            ),
        ),
        tool(
            "browser.drag_at",
            "Drag a finite primary-button path in an inspected screenshot. Supply exact physical viewport dimensions and a current snapshot ID; all points must stay inside that viewport.",
            coordinate_schema(
                json!({"path":{"type":"array","minItems":2,"maxItems":64,"items":point()}}),
                &["path"],
            ),
        ),
        tool(
            "browser.annotation.clear",
            "Clear the current owned page's native annotation layer. This does not change the DOM, focus, or input.",
            document_schema(json!({}), &[]),
        ),
        tool(
            "browser.screenshot.annotated",
            "Capture the current viewport and its admitted native annotations as an owner-only PNG artifact through mandatory output policy. Ordinary browser.screenshot remains unmarked.",
            document_schema(json!({}), &[]),
        ),
    ];
    specs.push(tool("browser.annotate", "Replace native highlights and plain-text labels for the current page, without DOM injection or browser input. Marks are bound to this snapshot, document, writer and exact screenshot dimensions; navigation, resizing, control loss, expiry or explicit clear removes them.", coordinate_schema(json!({
        "annotations":{"type":"array","minItems":1,"maxItems":16,"items": object_schema(json!({
            "target":{"oneOf":[
                object_schema(json!({"kind":{"const":"element"},"element":reference()}), &["kind","element"]),
                object_schema(json!({"kind":{"const":"rectangle"},"rect":object_schema(json!({
                    "x":{"type":"integer","minimum":0,"maximum":4095},
                    "y":{"type":"integer","minimum":0,"maximum":4095},
                    "width":{"type":"integer","minimum":1,"maximum":4096},
                    "height":{"type":"integer","minimum":1,"maximum":4096}
                }), &["x","y","width","height"])}), &["kind","rect"])
            ]},
            "label":{"type":"string","maxLength":160},
            "color":{"type":"string","enum":["blue","red","green","yellow"]}
        }), &["target","label"])},
        "ttl_ms":{"type":"integer","minimum":1,"maximum":300000}
    }), &["annotations","ttl_ms"])));
    specs
}

fn reference() -> Value {
    object_schema(
        json!({"document_id":id_schema("bd"),"snapshot_id":id_schema("bn"),"element_id":id_schema("be")}),
        &["document_id", "snapshot_id", "element_id"],
    )
}
fn snapshot_schema(properties: Value, required: &[&str]) -> Value {
    let mut all = json!({"snapshot_id":id_schema("bn")});
    if let (Some(all), Some(properties)) = (all.as_object_mut(), properties.as_object()) {
        all.extend(properties.clone());
    }
    let mut fields = vec!["snapshot_id"];
    fields.extend(required);
    document_schema(all, &fields)
}

fn point() -> Value {
    object_schema(
        json!({"x":{"type":"integer","minimum":0,"maximum":4095},"y":{"type":"integer","minimum":0,"maximum":4095}}),
        &["x", "y"],
    )
}

fn coordinate_schema(properties: Value, required: &[&str]) -> Value {
    let mut all = json!({"snapshot_id":id_schema("bn"),"viewport":object_schema(json!({
        "width":{"type":"integer","minimum":1,"maximum":4096},
        "height":{"type":"integer","minimum":1,"maximum":4096}
    }), &["width","height"])});
    all.as_object_mut()
        .unwrap()
        .extend(properties.as_object().unwrap().clone());
    let mut fields = vec!["snapshot_id", "viewport"];
    fields.extend(required);
    document_schema(all, &fields)
}
