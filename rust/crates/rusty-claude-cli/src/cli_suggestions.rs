//! Suggesting a correction: the "did you mean" machinery behind a mistyped
//! option, an unknown slash command, and the message printed in each case.
//!
//! Moved out of `main.rs` (DEV-STRUCT-01, §1.19 step 2). The boundary came from
//! the measured dependency closure - 11 items, 151 lines, no runtime state - and
//! not from a line-count estimate; the estimate for the *neighbouring* parsing
//! subsystem was wrong by a factor of three, which is what prompted measuring.
//!
//! Everything here is pure string work, which is why it is the part of the
//! parsing work that can be tested without building a CLI action.

use std::fmt::Write as _;

use commands::slash_command_specs;

/// The program name every guidance message tells the user to run.
///
/// Every string that names a command must name the binary the user actually has.
/// This was not always so: two of these messages said `claw` - a name no build
/// produces - while their neighbours said `sego`, so the same failure path
/// suggested a command that cannot run *and* contradicted itself inside a single
/// sentence. One constant is what keeps the four of them from drifting apart
/// again; `every_guidance_message_names_the_binary_the_user_runs` asserts it.
const PROGRAM_NAME: &str = "sego";
pub(crate) const CLI_OPTION_SUGGESTIONS: &[&str] = &[
    "--help",
    "-h",
    "--version",
    "-V",
    "--model",
    "--output-format",
    "--permission-mode",
    "--dangerously-skip-permissions",
    "--allowedTools",
    "--allowed-tools",
    "--resume",
    "--print",
    "-p",
];

pub(crate) fn bare_slash_command_guidance(command_name: &str) -> Option<String> {
    if matches!(
        command_name,
        "dump-manifests"
            | "bootstrap-plan"
            | "agents"
            | "mcp"
            | "skills"
            | "system-prompt"
            | "update"
            | "login"
            | "logout"
            | "init"
            | "prompt"
    ) {
        return None;
    }
    let slash_command = slash_command_specs().iter().find(|spec| spec.name == command_name)?;
    let guidance = if slash_command.resume_supported {
        format!(
            "`{PROGRAM_NAME} {command_name}` is a slash command. Use `{PROGRAM_NAME} --resume SESSION.jsonl /{command_name}` or start `{PROGRAM_NAME}` and run `/{command_name}`."
        )
    } else {
        format!(
            "`{PROGRAM_NAME} {command_name}` is a slash command. Start `{PROGRAM_NAME}` and run `/{command_name}` inside the REPL."
        )
    };
    Some(guidance)
}

pub(crate) fn format_unknown_option(option: &str) -> String {
    let mut message = format!("unknown option: {option}");
    if let Some(suggestion) = suggest_closest_term(option, CLI_OPTION_SUGGESTIONS) {
        message.push_str("\nDid you mean ");
        message.push_str(suggestion);
        message.push('?');
    }
    let _ = write!(message, "\nRun `{PROGRAM_NAME} --help` for usage.");
    message
}

pub(crate) fn format_unknown_direct_slash_command(name: &str) -> String {
    let mut message = format!("unknown slash command outside the REPL: /{name}");
    if let Some(suggestions) = render_suggestion_line("Did you mean", &suggest_slash_commands(name))
    {
        message.push('\n');
        message.push_str(&suggestions);
    }
    let _ = write!(
        message,
        "\nRun `{PROGRAM_NAME} --help` for CLI usage, or start `{PROGRAM_NAME}` and use /help."
    );
    message
}

pub(crate) fn format_unknown_slash_command(name: &str) -> String {
    let mut message = format!("Unknown slash command: /{name}");
    if let Some(suggestions) = render_suggestion_line("Did you mean", &suggest_slash_commands(name))
    {
        message.push('\n');
        message.push_str(&suggestions);
    }
    message.push_str("\n  Help             /help lists available slash commands");
    message
}

pub(crate) fn render_suggestion_line(label: &str, suggestions: &[String]) -> Option<String> {
    (!suggestions.is_empty()).then(|| format!("  {label:<16} {}", suggestions.join(", ")))
}

