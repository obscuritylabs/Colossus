//! Conservative display-only word boundaries, never shell evaluation or rewriting.

use regex::Regex;

pub(super) fn ranges(
    text: &str,
    prefixes: &[&Regex],
    spelling: &str,
    original_ends: &[usize],
) -> Vec<std::ops::Range<usize>> {
    let mut output = Vec::new();
    for prefix in prefixes {
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
    }
    // Recognize literal credential names split by shell quotes or escapes. This
    // auxiliary spelling is never executed and never substitutes variables or
    // commands. Every released mask is mapped back to the original input bytes.
    for prefix in prefixes {
        let mut cursor = 0;
        for matched in prefix.find_iter(spelling) {
            let start = matched
                .start()
                .checked_sub(1)
                .map_or(0, |i| original_ends[i]);
            let value_start = original_ends[matched.end() - 1];
            // Keep original boundaries when already recognizable (in particular,
            // do not skip an empty quoted credential to mask the following word).
            if value_start < cursor || prefix.is_match(&text[start..value_start]) {
                continue;
            }
            let end = value_start + word_end(&text[value_start..]);
            if end > value_start {
                output.push(value_start..end);
                cursor = end;
            }
        }
    }
    output
}

pub(super) fn literal_spelling(text: &str) -> (String, Vec<usize>) {
    let mut spelling = String::with_capacity(text.len());
    let mut original_ends = Vec::with_capacity(text.len());
    let mut characters = text.char_indices().peekable();
    while let Some((mut index, mut character)) = characters.next() {
        if matches!(character, '\'' | '"') {
            continue;
        }
        if matches!(character, '\\' | '^')
            && let Some((next_index, next)) = characters.next()
        {
            if next == '\n' {
                continue;
            }
            if next == '\r' && characters.peek().is_some_and(|(_, value)| *value == '\n') {
                characters.next();
                continue;
            }
            index = next_index;
            character = next;
        }
        spelling.push(character);
        original_ends.extend(std::iter::repeat_n(
            index + character.len_utf8(),
            character.len_utf8(),
        ));
    }
    (spelling, original_ends)
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
