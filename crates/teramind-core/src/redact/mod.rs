pub mod patterns;
pub mod rules;

use rules::RuleSet;

pub struct Redactor {
    rules: RuleSet,
}

impl Redactor {
    pub fn with_default_rules() -> Self {
        Self { rules: RuleSet::default() }
    }
    pub fn with_extra(extra: &[(&str, &str)]) -> Result<Self, regex::Error> {
        Ok(Self { rules: RuleSet::with_extra(extra)? })
    }
    pub fn apply(&self, input: &str) -> String { self.rules.apply(input) }
}
