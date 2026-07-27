use anyhow::{bail, Context};
use sqlx::postgres::PgPoolOptions;
use work_insights_db::identity::{bootstrap_organization, BootstrapOrganizationInput};

fn usage() -> ! {
    eprintln!(
        "usage: bootstrap-org --org-name NAME --owner-supabase-user-id UUID --owner-email EMAIL [--org-id ID] [--org-slug SLUG] [--owner-display-name NAME] [--domain DOMAIN ...]"
    );
    std::process::exit(2);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args(std::env::args().skip(1).collect())?;
    let database_url = std::env::var("WORK_INSIGHTS_DATABASE_URL")
        .context("WORK_INSIGHTS_DATABASE_URL is required")?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("connecting to Postgres failed")?;
    work_insights_db::migrate(&pool)
        .await
        .context("running migrations failed")?;

    let identity = bootstrap_organization(&pool, &args).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&identity).context("serializing bootstrap result failed")?
    );
    Ok(())
}

fn parse_args(args: Vec<String>) -> anyhow::Result<BootstrapOrganizationInput> {
    let mut org_id = None;
    let mut org_name = None;
    let mut org_slug = None;
    let mut owner_user_id = None;
    let mut owner_email = None;
    let mut owner_display_name = None;
    let mut allowed_email_domains = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let flag = &args[i];
        let next = |i: &mut usize| -> anyhow::Result<String> {
            *i += 1;
            args.get(*i).cloned().context("missing value for flag")
        };
        match flag.as_str() {
            "--org-id" => org_id = Some(next(&mut i)?),
            "--org-name" => org_name = Some(next(&mut i)?),
            "--org-slug" => org_slug = Some(next(&mut i)?),
            "--owner-supabase-user-id" => owner_user_id = Some(next(&mut i)?),
            "--owner-email" => owner_email = Some(next(&mut i)?),
            "--owner-display-name" => owner_display_name = Some(next(&mut i)?),
            "--domain" => allowed_email_domains.push(next(&mut i)?),
            "--help" | "-h" => usage(),
            other => bail!("unknown flag: {other}"),
        }
        i += 1;
    }

    let input = BootstrapOrganizationInput {
        org_id,
        org_name: org_name.unwrap_or_else(|| usage()),
        org_slug,
        allowed_email_domains,
        owner_user_id: owner_user_id.unwrap_or_else(|| usage()),
        owner_email: owner_email.unwrap_or_else(|| usage()),
        owner_display_name,
    };
    Ok(input)
}
