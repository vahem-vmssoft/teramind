//! `teramind redact test [<input>]` — preview redactions for sanity-check.
//!
//! If `input` is None, read from stdin. The result is printed to stdout.

use anyhow::Result;
use std::io::Read;
use teramind_core::redact::Redactor;

pub async fn test(input: Option<String>) -> Result<()> {
    let raw = match input {
        Some(s) => s,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };
    let redactor = Redactor::with_default_rules();
    let out = redactor.apply(&raw);
    println!("{out}");
    Ok(())
}
