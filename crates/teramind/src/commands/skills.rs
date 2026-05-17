//! `teramind skills list / show / observations`.

use anyhow::Result;
use teramind_ipc::proto::{Request, Response};

pub async fn list(filter: String, limit: u32) -> Result<()> {
    let resp = crate::ipc::request(
        Request::SkillsList {
            filter: Some(filter),
            limit,
        },
        10_000,
    )
    .await?;
    match resp {
        Response::SkillsList { rows } => {
            if rows.is_empty() {
                println!("(no skills)");
                return Ok(());
            }
            println!(
                "{:<36}  {:<20}  {:<30}  description",
                "id", "source", "name"
            );
            for r in rows {
                let src = match r.status {
                    Some(s) => format!("candidate({s})"),
                    None => r.source,
                };
                println!(
                    "{:<36}  {:<20}  {:<30}  {}",
                    r.id, src, r.name, r.description
                );
            }
        }
        Response::Error(e) => return Err(anyhow::anyhow!(e)),
        other => return Err(anyhow::anyhow!("unexpected response: {other:?}")),
    }
    Ok(())
}

pub async fn show(name_or_id: String) -> Result<()> {
    let resp =
        crate::ipc::request(Request::SkillsShow { name_or_id }, 10_000).await?;
    match resp {
        Response::SkillShow {
            name,
            description,
            body,
            source,
            applies_to_cwds,
        } => {
            println!("# {name}");
            println!("source: {source}");
            println!("applies_to_cwds: {applies_to_cwds:?}");
            println!("description: {description}");
            println!();
            println!("{body}");
        }
        Response::Error(e) => return Err(anyhow::anyhow!(e)),
        other => return Err(anyhow::anyhow!("unexpected response: {other:?}")),
    }
    Ok(())
}

pub async fn observations(
    kind: Option<String>,
    min_freq: i32,
    status: Option<String>,
    limit: u32,
) -> Result<()> {
    let resp = crate::ipc::request(
        Request::SkillsObservations {
            kind,
            min_freq,
            status,
            limit,
        },
        10_000,
    )
    .await?;
    match resp {
        Response::SkillsObservations { rows } => {
            if rows.is_empty() {
                println!("(no observations)");
                return Ok(());
            }
            println!(
                "{:<36}  {:<14}  {:>5}  {:<14}  signature",
                "id", "kind", "freq", "status"
            );
            for r in rows {
                println!(
                    "{:<36}  {:<14}  {:>5}  {:<14}  {}",
                    r.id, r.kind, r.frequency, r.status, r.signature
                );
            }
        }
        Response::Error(e) => return Err(anyhow::anyhow!(e)),
        other => return Err(anyhow::anyhow!("unexpected response: {other:?}")),
    }
    Ok(())
}