pub(crate) fn suggest_slash_commands(input: &str) -> Vec<String> {
    let mut candidates = slash_command_specs()
        .iter()
        .flat_map(|spec| {
            std::iter::once(spec.name)
                .chain(spec.aliases.iter().copied())
                .map(|name| format!("/{name}"))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    let candidate_refs = candidates.iter().map(String::as_str).collect::<Vec<_>>();
    ranked_suggestions(input.trim_start_matches('/'), &candidate_refs)
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub(crate) fn suggest_closest_term<'a>(input: &str, candidates: &'a [&'a str]) -> Option<&'a str> {
    ranked_suggestions(input, candidates).into_iter().next()
}

pub(crate) fn ranked_suggestions<'a>(input: &str, candidates: &'a [&'a str]) -> Vec<&'a str> {
    let normalized_input = input.trim_start_matches('/').to_ascii_lowercase();
    let mut ranked = candidates
        .iter()
        .filter_map(|candidate| {
            let normalized_candidate = candidate.trim_start_matches('/').to_ascii_lowercase();
            let distance = levenshtein_distance(&normalized_input, &normalized_candidate);
            let prefix_bonus = usize::from(
                !(normalized_candidate.starts_with(&normalized_input)
                    || normalized_input.starts_with(&normalized_candidate)),
            );
            let score = distance + prefix_bonus;
            (score <= 4).then_some((score, *candidate))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.cmp(right).then_with(|| left.1.cmp(right.1)));
    ranked.into_iter().map(|(_, candidate)| candidate).take(3).collect()
}

pub(crate) fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        previous.clone_from(&current);
    }

    previous[right_chars.len()]
}

pub(crate) fn looks_like_slash_command_token(token: &str) -> bool {
    let trimmed = token.trim_start();
    let Some(name) = trimmed.strip_prefix('/').and_then(|value| {
        value.split_whitespace().next().map(str::trim).filter(|value| !value.is_empty())
    }) else {
        return false;
    };

    slash_command_specs().iter().any(|spec| spec.name == name || spec.aliases.contains(&name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_distance_is_zero_only_for_equal_strings() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("help", "help"), 0);
        assert_ne!(levenshtein_distance("help", "held"), 0);
    }

    #[test]
    fn levenshtein_distance_counts_each_edit_kind() {
        // A table rather than examples: the three edit kinds are the whole
        // contract of the function, and a single one passes with a broken
        // implementation of the other two.
        let cases = [
            ("", "abc", 3, "insertions from empty"),
            ("abc", "", 3, "deletions to empty"),
            ("abc", "abd", 1, "substitution"),
            ("abc", "abcd", 1, "trailing insertion"),
            ("abcd", "abc", 1, "trailing deletion"),
            ("kitten", "sitting", 3, "the textbook case"),
        ];
        for (left, right, expected, why) in cases {
            assert_eq!(levenshtein_distance(left, right), expected, "{why}: {left} -> {right}");
        }
    }

    #[test]
    fn levenshtein_distance_is_symmetric() {
        for (a, b) in [("help", "held"), ("", "abc"), ("kitten", "sitting")] {
            assert_eq!(
                levenshtein_distance(a, b),
                levenshtein_distance(b, a),
                "distance must not depend on argument order: {a} / {b}"
            );
        }
    }

    #[test]
    fn a_near_miss_option_gets_a_suggestion_and_a_far_one_does_not() {
        // "modle" is one transposition from "--model" territory; "zzzzzz" is not
        // near anything, and suggesting something there would be noise.
        // Candidates keep their dashes, so the near-miss has to as well.
        assert_eq!(suggest_closest_term("--modle", CLI_OPTION_SUGGESTIONS), Some("--model"));
        assert_eq!(suggest_closest_term("zzzzzzzzzz", CLI_OPTION_SUGGESTIONS), None);
    }

    #[test]
    fn ranked_suggestions_are_closest_first_and_bounded() {
        let candidates = ["model", "mode", "models", "modelling"];
        let ranked = ranked_suggestions("mode", &candidates);
        assert!(!ranked.is_empty(), "an exact match must still suggest");
        assert_eq!(ranked[0], "mode", "the closest candidate comes first: {ranked:?}");
        assert!(ranked.len() <= 3, "the list is bounded so it stays readable: {ranked:?}");
    }

    #[test]
    fn a_suggestion_line_is_absent_when_there_is_nothing_to_suggest() {
        assert_eq!(render_suggestion_line("Did you mean", &[]), None);
        let line = render_suggestion_line("Did you mean", &["model".to_string()]);
        assert_eq!(line.as_deref(), Some("  Did you mean     model"));
    }

    #[test]
    fn an_unknown_option_message_carries_the_option_and_the_suggestion() {
        let near = format_unknown_option("--modle");
        assert!(near.contains("unknown option: --modle"), "{near}");
        assert!(near.contains("Did you mean --model?"), "{near}");
        assert!(near.contains("sego --help"), "the message must say where usage lives:\n{near}");

        let far = format_unknown_option("zzzzzzzzzz");
        assert!(far.contains("unknown option: zzzzzzzzzz"), "{far}");
        assert!(
            !far.contains("Did you mean"),
            "a far-off option must not get a suggestion:\n{far}"
        );
    }

    #[test]
    fn slash_command_tokens_are_recognised_by_shape() {
        let cases = [
            ("/help", true, "a bare slash command"),
            ("/status detail", true, "a slash command with arguments"),
            ("help", false, "a plain word is not a slash command"),
            ("//x", false, "a double slash is a path, not a command"),
            ("/", false, "a lone slash names nothing"),
        ];
        for (token, expected, why) in cases {
            assert_eq!(looks_like_slash_command_token(token), expected, "{why}: {token}");
        }
    }

    #[test]
    fn a_mistyped_slash_command_suggests_the_real_one() {
        // The input is a slash command, which is the case this is called for.
        let suggestions = suggest_slash_commands("/statuss");
        assert!(
            suggestions.iter().any(|s| s == "/status"),
            "a near-miss must suggest the command it nearly is, in slash form: {suggestions:?}"
        );
        assert!(
            suggest_slash_commands("/zzzzzzzzzzzz").is_empty(),
            "garbage must not produce suggestions"
        );
    }

    #[test]
    fn unknown_slash_command_messages_name_the_input_and_point_at_help() {
        for message in
            [format_unknown_slash_command("nope"), format_unknown_direct_slash_command("nope")]
        {
            assert!(message.contains("nope"), "{message}");
            assert!(
                message.contains("/help") || message.contains("--help"),
                "the message must say where to find the real list:\n{message}"
            );
        }
    }

    #[test]
    fn a_bare_command_word_points_at_its_slash_form() {
        // Typing `status` without the slash is a common slip, and the guidance
        // has to name the slash form rather than just failing.
        let guidance = bare_slash_command_guidance("status").expect("status has a slash form");
        assert!(guidance.contains("/status"), "{guidance}");
        assert_eq!(bare_slash_command_guidance("zzzzzzzzzz"), None);
    }
    #[test]
    fn every_guidance_message_names_the_binary_the_user_runs() {
        // These strings tell the user what to type, so naming a program that does
        // not exist is worse than saying nothing. Two of them used to say `claw`
        // while their neighbours said `sego` - one failure path suggesting a
        // command that cannot run and contradicting itself inside a single
        // sentence. Iterating the whole spec table covers both branches of the
        // resume-supported split without naming which command is which.
        let mut checked = 0;
        for spec in slash_command_specs() {
            let Some(guidance) = bare_slash_command_guidance(spec.name) else {
                continue;
            };
            checked += 1;
            assert!(
                guidance.contains(PROGRAM_NAME),
                "the guidance for /{} must name the binary the user runs:\n{guidance}",
                spec.name
            );
            assert!(
                !guidance.contains("claw"),
                "the guidance for /{} must not name a program that does not exist:\n{guidance}",
                spec.name
            );
        }
        assert!(
            checked > 0,
            "no guidance messages were produced, so the loop above proved nothing"
        );

        for message in
            [format_unknown_option("--modle"), format_unknown_direct_slash_command("statuz")]
        {
            assert!(message.contains(PROGRAM_NAME), "{message}");
            assert!(!message.contains("claw"), "{message}");
        }
    }
}
