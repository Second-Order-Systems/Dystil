use rand::{rngs::OsRng, RngCore};
use sqlx::postgres::PgPoolOptions;
use work_insights_db::ai_gateway;

fn usage() -> ! {
    eprintln!(
        "usage:\n  ai_key issue --email EMAIL --limit-usd USD\n  ai_key revoke --key-prefix PREFIX"
    );
    std::process::exit(2);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let database_url = std::env::var("WORK_INSIGHTS_DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("WORK_INSIGHTS_DATABASE_URL is required"))?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;
    work_insights_db::migrate(&pool).await?;

    match args.first().map(String::as_str) {
        Some("issue") => issue(&pool, &args[1..]).await,
        Some("revoke") => revoke(&pool, &args[1..]).await,
        _ => usage(),
    }
}

async fn issue(pool: &sqlx::PgPool, args: &[String]) -> anyhow::Result<()> {
    let email = argument(args, "--email").unwrap_or_else(|| usage());
    if !email.contains('@') || email.len() > 320 {
        anyhow::bail!("--email must be a valid email address");
    }
    let limit = argument(args, "--limit-usd").unwrap_or_else(|| usage());
    let spend_limit_microusd = parse_usd_micros(limit)?;
    if spend_limit_microusd <= 0 {
        anyhow::bail!("--limit-usd must be greater than zero");
    }

    let mut prefix_bytes = [0_u8; 4];
    let mut secret_bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut prefix_bytes);
    OsRng.fill_bytes(&mut secret_bytes);
    let key_prefix = format!("dst_live_{}", hex::encode(prefix_bytes));
    let raw_key = format!("{}_{}", key_prefix, hex::encode(secret_bytes));
    ai_gateway::insert_ai_key(pool, email, &key_prefix, &raw_key, spend_limit_microusd).await?;

    println!("email: {email}");
    println!("limit_usd: {limit}");
    println!("api_key: {raw_key}");
    println!("This key is shown once. Store and send it securely.");
    Ok(())
}

async fn revoke(pool: &sqlx::PgPool, args: &[String]) -> anyhow::Result<()> {
    let key_prefix = argument(args, "--key-prefix").unwrap_or_else(|| usage());
    if !ai_gateway::revoke_ai_key(pool, key_prefix).await? {
        anyhow::bail!("AI key prefix was not found");
    }
    println!("revoked: {key_prefix}");
    Ok(())
}

fn argument<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn parse_usd_micros(value: &str) -> anyhow::Result<i64> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') || value.starts_with('+') {
        anyhow::bail!("USD limit must be a positive decimal amount");
    }
    let mut parts = value.split('.');
    let dollars = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("USD limit is missing"))?
        .parse::<i64>()?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || fraction.len() > 6
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        anyhow::bail!("USD limit may contain at most six decimal places");
    }
    let mut micros = fraction.to_string();
    micros.extend(std::iter::repeat_n('0', 6 - micros.len()));
    let fraction_micros = if micros.is_empty() {
        0
    } else {
        micros.parse::<i64>()?
    };
    dollars
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(fraction_micros))
        .ok_or_else(|| anyhow::anyhow!("USD limit is too large"))
}

#[cfg(test)]
mod tests {
    use super::parse_usd_micros;

    #[test]
    fn parses_usd_without_floating_point() {
        assert_eq!(parse_usd_micros("10").unwrap(), 10_000_000);
        assert_eq!(parse_usd_micros("10.25").unwrap(), 10_250_000);
        assert_eq!(parse_usd_micros("0.000001").unwrap(), 1);
        assert!(parse_usd_micros("1.0000001").is_err());
    }
}
