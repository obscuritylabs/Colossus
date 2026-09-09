//! Display-only hints for ambiguous short options. Never resolve or run a command.

use std::{ops::Range, sync::LazyLock};

use regex::Regex;
use serde_json::Value;

pub(super) struct Profile {
    programs: &'static [&'static str],
    login_only: bool,
    cue: Regex,
    argument: Regex,
    shell: Regex,
}

static PROFILES: LazyLock<Vec<Profile>> = LazyLock::new(|| {
    // Programs, value-taking flags, attached/grouped short forms, login-only.
    [
        (&["ssh-keygen"][..], "NP", true, false),
        (
            &[
                "sshpass", "mysql", "mariadb", "mongo", "mongosh", "7z", "7za", "7zz",
            ][..],
            "p",
            true,
            false,
        ),
        (&["redis-cli"][..], "a", true, false),
        (&["security"][..], "w", true, false),
        (&["openssl"][..], "k", false, false),
        (&["docker", "podman"][..], "p", true, true),
    ]
    .into_iter()
    .map(|(programs, flags, attached, login_only)| {
        let names = programs
            .iter()
            .map(|name| regex::escape(name))
            .collect::<Vec<_>>()
            .join("|");
        let invocation = if login_only {
            r"[^\r\n;&|]*\blogin\b"
        } else {
            ""
        };
        let option = if attached {
            format!(r"-[a-zA-Z#]*?[{flags}]")
        } else {
            format!(r"-[{flags}]")
        };
        Profile {
            programs,
            login_only,
            cue: Regex::new(&format!(r"(?i)\b(?:{names})(?:\.exe)?\b{invocation}"))
                .expect("constant credential program hint"),
            argument: Regex::new(&format!(
                "^{option}{}",
                if attached { "" } else { "(?:=|$)" }
            ))
            .expect("constant credential argument option"),
            shell: Regex::new(&format!(
                r"(?:^|[\s;&|]){option}{}",
                if attached { r"[\s=]*" } else { r"[\s=]+" }
            ))
            .expect("constant credential shell option"),
        }
    })
    .collect()
});

impl Profile {
    pub(super) fn value_start(&self, argument: &str) -> Option<usize> {
        self.argument.find(argument).map(|matched| matched.end())
    }
}

pub(super) fn for_invocation(executable: &str, arguments: &[Value]) -> Vec<&'static Profile> {
    let name = executable
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(executable)
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    PROFILES
        .iter()
        .filter(|profile| {
            profile.programs.contains(&name)
                && (!profile.login_only
                    || arguments
                        .iter()
                        .any(|argument| argument.as_str() == Some("login")))
        })
        .collect()
}

pub(super) fn ranges(
    text: &str,
    spelling: &str,
    original_ends: &[usize],
    invocation: &[&Profile],
) -> Vec<Range<usize>> {
    let prefixes = PROFILES
        .iter()
        .filter(|profile| {
            invocation
                .iter()
                .any(|selected| std::ptr::eq(*selected, *profile))
                || profile.cue.is_match(text)
                || profile.cue.is_match(spelling)
        })
        .map(|profile| &profile.shell)
        .collect::<Vec<_>>();
    super::shell_value::ranges(text, &prefixes, spelling, original_ends)
}
