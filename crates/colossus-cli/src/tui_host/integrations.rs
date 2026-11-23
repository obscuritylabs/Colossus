use colossus_presentation::{
    PresentationBlock, PresentationDocument, PresentationTone, document_from_json,
};
use serde_json::Value;

pub(super) fn pull_request_arguments(input: &str) -> Result<Value, String> {
    let (_, arguments) = input
        .trim()
        .split_once(char::is_whitespace)
        .filter(|(tool, _)| *tool == "github.pull_request")
        .ok_or_else(|| "Usage: /integration call github.pull_request JSON".to_owned())?;
    serde_json::from_str(arguments)
        .map_err(|_| "Pull request arguments must be valid inline JSON.".into())
}

pub(super) fn pull_request_document(result: &Value) -> PresentationDocument {
    let title = Some("GitHub pull request");
    let Some(mut observation) = result["result"].as_object().cloned() else {
        return document_from_json(result, title);
    };
    let checks = observation.remove("check_runs");
    let statuses = observation.remove("commit_statuses");
    let mut document = document_from_json(&Value::Object(observation), title);
    for (title, evidence) in [("Check runs", checks), ("Commit statuses", statuses)] {
        let Some(mut evidence) = evidence else {
            continue;
        };
        let items = evidence
            .as_object_mut()
            .and_then(|fields| fields.remove("items"));
        let mut body = document_from_json(&evidence, None).blocks;
        if let Some(items) = items {
            body.extend(document_from_json(&items, None).blocks);
        }
        document.push(PresentationBlock::Card {
            title: title.into(),
            tone: PresentationTone::Neutral,
            body,
        });
    }
    document
}
