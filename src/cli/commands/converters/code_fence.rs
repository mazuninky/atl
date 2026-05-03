//! Shared helpers for picking a backtick-fence length when emitting Markdown
//! fenced code blocks.
//!
//! [CommonMark §4.5](https://spec.commonmark.org/0.30/#fenced-code-blocks)
//! requires the opening fence to be at least as long as the longest run of
//! consecutive backticks inside the code content; otherwise the inner run
//! prematurely closes the block. Always emit at least 3 backticks (the
//! conventional minimum), and one more than the longest backtick run in the
//! body when that run is 3 or more.
//!
//! Used by [`super::storage_to_md`] and [`super::adf_to_md`]; the input-side
//! converters (`md_to_storage`, `md_to_adf`, `md_to_wiki`) parse fences instead
//! of emitting them, so they don't need this.

/// Return the longest run of consecutive backtick (`` ` ``) characters in `s`.
///
/// Empty input returns `0`.
pub(super) fn longest_backtick_run(s: &str) -> usize {
    let mut max = 0;
    let mut cur = 0;
    for ch in s.chars() {
        if ch == '`' {
            cur += 1;
            if cur > max {
                max = cur;
            }
        } else {
            cur = 0;
        }
    }
    max
}

/// Return the backtick-fence string to use when wrapping `content` in a
/// Markdown fenced code block.
///
/// The fence is at least 3 backticks and always at least one longer than the
/// longest backtick run in `content`.
pub(super) fn pick_code_fence(content: &str) -> String {
    let max_run = longest_backtick_run(content);
    // At least 3 backticks; if content has 3+ in a row, extend by one more.
    let fence_len = max_run.max(2) + 1;
    "`".repeat(fence_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_backtick_run_finds_max_consecutive() {
        // " `` " has a run of 2, " ``` " has a run of 3, " ` " has a run of 1.
        // Maximum across the whole string is 3.
        assert_eq!(longest_backtick_run("a `` b ``` c ` d"), 3);
    }

    #[test]
    fn longest_backtick_run_handles_only_backticks() {
        assert_eq!(longest_backtick_run("`````"), 5);
    }

    #[test]
    fn longest_backtick_run_empty_input() {
        assert_eq!(longest_backtick_run(""), 0);
    }

    #[test]
    fn longest_backtick_run_no_backticks() {
        assert_eq!(longest_backtick_run("hello world"), 0);
    }

    #[test]
    fn longest_backtick_run_resets_on_non_backtick() {
        // "``a`" has runs of 2 then 1, so max is 2.
        assert_eq!(longest_backtick_run("``a`"), 2);
    }

    #[test]
    fn pick_code_fence_no_backticks_uses_three() {
        assert_eq!(pick_code_fence("hello"), "```");
    }

    #[test]
    fn pick_code_fence_double_backticks_uses_three() {
        // Two backticks fit safely inside a three-backtick fence.
        assert_eq!(pick_code_fence("a `` b"), "```");
    }

    #[test]
    fn pick_code_fence_triple_backticks_uses_four() {
        assert_eq!(pick_code_fence("a ``` b"), "````");
    }

    #[test]
    fn pick_code_fence_quadruple_backticks_uses_five() {
        assert_eq!(pick_code_fence("a ```` b"), "`````");
    }
}
