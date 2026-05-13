use crate::redact::patterns::PATTERNS;
use regex::Regex;

pub struct RuleSet {
    compiled: Vec<(&'static str, Regex)>,
}

impl Default for RuleSet {
    fn default() -> Self {
        let compiled = PATTERNS.iter()
            .map(|p| (p.name, Regex::new(p.regex).expect("invalid built-in pattern")))
            .collect();
        Self { compiled }
    }
}

impl RuleSet {
    pub fn apply(&self, input: &str) -> String {
        let mut out = input.to_string();
        for (name, re) in &self.compiled {
            out = re.replace_all(&out, format!("«redacted:{}»", name).as_str()).into_owned();
        }
        out
    }
}

impl RuleSet {
    pub fn with_extra(extra: &[(&str, &str)]) -> Result<Self, regex::Error> {
        let mut compiled: Vec<(&'static str, Regex)> = PATTERNS.iter()
            .map(|p| (p.name, Regex::new(p.regex).expect("invalid built-in pattern")))
            .collect();
        for (name, re) in extra {
            let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
            compiled.push((leaked, Regex::new(re)?));
        }
        Ok(Self { compiled })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extra_rules_redact_custom_pattern() {
        let r = RuleSet::with_extra(&[("project_token", r"PROJTOK-[A-Z0-9]{8}")]).unwrap();
        let out = r.apply("see PROJTOK-ABCDEFGH here");
        assert!(out.contains("«redacted:project_token»"));
    }
}
