//! Conservative display-only word boundaries, never shell evaluation or rewriting.

use regex::Regex;

pub(super) fn ranges(text: &str, prefix: &Regex) -> Vec<std::ops::Range<usize>> {
    let mut output = Vec::new();
    let mut cursor = 0;
    for matched in prefix.find_iter(text) {
        if matched.start() < cursor {
            continue;
        }
        let end = matched.end() + word_end(&text[matched.end()..]);
        if end == matched.end() {
            continue;
        }
        output.push(matched.end()..end);
        cursor = end;
    }
    output
}

#[derive(Clone, Copy)]
enum Nested {
    Quote(char),
    Group(char),
}

fn word_end(text: &str) -> usize {
    let mut nested = Vec::new();
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        // Ambiguous/unclosed words and excessive nesting conservatively redact
        // the remainder. Never release a tail whose credential boundary is unknown.
        if nested.len() >= 64 {
            return text.len();
        }
        match nested.last().copied() {
            Some(Nested::Quote(quote)) => {
                if character == quote {
                    nested.pop();
                } else if quote != '\'' && character == '\\' {
                    characters.next();
                } else if quote == '"' && character == '`' {
                    nested.push(Nested::Quote('`'));
                } else if quote == '"'
                    && character == '$'
                    && let Some((_, next @ ('(' | '{'))) = characters.peek().copied()
                {
                    characters.next();
                    nested.push(Nested::Group(if next == '(' { ')' } else { '}' }));
                }
            }
            _ => match character {
                '\\' | '^' => {
                    characters.next();
                }
                '\'' | '"' | '`' => nested.push(Nested::Quote(character)),
                '(' => nested.push(Nested::Group(')')),
                '{' => nested.push(Nested::Group('}')),
                '[' => nested.push(Nested::Group(']')),
                ')' | '}' | ']' => {
                    if matches!(nested.last(), Some(Nested::Group(end)) if *end == character) {
                        nested.pop();
                    } else {
                        return text.len();
                    }
                }
                value
                    if nested.is_empty() && (value.is_whitespace() || ";&|<>".contains(value)) =>
                {
                    return index;
                }
                _ => {}
            },
        }
    }
    text.len()
}
