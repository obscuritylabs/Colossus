//! Display-only hints for program-specific options. Never resolve or run a command.

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
    let mut profiles: Vec<_> = [
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
        (&["openssl"][..], "kK", false, false),
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
    .collect();
    // Single-dash named options are not grouped short flags. Match complete
    // value-taking names, not command verbs such as keytool's -storepasswd.
    // Keep these patterns shared by shell spelling and prepared argv handling.
    for (programs, option) in [
        (
            &["keytool"][..],
            r"(?i)-(?:(?:src|dest)?(?:store|key)pass|new)",
        ),
        (&["jarsigner"][..], r"(?i)-(?:store|key)pass"),
        (
            &["openssl"][..],
            r"-(?:pwri_password|password|passin|passout|secretkey|hmac|macopt)",
        ),
    ] {
        let names = programs.join("|");
        profiles.push(Profile {
            programs,
            login_only: false,
            cue: Regex::new(&format!(r"(?i)\b(?:{names})(?:\.exe)?\b"))
                .expect("constant named credential program hint"),
            argument: Regex::new(&format!(r"^{option}(?:=|$)"))
                .expect("constant named credential argument option"),
            shell: Regex::new(&format!(r"(?:^|[\s;&|]){option}[\s=]+"))
                .expect("constant named credential shell option"),
        });
    }
    profiles
});

impl Profile {
    pub(super) fn value_start(&self, argument: &str) -> Option<usize> {
        self.argument.find(argument).map(|matched| matched.end())
    }
}

pub(super) fn for_invocation(executable: &str, arguments: &[Value]) -> Vec<&'static Profile> {
    let name = executable_name(executable);
    // Carry only literal program hints through ordinary launchers. A package
    // named mysql in `cargo test -p mysql` is not an invoked credential utility.
    // This never resolves PATH, evaluates script code, or grants execution.
    let wrapper = [
        "env",
        "sudo",
        "doas",
        "nice",
        "nohup",
        "timeout",
        "gtimeout",
        "stdbuf",
        "setsid",
        "chrt",
        "taskset",
        "prlimit",
        "runuser",
        "xargs",
        "parallel",
        "busybox",
        "sh",
        "bash",
        "dash",
        "zsh",
        "fish",
        "ksh",
        "cmd",
        "pwsh",
        "powershell",
        "wsl",
        "ssh",
    ]
    .contains(&name.as_str());
    let candidates = std::iter::once(name)
        .chain(arguments.iter().filter_map(|argument| {
            wrapper
                .then(|| argument.as_str().map(executable_name))
                .flatten()
        }))
        .collect::<Vec<_>>();
    PROFILES
        .iter()
        .filter(|profile| {
            candidates
                .iter()
                .any(|name| profile.programs.contains(&name.as_str()))
                && (!profile.login_only
                    || arguments
                        .iter()
                        .any(|argument| argument.as_str() == Some("login")))
        })
        .collect()
}

fn executable_name(value: &str) -> String {
    let name = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_owned()
}

pub(super) fn ranges(
    text: &str,
    spelling: &str,
    original_ends: &[usize],
    invocation: &[&Profile],
) -> Vec<Range<usize>> {
    let segments = super::shell_value::command_segments(text);
    let mut output = Vec::new();
    for profile in PROFILES.iter() {
        let selected = invocation
            .iter()
            .any(|selected| std::ptr::eq(*selected, profile));
        if !selected && !profile.cue.is_match(text) && !profile.cue.is_match(spelling) {
            continue;
        }
        let candidates =
            super::shell_value::ranges(text, &[&profile.shell], spelling, original_ends);
        if selected {
            output.extend(candidates);
            continue;
        }
        // Keep the earliest hint per segment. Repeated program words must not
        // rescan the rest of a long command or make this projection quadratic.
        let mut hints = vec![None; segments.len()];
        for end in profile
            .cue
            .find_iter(text)
            .map(|matched| matched.end())
            .chain(
                profile
                    .cue
                    .find_iter(spelling)
                    .map(|matched| original_ends[matched.end() - 1]),
            )
        {
            let index = segments.partition_point(|segment| segment.end < end);
            if let Some(segment) = segments.get(index)
                && segment.start < end
                && end <= segment.end
            {
                hints[index] = Some(hints[index].map_or(end, |previous: usize| previous.min(end)));
            }
        }
        output.extend(candidates.into_iter().filter(|range| {
            let index = segments.partition_point(|segment| segment.end <= range.start);
            hints
                .get(index)
                .copied()
                .flatten()
                .is_some_and(|end| range.start >= end)
        }));
    }
    output
}
