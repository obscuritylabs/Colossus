use regex::Regex;
use std::sync::LazyLock;

static COMMAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(literal(?:[ \t]+|[.,?!:;][ \t]*))?(period|full(?:[ \t]+|[.,?!:;][ \t]*)stop|comma|question(?:[ \t]+|[.,?!:;][ \t]*)mark|exclamation(?:[ \t]+|[.,?!:;][ \t]*)(?:mark|point)|colon|semicolon|semi[ -]colon|new(?:[ \t]+|[.,?!:;][ \t]*)(?:line|paragraph))\b").expect("fixed punctuation expression")
});

/// Interpret explicit English spoken punctuation, with `literal` as an escape.
/// Reformat the complete owned speech suffix so split commands also work.
#[must_use]
pub fn format_spoken_punctuation(source: &str) -> String {
    let raw = source.trim();
    let mut result = String::new();
    let mut copied = 0;
    let mut after_command = false;
    for capture in COMMAND.captures_iter(raw) {
        let Some(matched) = capture.get(0) else {
            continue;
        };
        let literal = capture.get(1).is_some();
        let Some(command) = capture.get(2) else {
            continue;
        };
        let name = command.as_str().to_lowercase();
        let words: Vec<_> = name
            .split([' ', '\t', '.', ',', '?', '!', ':', ';'])
            .filter(|word| !word.is_empty())
            .collect();
        let name = words.join(" ");
        let punctuation = match name.as_str() {
            "period" | "full stop" => ".",
            "comma" => ",",
            "question mark" => "?",
            "exclamation mark" | "exclamation point" => "!",
            "colon" => ":",
            "semicolon" | "semi colon" | "semi-colon" => ";",
            "new line" => "\n",
            "new paragraph" => "\n\n",
            _ => continue,
        };
        let mut prose = &raw[copied..matched.start()];
        if !literal && !punctuation.starts_with('\n') {
            prose = prose.trim_end_matches([' ', '\t', '.', ',', '?', '!', ':', ';']);
        }
        append(&mut result, prose, after_command);
        if literal {
            append(&mut result, &name, after_command);
        } else {
            result.truncate(result.trim_end_matches([' ', '\t']).len());
            result.push_str(punctuation);
        }
        after_command = !literal;
        copied = matched.end();
        if !literal {
            let rest = &raw[copied..];
            let trimmed = rest.trim_start_matches([' ', '\t']);
            let without_punctuation = trimmed.trim_start_matches(['.', ',', '?', '!', ':', ';']);
            if trimmed.len() != without_punctuation.len() {
                copied = raw.len() - without_punctuation.len();
            }
        }
    }
    append(&mut result, &raw[copied..], after_command);
    result.trim_matches([' ', '\t']).to_owned()
}
fn append(result: &mut String, prose: &str, after_command: bool) {
    let prose = if after_command {
        prose.trim_start_matches([' ', '\t'])
    } else {
        prose
    };
    if after_command
        && !prose.is_empty()
        && !result.ends_with(char::is_whitespace)
        && !prose.starts_with(['\n', '.', ',', '?', '!', ':', ';'])
    {
        result.push(' ');
    }
    result.push_str(prose);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_literal_escape_and_joined_segments() {
        assert_eq!(
            format_spoken_punctuation("Ready question mark. Yes period"),
            "Ready? Yes."
        );
        assert_eq!(
            format_spoken_punctuation("say literal period and literal question mark"),
            "say period and question mark"
        );
        assert_eq!(
            format_spoken_punctuation("First new paragraph second comma next"),
            "First\n\nsecond, next"
        );
        assert_eq!(format_spoken_punctuation("Fine. question mark"), "Fine?");
    }
}
