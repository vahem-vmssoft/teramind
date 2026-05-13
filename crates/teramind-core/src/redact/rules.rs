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
