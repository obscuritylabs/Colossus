//! Discovery query behavior against the active registry and per-turn tool ceiling.
use super::{DiscoverableToolExecutor, UnusedToolExecutor};
use colossus_contracts::{ExecutionContext, ToolCall, ToolSpec};
use colossus_ports::{ToolError, ToolExecutor, ToolRegistry};
use serde_json::{Value, json};
use std::sync::Arc;

fn native_search() -> DiscoverableToolExecutor {
    DiscoverableToolExecutor {
        registry: Arc::new(
            colossus_tools::StaticToolRegistry::builtins(&[
                "tool.search".into(),
                "computer.windows".into(),
                "computer.open".into(),
                "computer.snapshot".into(),
                "echo".into(),
            ])
            .expect("catalog"),
        ),
        inner: Arc::new(UnusedToolExecutor),
    }
}

async fn search(
    executor: &DiscoverableToolExecutor,
    query: &str,
    offered: &[&str],
    max_results: u64,
) -> Result<Value, ToolError> {
    let result = executor
        .execute(
            ToolCall {
                call_id: "discovery".into(),
                name: "tool.search".into(),
                arguments: json!({"query": query, "max_results": max_results}),
            },
            ExecutionContext {
                offered_tools: offered.iter().map(|name| (*name).into()).collect(),
                ..ExecutionContext::default()
            },
        )
        .await?;
    Ok(serde_json::from_str(&result.output).expect("search JSON"))
}

#[tokio::test]
async fn tool_search_attached_plus_preserves_ranking_and_result_bounds() {
    let executor = native_search();
    let offered = [
        "tool.search",
        "computer.windows",
        "computer.open",
        "computer.snapshot",
    ];
    for (plain, prefixed, truncated) in [
        ("computer windows", "+computer windows", true),
        ("computer windows", "+computer +windows", true),
        ("computer.windows", "+computer.windows", false),
    ] {
        let plain = search(&executor, plain, &offered, 1).await.unwrap();
        let prefixed = search(&executor, prefixed, &offered, 1).await.unwrap();
        assert_eq!(prefixed["tools"], plain["tools"]);
        assert_eq!(prefixed["tools"].as_array().unwrap().len(), 1);
        assert_eq!(prefixed["tools"][0]["name"], "computer.windows");
        assert_eq!(prefixed["truncated"], truncated);
        assert_eq!(prefixed["truncated"], plain["truncated"]);
    }
}

#[tokio::test]
async fn tool_search_plus_cannot_discover_unoffered_native_tools() {
    let executor = native_search();
    for query in [
        "computer",
        "+computer",
        "+computer windows",
        "+computer.windows",
    ] {
        let result = search(&executor, query, &["tool.search", "echo"], 50)
            .await
            .unwrap();
        assert_eq!(result["tools"], json!([]));
        assert_eq!(result["truncated"], false);
    }
}

#[tokio::test]
async fn tool_search_preserves_literal_language_and_flag_punctuation() {
    let mut specs = colossus_tools::StaticToolRegistry::builtins(&["echo".into()])
        .unwrap()
        .list_specs();
    let mut cpp: ToolSpec = specs.pop().unwrap();
    cpp.name = "source.inspect".into();
    cpp.description = "Read C++ sources and filenames from rg --files.".into();
    let mut c = cpp.clone();
    c.name = "text.inspect".into();
    c.description = "Read C files.".into();
    let executor = DiscoverableToolExecutor {
        registry: Arc::new(colossus_tools::StaticToolRegistry::new([cpp, c]).unwrap()),
        inner: Arc::new(UnusedToolExecutor),
    };
    for query in ["C++", "+C++", "--files", "+--files"] {
        let result = search(&executor, query, &["source.inspect", "text.inspect"], 50)
            .await
            .unwrap();
        assert_eq!(result["tools"].as_array().unwrap().len(), 1);
        assert_eq!(result["tools"][0]["name"], "source.inspect");
    }
    let repeated = search(&executor, "++C++", &["source.inspect", "text.inspect"], 50)
        .await
        .unwrap();
    assert_eq!(repeated["tools"], json!([]));
}

#[tokio::test]
async fn tool_search_missing_terms_never_enumerates_the_catalog() {
    let executor = native_search();
    for query in ["+", "computer +", " \t "] {
        assert!(matches!(
            search(&executor, query, &["tool.search", "computer.windows"], 50).await,
            Err(ToolError::InvalidArguments { .. })
        ));
    }
}
