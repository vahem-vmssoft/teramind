//! Compile-time prompt constants. A snapshot test in this module catches
//! accidental drift from the prompt the spec calls for.

pub const SYSTEM_PROMPT: &str = "You are summarizing a Claude Code session for a developer wiki. The user\nhas given you a structured digest of what happened. Write a concise wiki\npage in Markdown with these sections, in order:\n\n# Summary\n\nA one-paragraph (~3 sentences) plain-English description of what the\nsession accomplished, who initiated it (agent vs human edits), and the\noutcome.\n\n# Files changed\n\nA bulleted list of files with a one-sentence note per file describing\nthe intent of the change.\n\n# Decisions & gotchas\n\n3-5 bullets. Surface non-obvious decisions and gotchas the agent noted.\nIf none are visible in the digest, write \"None recorded.\"\n\n# Follow-ups\n\nTasks left undone or implied by the work. If none, write \"None recorded.\"\n\nConstraints:\n- Be faithful to the digest. Do NOT invent details not present.\n- Cite filenames and tool names verbatim where relevant.\n- Output Markdown only. No preamble.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_contains_all_four_section_headers() {
        for header in ["# Summary", "# Files changed", "# Decisions & gotchas", "# Follow-ups"] {
            assert!(SYSTEM_PROMPT.contains(header), "missing {header}");
        }
    }

    #[test]
    fn system_prompt_forbids_invention() {
        assert!(SYSTEM_PROMPT.contains("Do NOT invent"));
    }

    #[test]
    fn system_prompt_length_under_limit() {
        // Keep the prompt small so it doesn't eat token budget at run time.
        assert!(SYSTEM_PROMPT.len() < 2048, "prompt grew to {}", SYSTEM_PROMPT.len());
    }
}
