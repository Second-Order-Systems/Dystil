use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::DbError;

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppOrganization {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppIdentity {
    pub user_id: String,
    pub email: String,
    pub display_name: Option<String>,
    #[serde(skip_serializing)]
    pub org_id: String,
    #[serde(skip_serializing)]
    pub org_name: Option<String>,
    #[serde(skip_serializing)]
    pub org_slug: Option<String>,
    pub org: Option<AppOrganization>,
    pub onboarding_state: String,
}

#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub device_id: String,
    pub org_id: String,
    pub user_id: String,
    pub device_label: String,
    pub platform: String,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub capture_state: Option<String>,
    pub capture_pause_until: Option<DateTime<Utc>>,
    pub capture_state_updated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RegisteredDevice {
    pub device: DeviceRecord,
    pub raw_token: String,
}

#[derive(Debug, Clone)]
pub struct BootstrapOrganizationInput {
    pub org_id: Option<String>,
    pub org_name: String,
    pub org_slug: Option<String>,
    pub allowed_email_domains: Vec<String>,
    pub owner_user_id: String,
    pub owner_email: String,
    pub owner_display_name: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct IdentityRow {
    user_id: String,
    email: String,
    display_name: Option<String>,
    org_id: String,
    org_name: Option<String>,
    org_slug: Option<String>,
    roles: Option<Vec<String>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct DashboardIdentityRow {
    app_user_id: String,
    email: String,
    display_name: Option<String>,
    org_id: String,
    org_slug: Option<String>,
    org_name: Option<String>,
    superuser_user_id: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct DeviceRow {
    device_id: String,
    org_id: String,
    user_id: String,
    device_label: String,
    platform: String,
    revoked_at: Option<DateTime<Utc>>,
    last_seen_at: Option<DateTime<Utc>>,
    capture_state: Option<String>,
    capture_pause_until: Option<DateTime<Utc>>,
    capture_state_updated_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

pub async fn resolve_app_identity(
    pool: &PgPool,
    user: &AuthenticatedUser,
) -> Result<AppIdentity, DbError> {
    let user_id = upsert_app_user(pool, user).await?;

    if let Some(identity) = find_user_with_org(pool, &user_id).await? {
        return Ok(identity);
    }

    if let Some(domain) = email_domain(&user.email) {
        let org_matches = find_org_ids_by_domain(pool, &domain).await?;
        if org_matches.len() == 1 {
            set_user_org(pool, &user_id, &org_matches[0]).await?;
            if let Some(identity) = find_user_with_org(pool, &user_id).await? {
                return Ok(identity);
            }
        }
    }

    // Domain matching is opportunistic and never blocks signup. No match or
    // an ambiguous match receives an isolated personal organization.
    let org_id = create_personal_org(pool, user).await?;
    set_user_org(pool, &user_id, &org_id).await?;
    find_user_with_org(pool, &user_id)
        .await?
        .ok_or_else(|| DbError::Other("identity lookup failed after org creation".into()))
}

pub async fn bootstrap_organization(
    pool: &PgPool,
    input: &BootstrapOrganizationInput,
) -> Result<AppIdentity, DbError> {
    let org_id = resolve_org_id(pool, input).await?;
    let allowed_email_domains = normalize_domains(&input.allowed_email_domains);

    sqlx::query(
        "INSERT INTO organizations (id, name, slug, allowed_email_domains)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (id)
         DO UPDATE SET
             name = EXCLUDED.name,
             slug = EXCLUDED.slug,
             allowed_email_domains = EXCLUDED.allowed_email_domains",
    )
    .bind(&org_id)
    .bind(&input.org_name)
    .bind(&input.org_slug)
    .bind(&allowed_email_domains)
    .execute(pool)
    .await?;

    let owner = AuthenticatedUser {
        user_id: input.owner_user_id.clone(),
        email: input.owner_email.clone(),
        display_name: input.owner_display_name.clone(),
    };
    let user_id = upsert_app_user(pool, &owner).await?;
    set_user_org(pool, &user_id, &org_id).await?;
    set_org_superuser(pool, &org_id, &user_id).await?;
    resolve_app_identity(pool, &owner).await
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DashboardIdentity {
    pub user_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub organization: DashboardOrganization,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DashboardOrganization {
    pub id: String,
    pub slug: Option<String>,
    pub name: Option<String>,
}

pub async fn resolve_dashboard_identity(
    pool: &PgPool,
    user: &AuthenticatedUser,
) -> Result<Option<DashboardIdentity>, DbError> {
    let row = sqlx::query_as::<_, DashboardIdentityRow>(
        "SELECT
             au.id AS app_user_id,
             au.email,
             au.display_name,
             org.id AS org_id,
             org.slug AS org_slug,
             org.name AS org_name,
             org.superuser_user_id
         FROM app_users au
         JOIN organizations org ON org.id = au.org_id
         WHERE au.user_id = $1
         LIMIT 1",
    )
    .bind(&user.user_id)
    .fetch_optional(pool)
    .await?;

    let row = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    if row.superuser_user_id.as_deref() != Some(&row.app_user_id) {
        return Ok(None);
    }

    Ok(Some(DashboardIdentity {
        user_id: row.app_user_id,
        email: row.email,
        display_name: row.display_name,
        organization: DashboardOrganization {
            id: row.org_id,
            slug: row.org_slug,
            name: row.org_name,
        },
    }))
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct OrganizationInfo {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
}

pub async fn lookup_organization_by_slug(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<OrganizationInfo>, DbError> {
    let row = sqlx::query_as::<_, OrganizationInfo>(
        "SELECT id, name, slug FROM organizations WHERE slug = $1 LIMIT 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn register_device(
    pool: &PgPool,
    org_id: &str,
    user_id: &str,
    device_label: &str,
    platform: &str,
) -> Result<RegisteredDevice, DbError> {
    let device_id = uuid::Uuid::new_v4().to_string();
    let raw_token = format!("{}.{}", device_id, uuid::Uuid::new_v4().simple());
    let token_hash = hash_device_token(&raw_token);

    let row = sqlx::query_as::<_, DeviceRow>(
        "INSERT INTO devices
         (id, org_id, user_id, device_label, platform, token_hash, last_seen_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         RETURNING
             id AS device_id,
             org_id,
             user_id,
             device_label,
             platform,
             revoked_at,
             last_seen_at,
             capture_state,
             capture_pause_until,
             capture_state_updated_at,
             created_at",
    )
    .bind(&device_id)
    .bind(org_id)
    .bind(user_id)
    .bind(device_label.trim())
    .bind(platform.trim())
    .bind(token_hash)
    .fetch_one(pool)
    .await?;

    Ok(RegisteredDevice {
        device: device_from_row(row),
        raw_token,
    })
}

pub async fn resolve_active_device(
    pool: &PgPool,
    raw_token: &str,
) -> Result<Option<DeviceRecord>, DbError> {
    let token_hash = hash_device_token(raw_token);
    let row = sqlx::query_as::<_, DeviceRow>(
        "SELECT
             id AS device_id,
             org_id,
             user_id,
             device_label,
             platform,
             revoked_at,
             last_seen_at,
             capture_state,
             capture_pause_until,
             capture_state_updated_at,
             created_at
         FROM devices
         WHERE token_hash = $1
           AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    if let Some(device) = row {
        sqlx::query("UPDATE devices SET last_seen_at = now() WHERE id = $1")
            .bind(&device.device_id)
            .execute(pool)
            .await?;
        return Ok(Some(DeviceRecord {
            last_seen_at: Some(Utc::now()),
            ..device_from_row(device)
        }));
    }

    Ok(None)
}

pub async fn update_device_client_metadata(
    pool: &PgPool,
    device_id: &str,
    app_version: Option<&str>,
    build_channel: Option<&str>,
    build_commit: Option<&str>,
    capabilities: Option<&[String]>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE devices SET app_version = COALESCE($2, app_version),
             build_channel = COALESCE($3, build_channel),
             build_commit = COALESCE($4, build_commit),
             sync_capabilities = COALESCE($5, sync_capabilities),
             version_reported_at = CASE WHEN $2 IS NULL AND $3 IS NULL AND $4 IS NULL AND $5 IS NULL THEN version_reported_at ELSE now() END
         WHERE id = $1",
    )
    .bind(device_id)
    .bind(app_version)
    .bind(build_channel)
    .bind(build_commit)
    .bind(capabilities)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_device_capture_state(
    pool: &PgPool,
    device_id: &str,
    capture_state: &str,
    capture_pause_until: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>, DbError> {
    let updated_at = sqlx::query_scalar::<_, DateTime<Utc>>(
        "UPDATE devices
         SET capture_state = $2,
             capture_pause_until = $3,
             capture_state_updated_at = CASE
                 WHEN capture_state IS DISTINCT FROM $2
                   OR capture_pause_until IS DISTINCT FROM $3
                 THEN now()
                 ELSE COALESCE(capture_state_updated_at, now())
             END
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING capture_state_updated_at",
    )
    .bind(device_id)
    .bind(capture_state)
    .bind(capture_pause_until)
    .fetch_one(pool)
    .await?;
    Ok(updated_at)
}

/// Best-effort pause-history projection of a device's already-persisted current
/// capture state. The queries re-check that state so a delayed history write
/// cannot open or close the wrong pause session after a newer transition.
///
/// Callers must treat errors from this function as telemetry failures: the
/// primary `devices` update has already completed successfully.
pub async fn record_device_capture_pause_transition(
    pool: &PgPool,
    device_id: &str,
    capture_state: &str,
    capture_pause_until: Option<DateTime<Utc>>,
) -> Result<(), DbError> {
    match (capture_state, capture_pause_until) {
        ("paused", Some(scheduled_resume_at)) => {
            sqlx::query(
                "INSERT INTO device_capture_pauses (
                     org_id,
                     user_id,
                     device_id,
                     scheduled_resume_at
                 )
                 SELECT org_id, user_id, id, $2
                 FROM devices
                 WHERE id = $1
                   AND revoked_at IS NULL
                   AND capture_state = 'paused'
                   AND capture_pause_until IS NOT DISTINCT FROM $2
                 ON CONFLICT (device_id) WHERE resumed_at IS NULL
                 DO UPDATE SET scheduled_resume_at = EXCLUDED.scheduled_resume_at
                 WHERE device_capture_pauses.scheduled_resume_at
                       IS DISTINCT FROM EXCLUDED.scheduled_resume_at",
            )
            .bind(device_id)
            .bind(scheduled_resume_at)
            .execute(pool)
            .await?;
        }
        ("recording", None) => {
            sqlx::query(
                "UPDATE device_capture_pauses AS pause
                 SET resumed_at = now()
                 FROM devices
                 WHERE pause.device_id = $1
                   AND devices.id = pause.device_id
                   AND devices.revoked_at IS NULL
                   AND devices.capture_state = 'recording'
                   AND pause.resumed_at IS NULL",
            )
            .bind(device_id)
            .execute(pool)
            .await?;
        }
        _ => {
            return Err(DbError::Other(
                "invalid device capture state transition for pause history".to_string(),
            ));
        }
    }

    Ok(())
}

pub async fn list_devices_for_org(
    pool: &PgPool,
    org_id: &str,
) -> Result<Vec<DeviceRecord>, DbError> {
    let rows = sqlx::query_as::<_, DeviceRow>(
        "SELECT
             id AS device_id,
             org_id,
             user_id,
             device_label,
             platform,
             revoked_at,
             last_seen_at,
             capture_state,
             capture_pause_until,
             capture_state_updated_at,
             created_at
         FROM devices
         WHERE org_id = $1
         ORDER BY created_at DESC",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(device_from_row).collect())
}

pub async fn find_device_for_org(
    pool: &PgPool,
    org_id: &str,
    device_id: &str,
) -> Result<Option<DeviceRecord>, DbError> {
    let row = sqlx::query_as::<_, DeviceRow>(
        "SELECT
             id AS device_id,
             org_id,
             user_id,
             device_label,
             platform,
             revoked_at,
             last_seen_at,
             capture_state,
             capture_pause_until,
             capture_state_updated_at,
             created_at
         FROM devices
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(device_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(device_from_row))
}

pub async fn revoke_device(pool: &PgPool, org_id: &str, device_id: &str) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE devices
         SET revoked_at = COALESCE(revoked_at, now())
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(device_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn save_onboarding_data(
    pool: &PgPool,
    user_id: &str,
    onboarding_data: &Value,
) -> Result<(), DbError> {
    sqlx::query("UPDATE app_users SET onboarding_data = $1 WHERE id = $2")
        .bind(onboarding_data)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn upsert_app_user(pool: &PgPool, user: &AuthenticatedUser) -> Result<String, DbError> {
    let id = sqlx::query_scalar::<_, String>("SELECT id FROM app_users WHERE user_id = $1")
        .bind(&user.user_id)
        .fetch_optional(pool)
        .await?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    sqlx::query(
        "INSERT INTO app_users (id, user_id, email, display_name, last_seen_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (user_id)
         DO UPDATE SET
             email = EXCLUDED.email,
             display_name = EXCLUDED.display_name,
             last_seen_at = now()",
    )
    .bind(&id)
    .bind(&user.user_id)
    .bind(user.email.to_ascii_lowercase())
    .bind(&user.display_name)
    .execute(pool)
    .await?;

    Ok(id)
}

async fn find_user_with_org(pool: &PgPool, user_id: &str) -> Result<Option<AppIdentity>, DbError> {
    let row = sqlx::query_as::<_, IdentityRow>(
        "SELECT
             au.id AS user_id,
             au.email,
             au.display_name,
             org.id AS org_id,
             org.name AS org_name,
             org.slug AS org_slug,
             org.roles
         FROM app_users au
         JOIN organizations org ON org.id = au.org_id
         WHERE au.id = $1
         LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let IdentityRow {
            user_id,
            email,
            display_name,
            org_id,
            org_name,
            org_slug,
            roles,
        } = r;

        AppIdentity {
            user_id,
            email,
            display_name,
            org_id: org_id.clone(),
            org_name: org_name.clone(),
            org_slug: org_slug.clone(),
            org: Some(AppOrganization {
                id: org_id,
                name: org_name,
                slug: org_slug,
                roles: roles.unwrap_or_default(),
            }),
            onboarding_state: "active".to_string(),
        }
    }))
}

async fn set_user_org(pool: &PgPool, user_id: &str, org_id: &str) -> Result<(), DbError> {
    sqlx::query("UPDATE app_users SET org_id = $1 WHERE id = $2")
        .bind(org_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn set_org_superuser(pool: &PgPool, org_id: &str, app_user_id: &str) -> Result<(), DbError> {
    sqlx::query("UPDATE organizations SET superuser_user_id = $1 WHERE id = $2")
        .bind(app_user_id)
        .bind(org_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn create_personal_org(pool: &PgPool, user: &AuthenticatedUser) -> Result<String, DbError> {
    let org_id = uuid::Uuid::new_v4().to_string();
    let org_name = user
        .display_name
        .clone()
        .unwrap_or_else(|| user.email.split('@').next().unwrap_or("user").to_string());

    sqlx::query(
        "INSERT INTO organizations (id, name, slug, allowed_email_domains)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&org_id)
    .bind(&org_name)
    .bind(&org_id)
    .bind(&Vec::<String>::new())
    .execute(pool)
    .await?;

    Ok(org_id)
}

async fn resolve_org_id(
    pool: &PgPool,
    input: &BootstrapOrganizationInput,
) -> Result<String, DbError> {
    if let Some(org_id) = &input.org_id {
        return Ok(org_id.clone());
    }

    if let Some(org_slug) = &input.org_slug {
        if let Some(existing) =
            sqlx::query_scalar::<_, String>("SELECT id FROM organizations WHERE slug = $1")
                .bind(org_slug)
                .fetch_optional(pool)
                .await?
        {
            return Ok(existing);
        }
    }

    Ok(uuid::Uuid::new_v4().to_string())
}

fn email_domain(email: &str) -> Option<String> {
    email
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
}

async fn find_org_ids_by_domain(pool: &PgPool, domain: &str) -> Result<Vec<String>, DbError> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT id
         FROM organizations
         WHERE allowed_email_domains IS NOT NULL
           AND $1 = ANY(allowed_email_domains)
         ORDER BY id",
    )
    .bind(domain)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn device_from_row(row: DeviceRow) -> DeviceRecord {
    DeviceRecord {
        device_id: row.device_id,
        org_id: row.org_id,
        user_id: row.user_id,
        device_label: row.device_label,
        platform: row.platform,
        revoked_at: row.revoked_at,
        last_seen_at: row.last_seen_at,
        capture_state: row.capture_state,
        capture_pause_until: row.capture_pause_until,
        capture_state_updated_at: row.capture_state_updated_at,
        created_at: row.created_at,
    }
}

pub fn hash_device_token(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    hex::encode(hasher.finalize())
}

fn normalize_domains(domains: &[String]) -> Vec<String> {
    let mut out = domains
        .iter()
        .map(|domain| domain.trim().trim_start_matches('@').to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::{email_domain, hash_device_token, normalize_domains};

    #[test]
    fn normalize_domains_trims_lowercases_and_dedups() {
        let domains = vec![
            " Example.com ".to_string(),
            "@example.com".to_string(),
            "team.io".to_string(),
        ];
        assert_eq!(
            normalize_domains(&domains),
            vec!["example.com".to_string(), "team.io".to_string()]
        );
    }

    #[test]
    fn email_domain_normalizes_and_rejects_missing_domains() {
        assert_eq!(email_domain("User@Example.COM"), Some("example.com".into()));
        assert_eq!(
            email_domain("user@  Example.COM  "),
            Some("example.com".into())
        );
        assert_eq!(email_domain("user@"), None);
        assert_eq!(email_domain("not-an-email"), None);
    }

    #[test]
    fn device_token_hash_is_deterministic() {
        let a = hash_device_token("device.secret");
        let b = hash_device_token("device.secret");
        let c = hash_device_token("device.other");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }
}
