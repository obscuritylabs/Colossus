use colossus_contracts::ToolSpec;
use serde_json::{Value, json};

/// Typed native desktop operations. Availability and authority come only from host composition.
pub(super) fn computer_specs() -> Vec<ToolSpec> {
    let mut specs = vec![
        tool(
            "computer.windows",
            "List bounded native Windows application choices. The requested count is an upper bound narrowed by workspace and native limits. Returns opaque window IDs, never operating-system handles. Computer use controls ordinary unsandboxed apps and may act outside the workspace.",
            object(
                json!({"max_windows":{"type":"integer","minimum":1,"maximum":128}}),
                &[],
            ),
        ),
        tool(
            "computer.open",
            "Acquire computer control for one previously observed opaque native window without activating it. Observe it next; if it is in the background, explicitly focus it and take a new snapshot before other input. A visible native control indicator lets the human pause or stop. Protected, elevated, and Colossus control windows are unavailable.",
            object(json!({"window_id":id("w")}), &["window_id"]),
        ),
        tool(
            "computer.status",
            "Read this run's current native computer session and control generation, including human pause or resume. Omit session_id to discover only the current owned session. This never resumes control.",
            object(json!({"session_id":id("s")}), &[]),
        ),
        tool(
            "computer.snapshot",
            "Read bounded native accessibility semantics without activating the selected window or positively owned same-app dialog. Match observation.window_id in observation.windows; if foreground is false, use computer.focus for that observed window and take a new snapshot before other input. The requested node count is narrowed by workspace and native limits. Sensitive values are withheld. Each input needs a fresh observation.",
            session(
                json!({"max_nodes":{"type":"integer","minimum":1,"maximum":1024}}),
                &[],
            ),
        ),
        tool(
            "computer.screenshot",
            "Capture the actual selected native window as a verified PNG without activating it. After output policy release, returns an inline image and owner-only artifact plus a fresh observation. Use computer.snapshot to inspect foreground and accessible controls before input. No paths or encoded image arguments.",
            session(json!({}), &[]),
        ),
        tool(
            "computer.close",
            "Release native automation and its writer lease. Leaves the human's application running. Human Stop or run cancellation cannot be bypassed by closing and reopening.",
            session(json!({}), &[]),
        ),
    ];
    for (name, description, properties, required) in [
        (
            "computer.focus",
            "Explicitly activate the selected window or a positively owned same-app dialog using its observed window_id and a fresh observation. Use when the observed target has foreground=false. Consumes the observation; take a new snapshot before other input. Windows may refuse activation; do not force focus through another route.",
            json!({"window_id":id("w")}),
            vec!["window_id"],
        ),
        (
            "computer.move",
            "Move the native pointer to a local physical pixel in the observed target window. Consumes the fresh observation.",
            json!({"x":coordinate(),"y":coordinate()}),
            vec!["x", "y"],
        ),
        (
            "computer.click",
            "Click a fresh local physical point or observed semantic element in the foreground target. Does not activate a background window: explicitly computer.focus its observed window_id, then take a new snapshot first. May submit data or trigger actions in an unsandboxed app; inspect first.",
            json!({"target":{"oneOf":[object(json!({"kind":{"const":"point"},"x":coordinate(),"y":coordinate()}), &["kind","x","y"]),object(json!({"kind":{"const":"element"},"element_id":id("e")}), &["kind","element_id"])]},"button":{"type":"string","enum":["left","right","middle"]},"count":{"type":"integer","minimum":1,"maximum":2}}),
            vec!["target", "button", "count"],
        ),
        (
            "computer.drag",
            "Drag between local physical pixels within the fresh observed target, then release the button. Queues one bounded balanced native trajectory; steps defaults to 16 and does not promise wall-clock timing.",
            json!({"from_x":coordinate(),"from_y":coordinate(),"to_x":coordinate(),"to_y":coordinate(),"steps":{"type":"integer","minimum":1,"maximum":100,"default":16}}),
            vec!["from_x", "from_y", "to_x", "to_y"],
        ),
        (
            "computer.scroll",
            "Scroll at a local physical point with bounded native wheel units. Consumes the observation and may trigger application handlers.",
            json!({"x":coordinate(),"y":coordinate(),"delta_x":{"type":"integer","minimum":-1200,"maximum":1200},"delta_y":{"type":"integer","minimum":-1200,"maximum":1200}}),
            vec!["x", "y", "delta_x", "delta_y"],
        ),
        (
            "computer.type",
            "Type at most 8192 UTF-8 bytes into the observed ordinary native field. Password and protected fields require human entry. Consumes the fresh observation.",
            json!({"text":{"type":"string","minLength":1,"maxLength":8192}}),
            vec!["text"],
        ),
        (
            "computer.press",
            "Press a bounded balanced key chord in the observed app. Use Control/Alt/Shift, uppercase letters, digits, Enter, Tab, Escape, navigation keys, or F1–F12. Global Windows and app-switching shortcuts are unavailable.",
            json!({"keys":{"type":"array","minItems":1,"maxItems":5,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":10}}}),
            vec!["keys"],
        ),
    ] {
        specs.push(tool(name, description, observation(properties, &required)));
    }
    specs
}

fn tool(name: &str, description: &str, input_schema: Value) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema,
        effect_action: Some(name.into()),
        capability: Some(name.into()),
        max_output_bytes: if name == "computer.screenshot" {
            16 * 1024 * 1024
        } else {
            1024 * 1024
        },
    }
}

fn id(kind: &str) -> Value {
    json!({"type":"string","minLength":37,"maxLength":37,"pattern":format!("^cu_{kind}_[0-9a-f]{{32}}$")})
}

fn coordinate() -> Value {
    json!({"type":"integer","minimum":0,"maximum":i32::MAX})
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
}

fn session(properties: Value, required: &[&str]) -> Value {
    let mut all = json!({"session_id":id("s"),"control_generation":{"type":"integer","minimum":1,"maximum":u64::MAX}});
    if let (Some(all), Some(properties)) = (all.as_object_mut(), properties.as_object()) {
        all.extend(properties.clone());
    }
    let mut fields = vec!["session_id", "control_generation"];
    fields.extend(required);
    object(all, &fields)
}

fn observation(properties: Value, required: &[&str]) -> Value {
    let mut all = json!({"observation_id":id("o")});
    if let (Some(all), Some(properties)) = (all.as_object_mut(), properties.as_object()) {
        all.extend(properties.clone());
    }
    let mut fields = vec!["observation_id"];
    fields.extend(required);
    session(all, &fields)
}
